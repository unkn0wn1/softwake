//! Voice machine, mock capture, the PCM wake stand-in, and the loaded soul pack.
//!
//! UI commands map onto [`softwake_state::Event`]. Capture starts in sleep,
//! stops for hibernate, and starts again when hibernate returns to sleep.
//! [`Command::ReloadSoul`] re-reads the soul directory. That read applies on
//! the next transition into awake, not in the middle of an awake session.
//! A wake phrase with a missing or invalid pack leaves the machine where it is.
//! A successful awake opens a text session with the rendered instructions.
//! Sleep and hibernate close that session and drop a pending confirmation.
//! Safe tools run only while awake. A confirm-gated tool waits for
//! `confirm_tool`. `cancel_tool` clears it without running.
//! Mock STT/TTS act only while awake ([ADR 0007](../../docs/ADR-0007-awake-stt-tts.md)).
//! `ask` completes one provider turn on the open session while awake
//! ([ADR 0013](../../docs/ADR-0013-session-provider.md)).

use std::time::Instant;

use softwake_audio::rms_level;
use softwake_ipc::VoiceState as WireState;
use softwake_ipc::{Command, Event as WireEvent, IpcError, PendingTool, ResponseBody, Status};
use softwake_session::{SessionPhase, TextStubSession};
use softwake_soul::SoulDir;
use softwake_state::{CooldownConfig, Effect, Event, Machine, StateError, VoiceState};
use softwake_voice::{EnergyUtterance, MockStt, MockTts, TextToSpeech, TranscriptEvent};
use softwake_wake::PhraseHit;

use crate::capture::{CaptureBackend, CaptureKind};
use crate::dispatch::{
    CancelledTool, ConfirmedTool, DispatchError, Hands, PendingToolCall, RanTool, RequestOutcome,
};
use crate::pcm::{PcmEngine, score_frame_detailed};
use crate::soul::LoadedSoul;

#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) body: ResponseBody,
    pub(crate) events: Vec<WireEvent>,
}

pub(crate) struct Runtime {
    machine: Machine,
    /// Wall clock for [`Self::tick`]. Phrase cooldowns use machine time only.
    last_tick: Instant,
    capture: CaptureBackend,
    pcm: PcmEngine,
    /// Agent name the current [`Self::pcm`] was built for. Avoids reloading ONNX on no-op soul reloads.
    pcm_agent: String,
    /// Last PCM score from [`Self::drain_pcm`]. `None` means the queue was empty.
    #[cfg_attr(not(test), allow(dead_code))]
    last_pcm_hit: Option<PhraseHit>,
    /// Latest [`rms_level`] from a drained frame. Cleared when capture stops.
    last_capture_level: Option<f32>,
    /// Rate-limit for mic-energy stderr lines (`PipeWire` path).
    last_energy_log: Option<Instant>,
    /// Rate-limit for verbose KWS hear lines (`-v` / `-vv`).
    last_kws_log: Option<Instant>,
    /// `0` quiet, `1` (`-v`) keyword hear/match, `2` (`-vv`) also mic energy.
    verbosity: u8,
    soul: LoadedSoul,
    session: TextStubSession,
    hands: Hands,
    stt: MockStt,
    tts: MockTts,
    talk: crate::talk::TalkSession,
    /// Energy-gated free speech while awake (inactive during PTT).
    auto_utt: EnergyUtterance,
    /// Finished free-speech PCM waiting for STT→ask (taken by serve after status).
    pending_auto_pcm: Option<Vec<i16>>,
    /// Wall time when the last ask/TTS finished; free speech stays quiet during cooldown.
    last_voice_activity: Option<Instant>,
    /// Last operator-facing voice/ask line retained for HUD polls.
    last_status_message: Option<String>,
    /// Last detail line retained for HUD polls.
    last_status_detail: Option<String>,
    /// Last ask context estimate (Status while awake).
    last_context_used: Option<u32>,
    /// Last resolved context limit.
    last_context_limit: Option<u32>,
    /// Whether the last ask compacted.
    last_context_compacted: bool,
    /// KWS / voice test mode. Default off. Phrases still transition; mic speech is not asked.
    voice_test: bool,
    /// State-announcement prompts recorded in tests. Production speaks them off-thread.
    #[cfg(test)]
    announced_prompts: Vec<String>,
    /// Last playback error after a successful ask. Empty when speech played or was skipped.
    last_speech_note: Option<String>,
    /// In-test provider. Absent in the serve binary, so ask uses the disk path.
    #[cfg(test)]
    chat_fixture: Option<crate::chat::ChatFixture>,
    /// When set, `talk_stop` skips cloud STT and asks with this text.
    #[cfg(test)]
    talk_transcript: Option<String>,
}

impl Runtime {
    /// Sleep, with mock capture already running and the soul directory read.
    #[cfg(test)]
    pub(crate) fn new(soul_dir: SoulDir) -> Self {
        Self::with_capture(soul_dir, CaptureKind::Mock).expect("mock capture cannot fail to open")
    }

    /// Sleep with the selected capture backend already running.
    ///
    /// # Errors
    ///
    /// Returns the backend error when `PipeWire` cannot be opened.
    #[cfg(test)]
    pub(crate) fn with_capture(
        soul_dir: SoulDir,
        kind: CaptureKind,
    ) -> Result<Self, crate::capture::CaptureError> {
        Self::with_capture_verbosity(soul_dir, kind, 0)
    }

    /// Sleep with capture and stderr verbosity (`0` / `-v` / `-vv`).
    ///
    /// # Errors
    ///
    /// Returns the backend error when `PipeWire` cannot be opened.
    pub(crate) fn with_capture_verbosity(
        soul_dir: SoulDir,
        kind: CaptureKind,
        verbosity: u8,
    ) -> Result<Self, crate::capture::CaptureError> {
        let capture = CaptureBackend::open(kind)?;
        let soul = LoadedSoul::open(soul_dir);
        let agent = soul.agent_name();
        let profile = soul.profile_log_token();
        let pcm = PcmEngine::for_agent(&agent);
        pcm.log_startup(&profile, verbosity);
        eprintln!(
            "softwaked: listen state={} capture={}",
            wire_state(VoiceState::Sleep).as_str(),
            kind.as_str()
        );
        Ok(Self {
            machine: Machine::new(CooldownConfig::default()),
            last_tick: Instant::now(),
            capture,
            pcm,
            pcm_agent: agent,
            last_pcm_hit: None,
            last_capture_level: None,
            last_energy_log: None,
            last_kws_log: None,
            verbosity,
            soul,
            session: TextStubSession::default(),
            hands: Hands::from_disk(),
            stt: MockStt::default(),
            tts: MockTts::default(),
            talk: crate::talk::TalkSession::default(),
            auto_utt: EnergyUtterance::new(),
            pending_auto_pcm: None,
            last_voice_activity: None,
            last_status_message: None,
            last_status_detail: None,
            last_context_used: None,
            last_context_limit: None,
            last_context_compacted: false,
            voice_test: false,
            #[cfg(test)]
            announced_prompts: Vec::new(),
            last_speech_note: None,
            #[cfg(test)]
            chat_fixture: None,
            #[cfg(test)]
            talk_transcript: None,
        })
    }

    /// Apply one client command.
    ///
    /// A voice-state change yields a [`WireEvent::StateChanged`] for every
    /// connected client. A rejection leaves the machine untouched and does
    /// not emit that event.
    pub(crate) fn handle(&mut self, command: Command) -> Outcome {
        // Score PCM that arrived before this command. Serve's queue is empty
        // unless a test pushed samples. The typed demo enters awake on its own.
        // While capture runs, queue a short mock listening tone before drain so
        // status replies (and HUD polls) see frame-derived levels without a mic.
        if self.capture.is_running() {
            let phase = self.last_tick.elapsed().as_secs_f32();
            self.capture.push_listening_tone_if_mock(phase);
        }
        let mut pcm_events = self.drain_pcm();
        match command {
            Command::GetStatus => {
                let mut outcome = Self::quiet(self.snapshot(
                    self.last_status_message.clone(),
                    self.last_status_detail.clone(),
                ));
                outcome.events.append(&mut pcm_events);
                outcome
            }
            Command::ReloadSoul => {
                self.soul.reload();
                self.rebuild_pcm();
                let mut outcome =
                    Self::quiet(self.snapshot(Some(self.soul.reload_summary()), None));
                outcome.events.append(&mut pcm_events);
                outcome
            }
            Command::Hibernate | Command::WakeFromUi | Command::Sleep => {
                let mut outcome = self.transition(command);
                outcome.events.append(&mut pcm_events);
                outcome
            }
        }
    }

    /// Enter awake from a wake phrase when the loaded pack is valid.
    ///
    /// An invalid pack does not change the voice state. The pack applied here
    /// is the last startup or `reload_soul` read, not a fresh disk read.
    ///
    /// Serve does not open a microphone. This method does not call
    /// [`Self::drain_pcm`]. The typed demo applies the same [`LoadedSoul`]
    /// gate on its own machine.
    pub(crate) fn wake_phrase(&mut self) -> Outcome {
        self.tick();
        if let Some(reason) = self.soul.refusal() {
            return Self::rejected(IpcError::protocol(reason));
        }
        self.apply_voice(Event::WakePhrase, |error| {
            IpcError::protocol(error.to_string())
        })
    }

    /// Move the phrase clock forward by the wall time since the previous tick.
    ///
    /// Called at the start of [`Self::wake_phrase`] and [`Self::transition`].
    /// A rejection still advances the clock and does not re-arm the gate.
    fn tick(&mut self) {
        let now = Instant::now();
        self.machine
            .advance(now.saturating_duration_since(self.last_tick));
        self.last_tick = now;
    }

    /// Instructions stored the last time awake was entered successfully.
    #[cfg(test)]
    pub(crate) fn applied_instructions(&self) -> Option<&str> {
        self.soul.applied_instructions()
    }

    /// Phase of the text session. Closed unless awake opened it.
    #[cfg(test)]
    pub(crate) fn session_phase(&self) -> SessionPhase {
        self.session.phase()
    }

    /// Instructions held by the open session.
    #[cfg(test)]
    pub(crate) fn session_instructions(&self) -> Option<&str> {
        self.session.instructions()
    }

    /// User lines recorded on the open session.
    #[cfg(test)]
    pub(crate) fn session_turns(&self) -> Vec<String> {
        self.session
            .user_texts()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Install the in-test provider. Production serve leaves this unset.
    #[cfg(test)]
    pub(crate) fn install_chat_fixture(&mut self, fixture: crate::chat::ChatFixture) {
        self.chat_fixture = Some(fixture);
    }

    /// Posts recorded by the in-test transport. Empty when no fixture is installed.
    #[cfg(test)]
    pub(crate) fn chat_posts(&self) -> Vec<crate::chat::RecordedPost> {
        self.chat_fixture
            .as_ref()
            .map(|fixture| fixture.transport.posts())
            .unwrap_or_default()
    }

    /// Complete one ask while awake.
    ///
    /// Blank text is rejected before the voice check. Sleep and hibernate
    /// refuse the turn before Settings are loaded. The assistant text is
    /// [`Status::message`]. No event is emitted. The bearer is not copied
    /// onto the status.
    pub(crate) fn ask(&mut self, text: &str) -> Outcome {
        if text.trim().is_empty() {
            return Self::chat_rejected("ask needs text");
        }
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::chat_rejected(&format!(
                "ask while {} (chat acts only while awake)",
                wire_state(self.machine.state())
            ));
        }
        if self.session.phase() != SessionPhase::Open {
            return Self::chat_rejected("session is closed");
        }
        if let Some(outcome) = self.try_shell_ask(text) {
            return outcome;
        }
        #[cfg(test)]
        if self.chat_fixture.is_some() {
            return self.ask_fixture(text);
        }
        self.ask_disk(text)
    }

    /// When Tools→shell is enabled, turn clear shell/ssh ask lines into a gated tool call.
    fn try_shell_ask(&mut self, text: &str) -> Option<Outcome> {
        self.sync_shell_glossary();
        let Ok(settings) = softwake_tools::resolve_tools_file()
            .and_then(softwake_tools::FileToolsSettings::new)
            .and_then(|store| store.load())
        else {
            return None;
        };
        if !settings.shell_enabled {
            return None;
        }
        let command = crate::shell_intent::propose_shell_command(text, self.hands.glossary())?;
        let step = self
            .hands
            .request(&self.machine, softwake_tools::SHELL_TOOL, &[command]);
        Some(self.outcome_for_request(step))
    }

    fn sync_shell_glossary(&mut self) {
        if let Some(pack) = self.soul.pack() {
            self.hands.set_glossary(pack.aliases().clone());
        }
    }

    fn chat_rejected(message: &str) -> Outcome {
        Self::rejected(IpcError::ChatRejected {
            message: message.to_owned(),
        })
    }

    #[cfg(test)]
    fn ask_fixture(&mut self, text: &str) -> Outcome {
        let extracted = self.chat_fixture.as_ref().map(|fixture| {
            let prepared = crate::chat::prepare_fixture(fixture);
            (prepared, std::sync::Arc::clone(&fixture.transport))
        });
        let Some((prepared, transport)) = extracted else {
            return self.ask_disk(text);
        };
        match prepared {
            Ok((prepared, bearer)) => {
                let call = prepared.clone();
                // Disabled memory keeps the appendix empty. A developer
                // `memory.json` must not change the fixture post.
                let appendix = crate::chat::appendix_for_ask(
                    text,
                    Some(&softwake_memory::MockMemory::default()),
                );
                let budget = crate::chat::budget_for_handle(
                    self.chat_fixture
                        .as_ref()
                        .map(|f| &f.handle)
                        .expect("fixture"),
                );
                let transport_compact = std::sync::Arc::clone(&transport);
                let call_c = call.clone();
                let bearer_c = bearer.clone();
                let trace = self.trace_context();
                match crate::chat::perform_ask(
                    &mut self.session,
                    text,
                    &appendix,
                    budget,
                    move |older| {
                        crate::chat::compact_fixture(&transport_compact, &call_c, &bearer_c, older)
                    },
                    move |system, messages| {
                        crate::chat::complete_fixture(&transport, &call, &bearer, system, messages)
                    },
                    trace,
                ) {
                    Ok(ok) => self.finish_ask_reply(ok),
                    Err(error) => Self::chat_rejected(error.sentence()),
                }
            }
            Err(message) => Self::chat_rejected(&message),
        }
    }

    fn ask_disk(&mut self, text: &str) -> Outcome {
        let ready = match crate::chat::load_disk_chat() {
            Ok(ready) => ready,
            Err(message) => return Self::chat_rejected(&message),
        };
        let crate::chat::DiskChat {
            prepared,
            bearer,
            tts_voice: _,
            stt_model: _,
            budget,
        } = ready;
        if let Err(message) = crate::chat::gate_live_http(&prepared, &bearer, text) {
            return Self::chat_rejected(&message);
        }
        let call = prepared.clone();
        let call_c = prepared.clone();
        let bearer_c = bearer.clone();
        let appendix =
            crate::chat::appendix_for_ask(text, Option::<&softwake_memory::MockMemory>::None);
        let trace = self.trace_context();
        match crate::chat::perform_ask(
            &mut self.session,
            text,
            &appendix,
            budget,
            move |older| crate::chat::finish_prepared_compact(&call_c, &bearer_c, older),
            move |system, messages| {
                crate::chat::finish_prepared_chat(&call, &bearer, system, messages)
            },
            trace,
        ) {
            Ok(ok) => self.finish_ask_reply(ok),
            Err(error) => Self::chat_rejected(error.sentence()),
        }
    }

    /// Assistant text on [`Status::message`]. Speaks when xAI TTS is configured.
    ///
    /// A playback failure keeps the reply and puts the player sentence on
    /// [`Status::detail`].
    fn finish_ask_reply(&mut self, ok: crate::chat::AskOk) -> Outcome {
        let crate::chat::AskOk { reply, context } = ok;
        self.last_context_used = Some(context.used);
        self.last_context_limit = Some(context.limit);
        self.last_context_compacted = context.compacted;
        self.speak_if_configured(&reply);
        let mut detail = self.last_speech_note.clone();
        if context.compacted {
            let compact_note = format!(
                "compacted; context ~{} / {} ({}%)",
                context.used, context.limit, context.percent
            );
            detail = Some(match detail {
                Some(existing) => format!("{existing}; {compact_note}"),
                None => compact_note,
            });
        }
        self.last_voice_activity = Some(Instant::now());
        self.auto_utt.reset();
        self.retain_status_text(Some(reply.clone()), detail.clone());
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(reply), detail)),
            events: Vec::new(),
        }
    }

    fn trace_context(&self) -> impl FnMut(&crate::chat::AskContext) + use<> {
        let verbosity = self.verbosity;
        let profile = self.soul.profile_log_token();
        move |context| {
            if verbosity == 0 {
                return;
            }
            eprintln!(
                "{}",
                crate::chat::format_context_sent_line(&profile, context)
            );
            if context.compacted {
                eprintln!("{}", crate::chat::format_compact_line(&profile, context));
            }
        }
    }

    fn speak_if_configured(&mut self, reply: &str) {
        self.last_speech_note = None;
        // Unit tests must not open a speaker or call TTS. Serve speaks for real.
        #[cfg(test)]
        {
            let _ = reply;
        }
        #[cfg(not(test))]
        {
            let ready = match crate::chat::load_disk_chat() {
                Ok(ready) => ready,
                Err(message) => {
                    self.last_speech_note = Some(format!("voice skipped: {message}"));
                    return;
                }
            };
            if let Err(message) = crate::talk::speak_reply(&ready, reply) {
                self.last_speech_note = Some(message);
            }
        }
    }

    /// Arm press-to-talk. Hibernate refuses. Sleep wakes first when the pack is valid.
    pub(crate) fn talk_start(&mut self) -> Outcome {
        if self.voice_test {
            return self.voice_test_speech_outcome();
        }
        if wire_state(self.machine.state()) == WireState::Hibernate {
            return Self::talk_rejected(crate::talk::TALK_HIBERNATING);
        }
        // PTT takes priority over free-speech gating.
        self.auto_utt.reset();
        self.pending_auto_pcm = None;
        if wire_state(self.machine.state()) == WireState::Sleep {
            let woke = self.wake_phrase();
            if woke.body.status().is_none() {
                return woke;
            }
            // Wake already broadcast by the caller when this returns events.
            self.talk.arm();
            self.drain_pcm();
            self.retain_status_text(
                Some("listening".to_owned()),
                Some("press and hold to talk".to_owned()),
            );
            return Outcome {
                body: ResponseBody::ok(self.snapshot(
                    Some("listening".to_owned()),
                    Some("press and hold to talk".to_owned()),
                )),
                events: woke.events,
            };
        }
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::talk_rejected(&crate::talk::talk_while(
                wire_state(self.machine.state()).as_str(),
            ));
        }
        self.talk.arm();
        self.drain_pcm();
        self.retain_status_text(
            Some("listening".to_owned()),
            Some("press and hold to talk".to_owned()),
        );
        Self::quiet(self.snapshot(
            Some("listening".to_owned()),
            Some("press and hold to talk".to_owned()),
        ))
    }

    /// Release press-to-talk, transcribe, and ask.
    ///
    /// Tests pass `samples` they already buffered through [`Self::push_talk_samples`].
    pub(crate) fn talk_stop(&mut self) -> Outcome {
        if self.voice_test {
            return self.voice_test_speech_outcome();
        }
        if !self.talk.is_armed() && self.talk_buffer_empty() {
            return Self::talk_rejected("talk is not armed");
        }
        if self.machine.permit_tool_dispatch().is_err() {
            self.talk.clear();
            return Self::talk_rejected(&crate::talk::talk_while(
                wire_state(self.machine.state()).as_str(),
            ));
        }
        self.drain_pcm();
        let samples = match self.talk.finish() {
            Ok(samples) => samples,
            Err(message) => return Self::talk_rejected(&message),
        };
        self.retain_status_text(
            Some("thinking…".to_owned()),
            Some("press to talk".to_owned()),
        );
        self.transcribe_and_ask(&samples)
    }

    fn talk_buffer_empty(&self) -> bool {
        !self.talk.is_armed()
    }

    #[cfg(test)]
    fn take_talk_transcript_override(&mut self) -> Option<String> {
        self.talk_transcript.take()
    }

    /// Next `talk_stop` uses `text` instead of cloud STT. Tests only.
    #[cfg(test)]
    pub(crate) fn set_talk_transcript_for_test(&mut self, text: impl Into<String>) {
        self.talk_transcript = Some(text.into());
    }

    pub(crate) fn transcribe_and_ask_pub(&mut self, samples: &[i16]) -> Outcome {
        self.transcribe_and_ask(samples)
    }

    fn transcribe_and_ask(&mut self, samples: &[i16]) -> Outcome {
        if self.voice_test {
            return self.voice_test_speech_outcome();
        }
        #[cfg(test)]
        if let Some(text) = self.take_talk_transcript_override() {
            let _ = samples;
            let events = self.emit_final_transcript(&text);
            let mut outcome = self.ask(&text);
            outcome.events.splice(0..0, events);
            return outcome;
        }
        let ready = match crate::chat::load_disk_chat() {
            Ok(ready) => ready,
            Err(message) => return Self::talk_rejected(&message),
        };
        let model = match crate::talk::stt_model_for(ready.prepared.provider, &ready.stt_model) {
            Ok(model) => model,
            Err(message) => return Self::talk_rejected(&message),
        };
        match crate::talk::transcribe_pcm(&ready, &model, samples) {
            Ok(text) => {
                let events = self.emit_final_transcript(&text);
                let mut outcome = self.ask(&text);
                outcome.events.splice(0..0, events);
                outcome
            }
            Err(message) => Self::talk_rejected(&message),
        }
    }

    fn emit_final_transcript(&mut self, text: &str) -> Vec<WireEvent> {
        self.stt.inject_final(text);
        let events = self.stt.drain();
        events
            .into_iter()
            .map(|event| match event {
                TranscriptEvent::Final { text } => WireEvent::FinalTranscript { text },
                TranscriptEvent::Partial { text } => WireEvent::PartialTranscript { text },
            })
            .collect()
    }

    fn talk_rejected(message: &str) -> Outcome {
        Self::rejected(IpcError::TalkRejected {
            message: message.to_owned(),
        })
    }

    /// Queue PCM into the armed talk buffer. No-op when talk is not armed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn push_talk_samples(&mut self, samples: &[i16]) {
        self.talk.push(samples);
    }

    /// Run one safe tool, or stage a confirm-gated tool.
    ///
    /// Sleep and hibernate refuse the call before the registry. A safe run
    /// emits [`WireEvent::ToolStarted`] and then [`WireEvent::ToolFinished`].
    /// A confirm-gated tool does not run: the outcome is
    /// [`WireEvent::ToolConfirmPending`] and [`Status::pending_tool`]. A
    /// refusal emits neither.
    pub(crate) fn invoke_tool(&mut self, name: &str, args: &[String]) -> Outcome {
        let step = self.hands.request(&self.machine, name, args);
        self.outcome_for_request(step)
    }

    /// Run the pending tool once when the id matches and the daemon is awake.
    ///
    /// `name`, when set, must match the pending tool. A second confirm of the
    /// same id fails because the first confirm clears the record.
    pub(crate) fn confirm_tool(&mut self, pending_id: &str, name: Option<&str>) -> Outcome {
        match self.hands.confirm(&self.machine, pending_id, name) {
            Ok(confirmed) => self.confirmed_outcome(confirmed),
            Err(error) => Self::rejected(tool_ipc_error(&error)),
        }
    }

    /// Clear a matching pending confirmation without running the tool.
    ///
    /// This is allowed when the daemon is not awake, so an operator can drop
    /// a confirmation that survived a state change. Sleep and hibernate also
    /// clear it on their own.
    pub(crate) fn cancel_tool(&mut self, pending_id: &str, name: Option<&str>) -> Outcome {
        match self.hands.cancel(pending_id, name) {
            Ok(cancelled) => self.cancelled_outcome(cancelled),
            Err(error) => Self::rejected(tool_ipc_error(&error)),
        }
    }

    /// Inject mock STT text while awake and emit transcript IPC events.
    ///
    /// Sleep and hibernate refuse the channel. Used by tests and as the serve
    /// stand-in until a microphone ASR path is wired.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inject_transcript(&mut self, text: &str) -> Outcome {
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::rejected(IpcError::protocol(format!(
                "cannot hear while {}",
                wire_state(self.machine.state())
            )));
        }
        self.stt.inject_partial(text);
        self.stt.inject_final(text);
        let events = self.stt.drain();
        let mut wire = Vec::with_capacity(events.len());
        for event in events {
            match event {
                TranscriptEvent::Partial { text } => {
                    wire.push(WireEvent::PartialTranscript { text });
                }
                TranscriptEvent::Final { text } => {
                    wire.push(WireEvent::FinalTranscript { text });
                }
            }
        }
        let detail = Some(format!("transcript events: {}", wire.len()));
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(format!("heard: {text}")), detail)),
            events: wire,
        }
    }

    /// Record mock TTS while awake.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn speak(&mut self, text: &str) -> Outcome {
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::rejected(IpcError::protocol(format!(
                "cannot say while {}",
                wire_state(self.machine.state())
            )));
        }
        let Ok(()) = TextToSpeech::speak(&mut self.tts, text);
        Outcome {
            body: ResponseBody::ok(
                self.snapshot(Some(format!("said: {text}")), Some("mock tts".to_owned())),
            ),
            events: Vec::new(),
        }
    }

    /// Strings recorded by mock TTS (tests).
    #[cfg(test)]
    pub(crate) fn spoken(&self) -> &[String] {
        self.tts.spoken()
    }

    /// Mock backend for tests that push frames by hand.
    #[cfg(test)]
    fn capture_mock(&mut self) -> &mut softwake_audio::MockAudioCapture {
        self.capture.mock_mut()
    }

    /// Replace the PCM engine with a scripted hit queue (tests only).
    #[cfg(test)]
    fn install_scripted_pcm(&mut self, hits: impl IntoIterator<Item = PhraseHit>) {
        self.pcm = PcmEngine::scripted(hits);
        self.pcm_agent = "scripted".to_owned();
    }

    /// Replace the PCM engine with scripted hits and raw keyword tags (tests only).
    #[cfg(test)]
    fn install_scripted_pcm_keywords<S>(&mut self, hits: impl IntoIterator<Item = (PhraseHit, S)>)
    where
        S: Into<String>,
    {
        self.pcm = PcmEngine::scripted_keywords(hits);
        self.pcm_agent = "scripted".to_owned();
    }

    /// Pull every queued frame into the PCM detector.
    ///
    /// Keeps the **first** actionable [`PhraseHit::Wake`] / [`PhraseHit::Sleep`]
    /// in the batch. Later [`PhraseHit::None`] frames must not clear a wake that
    /// already scored in an earlier window of the same drain (sherpa often
    /// emits keyword then silence before the next poll).
    /// [`NullDetector`] never matches. After `stop`, the mock drops the queue,
    /// so hibernate does not score a late frame.
    fn drain_pcm(&mut self) -> Vec<WireEvent> {
        let mut hit = PhraseHit::None;
        let mut scored = false;
        let mut energy_seen = false;
        let awake = wire_state(self.machine.state()) == WireState::Awake;
        // Half-duplex: while Eve TTS plays (and a short grace after), drop mic
        // frames so speakers do not feed free-speech / KWS / PTT.
        let muted = softwake_voice::input_muted();
        let auto_ok = awake
            && !self.voice_test
            && !self.talk.is_armed()
            && self.pending_auto_pcm.is_none()
            && !self.in_voice_cooldown()
            && !muted;
        if !auto_ok && (self.auto_utt.is_buffering() || !awake || muted || self.voice_test) {
            // PTT, cooldown, TTS mute, voice test, sleep, or a pending clip — drop a partial auto buffer.
            if self.talk.is_armed()
                || !awake
                || self.in_voice_cooldown()
                || muted
                || self.voice_test
            {
                self.auto_utt.reset();
            }
        }
        while let Ok(Some(frame)) = self.capture.poll_frame() {
            let samples = frame.samples();
            let level = rms_level(samples);
            self.last_capture_level = Some(level);
            if muted {
                // Still drain the queue / HUD level; do not treat as user input.
                continue;
            }
            if self.talk.is_armed() {
                self.talk.push(samples);
            }
            if level >= 0.02 {
                energy_seen = true;
            }
            if auto_ok && self.pending_auto_pcm.is_none() {
                if let Some(pcm) = self.auto_utt.push_frame(samples, level) {
                    self.pending_auto_pcm = Some(pcm);
                    self.retain_status_text(
                        Some("thinking…".to_owned()),
                        Some("free speech".to_owned()),
                    );
                }
            }
            let observed = self.observe_kws_frame(&frame);
            scored = true;
            // Stick the first wake/sleep/hibernate; do not let later None overwrite it.
            if matches!(hit, PhraseHit::None)
                && matches!(
                    observed,
                    PhraseHit::Wake | PhraseHit::Sleep | PhraseHit::Hibernate
                )
            {
                hit = observed;
            }
        }
        if scored {
            self.last_pcm_hit = Some(hit);
        }
        if energy_seen
            && matches!(hit, PhraseHit::None)
            && self.capture.kind() == CaptureKind::PipeWire
            && wire_state(self.machine.state()) == WireState::Sleep
            && !self.pcm.weights_loaded()
        {
            let now = Instant::now();
            let should_log = self
                .last_energy_log
                .is_none_or(|previous| now.duration_since(previous).as_secs() >= 2);
            if should_log {
                self.last_energy_log = Some(now);
                eprintln!(
                    "softwaked: mic energy heard (capture_level set); wake-from-voice needs KWS weights — see README"
                );
            }
        }
        if energy_seen
            && self.verbosity >= 2
            && matches!(hit, PhraseHit::None)
            && self.capture.kind() == CaptureKind::PipeWire
            && wire_state(self.machine.state()) == WireState::Sleep
            && self.pcm.weights_loaded()
        {
            let now = Instant::now();
            let should_log = self
                .last_energy_log
                .is_none_or(|previous| now.duration_since(previous).as_secs() >= 2);
            if should_log {
                self.last_energy_log = Some(now);
                eprintln!(
                    "softwaked: KWS {} listening (sleep) mic_rms={:.3} wake_phrases=[{}] — no keyword match yet",
                    self.soul.profile_log_token(),
                    self.last_capture_level.unwrap_or(0.0),
                    self.pcm.wake_phrases().join(", ")
                );
            }
        }
        if !self.capture.is_running() {
            self.last_capture_level = None;
        }
        self.apply_pcm_hit(hit)
    }

    /// Score one frame and return the spotter hit unchanged.
    ///
    /// Short words are not dropped when free speech or press-to-talk is long.
    /// A 1.5 s suppress cut real `sleep` commands during awake chat. A 400 ms
    /// silence reset of the online stream chopped keywords mid-utterance.
    /// Both gates are gone. The 3 s stream budget still refreshes a long session.
    fn observe_kws_frame(&mut self, frame: &softwake_audio::AudioFrame) -> PhraseHit {
        let detail = score_frame_detailed(&mut self.pcm, frame);
        let level = self.last_capture_level.unwrap_or(0.0);
        self.log_kws_observation(&detail, level);
        detail.hit
    }

    /// Log one KWS observation when verbosity asks for it.
    fn log_kws_observation(&mut self, detail: &softwake_wake::SpotDetail, level: f32) {
        if self.verbosity == 0 {
            return;
        }
        let Some(keyword) = detail.keyword.as_deref() else {
            return;
        };
        let now = Instant::now();
        // Always log a real keyword immediately once; rate-limit repeats.
        let should_log = detail.hit != PhraseHit::None
            || self
                .last_kws_log
                .is_none_or(|previous| now.duration_since(previous).as_millis() >= 250);
        if !should_log {
            return;
        }
        self.last_kws_log = Some(now);
        let match_label = match detail.hit {
            PhraseHit::Wake => "match=wake",
            PhraseHit::Sleep => "match=sleep",
            PhraseHit::Hibernate => "match=hibernate",
            PhraseHit::None => "match=none",
        };
        let profile = self.soul.profile_log_token();
        let state = wire_state(self.machine.state()).as_str();
        let wake = self.pcm.wake_phrases().join(", ");
        let sleep = self.pcm.sleep_phrases().join(", ");
        let hibernate = self.pcm.hibernate_phrases().join(", ");
        eprintln!(
            "{}",
            crate::verbose_log::format_kws_heard(&crate::verbose_log::KwsHeardLine {
                profile: &profile,
                keyword,
                match_label,
                state,
                mic_rms: level,
                wake: &wake,
                sleep: &sleep,
                hibernate: &hibernate,
            })
        );
    }

    /// Rebuild the PCM detector after a soul / profile reload.
    ///
    /// Reloads ONNX only when the agent / profile name actually changed.
    /// Repeat `reload_soul` with the same agent must not reopen KWS weights
    /// (each load holds ONNX / mmap FDs).
    fn rebuild_pcm(&mut self) {
        let agent = self.soul.agent_name();
        if agent == self.pcm_agent {
            return;
        }
        self.pcm = PcmEngine::for_agent(&agent);
        self.pcm
            .log_startup(&self.soul.profile_log_token(), self.verbosity);
        self.pcm_agent = agent;
    }

    /// Apply a KWS hit to the voice machine when the state allows it.
    fn apply_pcm_hit(&mut self, hit: PhraseHit) -> Vec<WireEvent> {
        match hit {
            PhraseHit::Wake if wire_state(self.machine.state()) == WireState::Sleep => {
                let outcome = self.wake_phrase();
                self.log_pcm_phrase_refuse("wake", &outcome);
                outcome.events
            }
            PhraseHit::Sleep if wire_state(self.machine.state()) == WireState::Awake => {
                let outcome = self.sleep_phrase();
                self.log_pcm_phrase_refuse("sleep", &outcome);
                outcome.events
            }
            PhraseHit::Hibernate
                if matches!(
                    wire_state(self.machine.state()),
                    WireState::Sleep | WireState::Awake
                ) =>
            {
                let outcome = self.hibernate_phrase();
                self.log_pcm_phrase_refuse("hibernate", &outcome);
                outcome.events
            }
            _ => Vec::new(),
        }
    }

    /// When a KWS match did not change state (soul refusal, cooldown, …), log why.
    fn log_pcm_phrase_refuse(&self, kind: &str, outcome: &Outcome) {
        if self.verbosity == 0 {
            return;
        }
        let ResponseBody::Err { error } = &outcome.body else {
            return;
        };
        eprintln!(
            "{}",
            crate::verbose_log::format_kws_refuse(
                &self.soul.profile_log_token(),
                kind,
                &error.to_string()
            )
        );
    }

    /// Leave awake on a sleep phrase (KWS or typed path).
    pub(crate) fn sleep_phrase(&mut self) -> Outcome {
        self.tick();
        self.apply_voice(Event::SleepPhrase, |error| {
            IpcError::protocol(error.to_string())
        })
    }

    /// Enter hibernate on `deep sleep`. Voice cannot leave hibernate.
    pub(crate) fn hibernate_phrase(&mut self) -> Outcome {
        self.tick();
        self.apply_voice(Event::HibernatePhrase, |error| {
            IpcError::protocol(error.to_string())
        })
    }

    /// Enable or disable voice test mode without changing the voice state.
    pub(crate) fn set_voice_test(&mut self, enabled: bool) -> Outcome {
        self.voice_test = enabled;
        if enabled {
            self.auto_utt.reset();
            self.pending_auto_pcm = None;
            self.talk.clear();
        }
        let note = if enabled {
            "voice test: on"
        } else {
            "voice test: off"
        };
        self.retain_status_text(Some(note.to_owned()), Some(note.to_owned()));
        Self::quiet(self.snapshot(Some(note.to_owned()), Some(note.to_owned())))
    }

    fn voice_test_speech_outcome(&mut self) -> Outcome {
        self.auto_utt.reset();
        self.pending_auto_pcm = None;
        self.talk.clear();
        let note = "voice test: speech not sent to chat";
        self.retain_status_text(Some(note.to_owned()), Some(note.to_owned()));
        Self::quiet(self.snapshot(Some(note.to_owned()), Some(note.to_owned())))
    }
    fn transition(&mut self, command: Command) -> Outcome {
        self.tick();
        let event = match command {
            Command::Hibernate => Event::UiHibernate,
            Command::WakeFromUi => Event::UiResume,
            Command::Sleep => Event::UiSleep,
            Command::GetStatus | Command::ReloadSoul => {
                return protocol_outcome("command is not a voice transition");
            }
        };
        self.apply_voice(event, |error| map_error(command, error))
    }

    fn apply_voice(
        &mut self,
        event: Event,
        map_err: impl FnOnce(StateError) -> IpcError,
    ) -> Outcome {
        let previous = wire_state(self.machine.state());
        match self.machine.apply(event) {
            Ok(applied) => {
                if event == Event::WakePhrase {
                    self.soul.commit_awake();
                    self.sync_shell_glossary();
                }
                let cleared = self.apply_effects(applied.effects);
                if self.verbosity >= 1 {
                    eprintln!(
                        "{}",
                        crate::verbose_log::format_voice_transition(
                            &self.soul.profile_log_token(),
                            applied.from.as_str(),
                            applied.to.as_str(),
                            event.as_str(),
                        )
                    );
                }
                self.announce_transition(applied.to);
                let state = wire_state(self.machine.state());
                let capture_running = self.capture.is_running();
                let detail = Some(format!("{previous} -> {state}"));
                let mut events = vec![WireEvent::StateChanged {
                    state,
                    previous,
                    capture_running,
                    detail: detail.clone(),
                }];
                events.extend(cleared);
                Outcome {
                    body: ResponseBody::ok(self.snapshot(None, detail)),
                    events,
                }
            }
            Err(error) => Self::rejected(map_err(error)),
        }
    }

    fn quiet(snapshot: Status) -> Outcome {
        Outcome {
            body: ResponseBody::ok(snapshot),
            events: Vec::new(),
        }
    }

    fn rejected(error: IpcError) -> Outcome {
        Outcome {
            body: ResponseBody::Err { error },
            events: Vec::new(),
        }
    }

    fn apply_effects(&mut self, effects: &[Effect]) -> Vec<WireEvent> {
        let mut extra = Vec::new();
        for effect in effects {
            match effect {
                Effect::OpenSession => self.open_session(),
                Effect::ReleaseActingResources => {
                    self.session.close();
                    self.last_context_used = None;
                    self.last_context_limit = None;
                    self.last_context_compacted = false;
                    self.talk.clear();
                    self.auto_utt.reset();
                    self.pending_auto_pcm = None;
                    self.last_status_message = None;
                    self.last_status_detail = None;
                    let _ = self.stt.drain();
                    if let Some(cancelled) = self.hands.clear_pending() {
                        extra.push(WireEvent::ToolConfirmResolved {
                            pending_id: cancelled.pending_id,
                            accepted: false,
                        });
                    }
                }
                Effect::StopCapture => {
                    let _ = self.capture.stop();
                    self.last_capture_level = None;
                }
                Effect::StartCapture => {
                    if let Err(error) = self.capture.start() {
                        eprintln!("softwaked: restart capture failed: {error}");
                    }
                }
            }
        }
        extra
    }

    fn open_session(&mut self) {
        // `commit_awake` runs before effects on a wake phrase. A missing pack
        // never reaches this effect; leave the session closed if it does.
        let Some(instructions) = self.soul.applied_instructions() else {
            return;
        };
        self.session = TextStubSession::open(instructions.to_owned());
    }

    fn snapshot(&self, message: Option<String>, detail: Option<String>) -> Status {
        Status {
            state: wire_state(self.machine.state()),
            capture_running: self.capture.is_running(),
            capture_level: if self.capture.is_running() {
                self.last_capture_level
            } else {
                None
            },
            soul_reload_pending: self.soul.reload_pending(),
            soul: Some(self.soul.report()),
            message,
            detail,
            pending_tool: self.pending_wire(),
            last_tool: self.hands.last_tool_line(),
            talking: self.talk.is_armed(),
            auto_listening: self.auto_listening_active(),
            context_used: self
                .last_context_used
                .filter(|_| self.session.phase() == softwake_session::SessionPhase::Open),
            context_limit: self
                .last_context_limit
                .filter(|_| self.session.phase() == softwake_session::SessionPhase::Open),
            context_compacted: self.last_context_compacted
                && self.session.phase() == softwake_session::SessionPhase::Open,
            voice_test: self.voice_test,
        }
    }

    fn announce_transition(&mut self, to: VoiceState) {
        #[cfg(test)]
        {
            self.announced_prompts
                .push(crate::announce::prompt_for(to).to_owned());
        }
        #[cfg(not(test))]
        {
            let system = self.announcement_system();
            let profile = self.soul.profile_log_token();
            crate::announce::spawn_announcement(system, to, profile, self.verbosity);
        }
    }

    #[cfg(not(test))]
    fn announcement_system(&self) -> String {
        if let Some(text) = self.soul.applied_instructions() {
            return text.to_owned();
        }
        if let Some(pack) = self.soul.pack() {
            return pack.render_instructions_as(&self.soul.agent_name());
        }
        String::new()
    }

    #[cfg(test)]
    pub(crate) fn announced_prompts(&self) -> &[String] {
        &self.announced_prompts
    }

    fn auto_listening_active(&self) -> bool {
        wire_state(self.machine.state()) == WireState::Awake
            && !self.voice_test
            && self.capture.is_running()
            && !self.talk.is_armed()
            && self.pending_auto_pcm.is_none()
            && !self.in_voice_cooldown()
            && !softwake_voice::input_muted()
    }

    fn in_voice_cooldown(&self) -> bool {
        self.last_voice_activity
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(2500))
    }

    /// Take a finished free-speech utterance for STT→ask (serve calls after `GetStatus`).
    pub(crate) fn take_pending_auto_pcm(&mut self) -> Option<Vec<i16>> {
        self.pending_auto_pcm.take()
    }

    /// Remember the latest HUD-facing message so polls see free-speech / async replies.
    fn retain_status_text(&mut self, message: Option<String>, detail: Option<String>) {
        if message.is_some() {
            self.last_status_message = message;
        }
        if detail.is_some() {
            self.last_status_detail = detail;
        }
    }

    fn pending_wire(&self) -> Option<PendingTool> {
        self.hands.pending().map(|pending| PendingTool {
            pending_id: pending.pending_id,
            name: pending.name,
            args: pending.args,
            description: pending.description,
        })
    }

    fn outcome_for_request(&self, step: Result<RequestOutcome, DispatchError>) -> Outcome {
        match step {
            Ok(RequestOutcome::Ran(ran)) => self.ran_outcome(ran),
            Ok(RequestOutcome::Pending(pending)) => self.pending_outcome(pending),
            Err(error) => Self::rejected(tool_ipc_error(&error)),
        }
    }

    fn ran_outcome(&self, ran: RanTool) -> Outcome {
        let RanTool { name, detail } = ran;
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(detail.clone()), Some(detail.clone()))),
            events: vec![
                WireEvent::ToolStarted { name: name.clone() },
                WireEvent::ToolFinished {
                    name,
                    detail: Some(detail),
                },
            ],
        }
    }

    fn pending_outcome(&self, pending: PendingToolCall) -> Outcome {
        let PendingToolCall {
            pending_id,
            name,
            args,
            description,
        } = pending;
        let message = format!("pending confirmation {pending_id} for {name}");
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(message), Some(description.clone()))),
            events: vec![WireEvent::ToolConfirmPending {
                pending_id,
                name,
                args,
                description,
            }],
        }
    }

    fn confirmed_outcome(&self, confirmed: ConfirmedTool) -> Outcome {
        let ConfirmedTool {
            pending_id,
            name,
            detail,
        } = confirmed;
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(detail.clone()), Some(detail.clone()))),
            events: vec![
                WireEvent::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
                WireEvent::ToolStarted { name: name.clone() },
                WireEvent::ToolFinished {
                    name,
                    detail: Some(detail),
                },
            ],
        }
    }

    fn cancelled_outcome(&self, cancelled: CancelledTool) -> Outcome {
        let message = format!("cancelled {}", cancelled.pending_id);
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(message), None)),
            events: vec![WireEvent::ToolConfirmResolved {
                pending_id: cancelled.pending_id,
                accepted: false,
            }],
        }
    }
}

fn protocol_outcome(message: &str) -> Outcome {
    Outcome {
        body: ResponseBody::Err {
            error: IpcError::protocol(message),
        },
        events: Vec::new(),
    }
}

fn tool_ipc_error(error: &DispatchError) -> IpcError {
    match error {
        DispatchError::Forbidden { name, state } => IpcError::ToolForbidden {
            name: name.clone(),
            state: wire_state(*state),
        },
        DispatchError::Unknown { name } => IpcError::UnknownTool { name: name.clone() },
        DispatchError::Denied { name } => IpcError::ToolDenied { name: name.clone() },
        DispatchError::Busy { pending_id } => IpcError::ConfirmationPending {
            pending_id: pending_id.clone(),
        },
        DispatchError::UnknownPending { pending_id } => IpcError::UnknownPending {
            pending_id: pending_id.clone(),
        },
        DispatchError::PendingMismatch { pending_id, name } => IpcError::PendingMismatch {
            pending_id: pending_id.clone(),
            name: name.clone(),
        },
        DispatchError::ConfirmForbidden { pending_id, state } => IpcError::ConfirmForbidden {
            pending_id: pending_id.clone(),
            state: wire_state(*state),
        },
        DispatchError::InvalidArgs { .. }
        | DispatchError::Connector { .. }
        | DispatchError::Shell { .. } => IpcError::protocol(error.to_string()),
    }
}

fn wire_state(state: VoiceState) -> WireState {
    match state {
        VoiceState::Sleep => WireState::Sleep,
        VoiceState::Awake => WireState::Awake,
        VoiceState::Hibernate => WireState::Hibernate,
    }
}

fn map_error(command: Command, error: StateError) -> IpcError {
    match error {
        StateError::IllegalTransition { from, reason, .. } => IpcError::IllegalTransition {
            from: wire_state(from),
            command,
            reason: reason.to_owned(),
        },
        StateError::Cooldown { remaining, .. } => IpcError::Cooldown {
            command,
            remaining_ms: u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX),
            detail: Some(error.to_string()),
        },
        StateError::ToolsForbidden { .. } => IpcError::protocol(error.to_string()),
    }
}

#[cfg(test)]
mod talk_tests {
    use super::Runtime;
    use crate::soul::TestSoulDir;
    use softwake_ipc::{Event, IpcError, ResponseBody, VoiceState};
    use softwake_voice::TALK_MIN_SAMPLES;

    fn awake() -> (Runtime, TestSoulDir) {
        let dir = TestSoulDir::valid();
        let mut runtime = Runtime::new(dir.soul_dir());
        let outcome = runtime.wake_phrase();
        assert!(outcome.body.status().is_some());
        (runtime, dir)
    }

    #[test]
    fn talk_start_arms_only_while_awake_and_hibernate_refuses() {
        let dir = TestSoulDir::valid();
        let mut runtime = Runtime::new(dir.soul_dir());
        runtime.handle(softwake_ipc::Command::Hibernate);
        let refused = runtime.talk_start();
        assert!(matches!(
            refused.body,
            ResponseBody::Err {
                error: IpcError::TalkRejected { .. }
            }
        ));
        assert!(!runtime.talk.is_armed());

        let started = runtime.talk_start();
        // Still hibernating; wake is not attempted.
        assert!(started.body.status().is_none());
    }

    #[test]
    fn talk_from_sleep_wakes_then_buffers_and_stop_asks() {
        let dir = TestSoulDir::valid();
        let mut runtime = Runtime::new(dir.soul_dir());
        assert_eq!(
            runtime
                .handle(softwake_ipc::Command::GetStatus)
                .body
                .status()
                .expect("status")
                .state,
            VoiceState::Sleep
        );
        let started = runtime.talk_start();
        let status = started.body.status().expect("woke");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(status.talking);
        assert!(started.events.iter().any(|event| matches!(
            event,
            Event::StateChanged {
                state: VoiceState::Awake,
                ..
            }
        )));

        runtime.push_talk_samples(&[0; 100]);
        let short = runtime.talk_stop();
        assert!(matches!(
            short.body,
            ResponseBody::Err {
                error: IpcError::TalkRejected { ref message }
            } if message.contains("longer")
        ));

        runtime.install_chat_fixture(crate::chat::xai_key_fixture(
            true,
            Some("test-key"),
            "she replies",
        ));
        runtime.set_talk_transcript_for_test("what time is it");
        let again = runtime.talk_start();
        assert!(again.body.status().expect("armed").talking);
        runtime.push_talk_samples(&vec![1; TALK_MIN_SAMPLES]);
        let stopped = runtime.talk_stop();
        let reply = stopped.body.status().expect("asked");
        assert_eq!(reply.message.as_deref(), Some("she replies"));
        assert!(stopped.events.iter().any(|event| matches!(
            event,
            Event::FinalTranscript { text } if text == "what time is it"
        )));
        assert!(!reply.talking);
    }

    #[test]
    fn sleep_clears_an_armed_talk_buffer() {
        let (mut runtime, _dir) = awake();
        runtime.talk_start();
        runtime.push_talk_samples(&vec![1; TALK_MIN_SAMPLES]);
        runtime.handle(softwake_ipc::Command::Sleep);
        let stopped = runtime.talk_stop();
        assert!(matches!(
            stopped.body,
            ResponseBody::Err {
                error: IpcError::TalkRejected { .. }
            }
        ));
    }

    #[test]
    fn awake_auto_utterance_queues_pcm_and_sleep_does_not() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let dir = TestSoulDir::valid();
        let mut runtime = Runtime::new(dir.soul_dir());
        let loud = vec![8_000_i16; 320];
        for _ in 0..40 {
            assert!(runtime.capture_mock().push_frame(&loud));
            runtime.drain_pcm();
        }
        assert!(runtime.take_pending_auto_pcm().is_none());

        let (mut runtime, _dir) = awake();
        for _ in 0..8 {
            assert!(runtime.capture_mock().push_frame(&loud));
            runtime.drain_pcm();
        }
        let quiet = vec![0_i16; 320];
        for _ in 0..30 {
            assert!(runtime.capture_mock().push_frame(&quiet));
            runtime.drain_pcm();
            if runtime.pending_auto_pcm.is_some() {
                break;
            }
        }
        let pcm = runtime
            .take_pending_auto_pcm()
            .expect("auto utterance while awake");
        assert!(pcm.len() >= TALK_MIN_SAMPLES);
        assert!(
            runtime
                .handle(softwake_ipc::Command::GetStatus)
                .body
                .status()
                .expect("status")
                .message
                .as_deref()
                .is_some_and(|m| m.contains("thinking"))
        );
    }

    #[test]
    fn ptt_resets_auto_gate_and_status_reports_auto_listening() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _dir) = awake();
        let outcome = runtime.handle(softwake_ipc::Command::GetStatus);
        let status = outcome.body.status().expect("status");
        assert!(status.auto_listening);
        let loud = vec![8_000_i16; 320];
        for _ in 0..8 {
            assert!(runtime.capture_mock().push_frame(&loud));
            runtime.drain_pcm();
        }
        assert!(runtime.auto_utt.is_buffering());
        let started = runtime.talk_start();
        assert!(started.body.status().expect("armed").talking);
        assert!(!runtime.auto_utt.is_buffering());
        assert!(!started.body.status().expect("armed").auto_listening);
        assert!(runtime.take_pending_auto_pcm().is_none());
    }

    #[test]
    fn tts_mute_drops_free_speech_and_clears_auto_listening() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _dir) = awake();
        let mute_gen = softwake_voice::begin_input_mute();
        assert!(softwake_voice::input_muted());
        let outcome = runtime.handle(softwake_ipc::Command::GetStatus);
        let status = outcome.body.status().expect("status");
        assert!(!status.auto_listening);
        let loud = vec![8_000_i16; 320];
        for _ in 0..40 {
            assert!(runtime.capture_mock().push_frame(&loud));
            runtime.drain_pcm();
        }
        assert!(
            runtime.take_pending_auto_pcm().is_none(),
            "muted input must not queue a free-speech utterance"
        );
        softwake_voice::end_input_mute(mute_gen);
        std::thread::sleep(
            softwake_voice::PLAYBACK_MUTE_GRACE + std::time::Duration::from_millis(50),
        );
        assert!(!softwake_voice::input_muted());
        softwake_voice::clear_input_mute_for_test();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Runtime;
    use crate::soul::{TestSoulDir, reload_message};
    use softwake_ipc::{Command, Event, IpcError, ResponseBody, VoiceState};
    use softwake_session::SessionPhase;
    use softwake_wake::PhraseHit;

    fn valid_runtime() -> (Runtime, TestSoulDir) {
        let dir = TestSoulDir::valid();
        let runtime = Runtime::new(dir.soul_dir());
        (runtime, dir)
    }

    fn wake(runtime: &mut Runtime) {
        let outcome = runtime.wake_phrase();
        assert!(
            outcome.body.status().is_some(),
            "wake should apply, got {outcome:?}"
        );
    }

    #[test]
    fn get_status_reports_capture_level_from_mock_pcm_while_listening() {
        let (mut runtime, _soul) = valid_runtime();
        assert!(runtime.capture.is_running());
        let outcome = runtime.handle(Command::GetStatus);
        let status = outcome.body.status().expect("ok");
        let level = status.capture_level.expect("listening tone was scored");
        assert!(level > 0.0, "{level}");
        assert!(level <= 1.0, "{level}");

        let outcome = runtime.handle(Command::Hibernate);
        let hibernated = outcome.body.status().expect("ok");
        assert!(!hibernated.capture_running);
        assert!(hibernated.capture_level.is_none());
    }

    #[test]
    fn queued_silence_is_scored_before_a_command_and_dropped_after_hibernate() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _soul) = valid_runtime();
        assert_eq!(runtime.last_pcm_hit, None);
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        let status = runtime.handle(Command::GetStatus);
        assert_eq!(
            status.body.status().expect("status").state,
            VoiceState::Sleep
        );
        assert_eq!(runtime.last_pcm_hit, Some(PhraseHit::None));
        assert!(runtime.capture_mock().poll_frame().is_none());

        runtime.handle(Command::Hibernate);
        assert!(!runtime.capture_mock().push_frame(&[0; 160]));
        runtime.last_pcm_hit = None;
        runtime.handle(Command::GetStatus);
        assert_eq!(runtime.last_pcm_hit, None);
    }

    #[test]
    fn drain_keeps_first_wake_hit_when_later_frames_are_none() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _soul) = valid_runtime();
        assert_eq!(
            runtime
                .handle(Command::GetStatus)
                .body
                .status()
                .expect("status")
                .state,
            VoiceState::Sleep
        );
        // One wake-scoring frame, then silence frames in the same drain batch —
        // the old bug overwrote Wake with later None and never called wake_phrase.
        runtime.install_scripted_pcm([PhraseHit::Wake, PhraseHit::None, PhraseHit::None]);
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        let events = runtime.drain_pcm();
        assert_eq!(runtime.last_pcm_hit, Some(PhraseHit::Wake));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Awake);
        assert!(
            runtime
                .announced_prompts()
                .iter()
                .any(|prompt| prompt.contains("now awake"))
        );
        assert!(
            runtime
                .session_turns()
                .iter()
                .all(|turn| !turn.contains("Tell the user"))
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::StateChanged {
                    state: VoiceState::Awake,
                    previous: VoiceState::Sleep,
                    ..
                }
            )),
            "expected sleep→awake event, got {events:?}"
        );
    }

    /// Bare `sleep` must still apply after a long awake utterance.
    ///
    /// The withdrawn gate ignored short keywords once free speech had buffered
    /// `24_000` samples (1.5 s at 16 kHz). This fills past that point, then
    /// scores keyword `sleep`. Reintroducing the suppress turns this into a
    /// stay-awake failure.
    #[test]
    fn short_sleep_stands_after_a_long_awake_buffer() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.machine.advance(Duration::from_millis(800));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Awake);

        // 20 ms frames. Stay loud so the energy gate keeps the utterance open
        // (silence would end it and clear the buffer before the keyword).
        let loud = vec![8_000_i16; 320];
        let mut pushed = 0_u32;
        while runtime.auto_utt.buffered_samples() < 24_000 {
            assert!(
                runtime.capture_mock().push_frame(&loud),
                "capture stopped while filling the awake buffer"
            );
            runtime.drain_pcm();
            pushed += 1;
            assert!(
                pushed < 400,
                "utterance closed before 1.5 s of speech was buffered"
            );
        }
        assert!(runtime.auto_utt.is_buffering());
        assert!(
            runtime.pending_auto_pcm.is_none(),
            "the utterance must still be open when sleep is scored"
        );
        assert!(runtime.auto_utt.buffered_samples() >= 24_000);
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Awake);

        runtime.install_scripted_pcm_keywords([(PhraseHit::Sleep, "sleep")]);
        assert!(runtime.capture_mock().push_frame(&loud));
        runtime.drain_pcm();
        assert_eq!(runtime.last_pcm_hit, Some(PhraseHit::Sleep));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
    }

    #[test]
    fn scripted_deep_sleep_enters_hibernate_from_sleep_and_awake() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _soul) = valid_runtime();
        runtime.install_scripted_pcm([PhraseHit::Hibernate]);
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        runtime.drain_pcm();
        assert_eq!(
            runtime.machine.state(),
            softwake_state::VoiceState::Hibernate
        );
        assert!(!runtime.capture.is_running());
        assert!(
            runtime
                .announced_prompts()
                .last()
                .is_some_and(|prompt| prompt.contains("deep sleep"))
        );

        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.install_scripted_pcm([PhraseHit::Hibernate]);
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        runtime.drain_pcm();
        assert_eq!(
            runtime.machine.state(),
            softwake_state::VoiceState::Hibernate
        );
        assert!(!runtime.capture.is_running());
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(
            runtime
                .session_turns()
                .iter()
                .all(|turn| !turn.contains("Tell the user"))
        );
    }

    #[test]
    fn phrase_hits_cannot_leave_hibernate() {
        let (mut runtime, _soul) = valid_runtime();
        runtime.handle(Command::Hibernate);
        assert!(runtime.apply_pcm_hit(PhraseHit::Wake).is_empty());
        assert!(runtime.apply_pcm_hit(PhraseHit::Sleep).is_empty());
        assert!(runtime.apply_pcm_hit(PhraseHit::Hibernate).is_empty());
        assert_eq!(
            runtime.machine.state(),
            softwake_state::VoiceState::Hibernate
        );
    }

    #[test]
    fn voice_test_keeps_wake_and_does_not_send_speech_to_chat() {
        let _guard = softwake_voice::INPUT_MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        softwake_voice::clear_input_mute_for_test();
        let (mut runtime, _soul) = valid_runtime();
        let enabled = runtime.set_voice_test(true);
        assert!(enabled.body.status().expect("on").voice_test);
        runtime.install_scripted_pcm([PhraseHit::Wake]);
        assert!(runtime.capture_mock().push_frame(&[0; 160]));
        runtime.drain_pcm();
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Awake);
        assert!(runtime.session_turns().is_empty());

        runtime.set_talk_transcript_for_test("what time is it");
        let stopped = runtime.talk_stop();
        let status = stopped.body.status().expect("voice test");
        assert_eq!(
            status.message.as_deref(),
            Some("voice test: speech not sent to chat")
        );
        assert!(runtime.chat_posts().is_empty());
        assert!(runtime.session_turns().is_empty());

        let loud = vec![8_000_i16; 320];
        for _ in 0..40 {
            assert!(runtime.capture_mock().push_frame(&loud));
            runtime.drain_pcm();
        }
        assert!(runtime.take_pending_auto_pcm().is_none());

        runtime.set_voice_test(false);
        assert!(
            !runtime
                .handle(Command::GetStatus)
                .body
                .status()
                .expect("off")
                .voice_test
        );
        for _ in 0..8 {
            assert!(runtime.capture_mock().push_frame(&loud));
            runtime.drain_pcm();
        }
        let quiet = vec![0_i16; 320];
        for _ in 0..30 {
            assert!(runtime.capture_mock().push_frame(&quiet));
            runtime.drain_pcm();
            if runtime.pending_auto_pcm.is_some() {
                break;
            }
        }
        assert!(
            runtime.take_pending_auto_pcm().is_some(),
            "voice test off restores free-speech capture"
        );
    }

    #[test]
    fn sleep_from_awake_keeps_capture_and_hibernate_stops_it() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        let slept = runtime.handle(Command::Sleep);
        let status = slept.body.status().expect("slept");
        assert_eq!(status.state, VoiceState::Sleep);
        assert!(status.capture_running);
        assert_eq!(status.detail.as_deref(), Some("awake -> sleep"));
        assert!(status.soul.as_ref().is_some_and(|soul| soul.ok));
        assert!(matches!(
            slept.events.as_slice(),
            [Event::StateChanged {
                state: VoiceState::Sleep,
                previous: VoiceState::Awake,
                capture_running: true,
                ..
            }]
        ));

        runtime
            .machine
            .advance(std::time::Duration::from_millis(800));
        wake(&mut runtime);
        let hibernated = runtime.handle(Command::Hibernate);
        let status = hibernated.body.status().expect("hibernated");
        assert_eq!(status.state, VoiceState::Hibernate);
        assert!(!status.capture_running);
        assert!(matches!(
            hibernated.events.as_slice(),
            [Event::StateChanged {
                capture_running: false,
                ..
            }]
        ));
    }

    #[test]
    fn wake_after_sleep_and_resume_respects_the_phrase_cooldown() {
        let (mut runtime, _soul) = valid_runtime();
        let woke = runtime.wake_phrase();
        assert_eq!(woke.body.status().expect("awake").state, VoiceState::Awake);

        runtime.handle(Command::Sleep);
        let cooled = runtime.wake_phrase();
        match cooled.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => assert!(
                message.contains("cannot apply wake phrase during cooldown"),
                "{message}"
            ),
            other => panic!("expected cooldown, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        runtime.machine.advance(Duration::from_millis(800));
        let again = runtime.wake_phrase();
        assert_eq!(
            again.body.status().expect("awake after cooldown").state,
            VoiceState::Awake
        );

        runtime.handle(Command::Hibernate);
        let resumed = runtime.handle(Command::WakeFromUi);
        assert_eq!(
            resumed.body.status().expect("resume").state,
            VoiceState::Sleep
        );
        let cooled = runtime.wake_phrase();
        match cooled.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => assert!(message.contains("during cooldown"), "{message}"),
            other => panic!("expected cooldown after resume, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        runtime.machine.advance(Duration::from_millis(800));
        let woke = runtime.wake_phrase();
        assert_eq!(
            woke.body
                .status()
                .expect("awake after resume cooldown")
                .state,
            VoiceState::Awake
        );
    }

    #[test]
    fn illegal_sleep_does_not_emit_a_state_change() {
        let (mut runtime, _soul) = valid_runtime();
        let outcome = runtime.handle(Command::Sleep);
        assert!(outcome.events.is_empty());
        match outcome.body {
            ResponseBody::Err {
                error: IpcError::IllegalTransition { from, command, .. },
            } => {
                assert_eq!(from, VoiceState::Sleep);
                assert_eq!(command, Command::Sleep);
            }
            other => panic!("expected illegal transition, got {other:?}"),
        }
        let status = runtime.handle(Command::GetStatus);
        assert!(status.events.is_empty());
        assert_eq!(
            status.body.status().expect("status").state,
            VoiceState::Sleep
        );
    }

    #[test]
    fn reload_sticks_across_hibernate_without_a_state_event() {
        let (mut runtime, _soul) = valid_runtime();
        let reloaded = runtime.handle(Command::ReloadSoul);
        assert!(reloaded.events.is_empty());
        let status = reloaded.body.status().expect("reload");
        assert!(status.soul_reload_pending);
        assert!(status.soul.as_ref().is_some_and(|soul| soul.ok));
        let expected = reload_message(true, None);
        assert!(expected.contains("reloaded soul pack; applies on next awake"));
        assert!(expected.contains("rules.md"));
        assert!(expected.contains("glossary.md"));
        assert_eq!(status.message.as_deref(), Some(expected.as_str()));

        runtime.handle(Command::Hibernate);
        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Hibernate);
        assert!(status.soul_reload_pending);
        assert!(status.message.is_none());
        assert!(status.soul.as_ref().is_some_and(|soul| soul.ok));
    }

    #[test]
    fn missing_soul_refuses_awake_and_leaves_hibernate_usable() {
        let soul = TestSoulDir::empty();
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        match refused.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => {
                assert!(message.contains("refusing awake"), "{message}");
                assert!(message.contains("missing soul.md"), "{message}");
            }
            other => panic!("expected a soul refusal, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.capture.is_running());
        assert!(runtime.machine.permit_tool_dispatch().is_err());
        assert!(runtime.applied_instructions().is_none());

        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Sleep);
        assert!(status.capture_running);
        assert!(!status.soul_reload_pending);
        let report = status.soul.expect("soul report");
        assert!(!report.ok);
        assert!(
            report
                .reason
                .unwrap_or_default()
                .contains("missing soul.md")
        );

        let hibernated = runtime.handle(Command::Hibernate);
        assert_eq!(
            hibernated.body.status().expect("hibernate").state,
            VoiceState::Hibernate
        );
        assert!(!runtime.capture.is_running());
        let resumed = runtime.handle(Command::WakeFromUi);
        assert_eq!(
            resumed.body.status().expect("resume").state,
            VoiceState::Sleep
        );
        assert!(runtime.capture.is_running());
        let slept = runtime.handle(Command::Sleep);
        assert!(matches!(
            slept.body,
            ResponseBody::Err {
                error: IpcError::IllegalTransition { .. }
            }
        ));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
    }

    #[test]
    fn reload_then_wake_applies_the_fixed_pack() {
        let soul = TestSoulDir::empty();
        let mut runtime = Runtime::new(soul.soul_dir());
        assert!(runtime.wake_phrase().events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        soul.write("fixed soul\n", "fixed user\n");
        // Files changed on disk. Without reload the cached miss still refuses.
        assert!(runtime.wake_phrase().events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        let reloaded = runtime.handle(Command::ReloadSoul);
        let status = reloaded.body.status().expect("reload");
        assert!(status.soul_reload_pending);
        assert!(status.soul.as_ref().is_some_and(|report| report.ok));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.applied_instructions().is_none());

        let woke = runtime.wake_phrase();
        let status = woke.body.status().expect("awake");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(!status.soul_reload_pending);
        assert!(matches!(
            woke.events.as_slice(),
            [Event::StateChanged {
                state: VoiceState::Awake,
                previous: VoiceState::Sleep,
                ..
            }]
        ));
        assert!(runtime.machine.permit_tool_dispatch().is_ok());
        let applied = runtime.applied_instructions().expect("applied");
        assert!(applied.contains("# Identity"));
        assert!(applied.contains("fixed soul"));
        assert!(applied.contains("# User profile"));
        assert!(applied.contains("fixed user"));
        assert!(applied.contains("# Rules"));
        assert!(applied.contains("# Glossary"));
        assert!(applied.contains("# Runtime policy"));
        assert!(applied.contains("State: awake."));
    }

    #[test]
    fn missing_rules_refuses_awake_and_leaves_other_transitions() {
        let soul = TestSoulDir::valid();
        std::fs::remove_file(soul.path().join("rules.md")).expect("remove rules");
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        match refused.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => {
                assert!(message.contains("refusing awake"), "{message}");
                assert!(message.contains("rules.md"), "{message}");
            }
            other => panic!("expected a soul refusal, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.capture.is_running());
        assert!(runtime.applied_instructions().is_none());

        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Sleep);
        let report = status.soul.expect("soul report");
        assert!(!report.ok);
        assert!(report.reason.unwrap_or_default().contains("rules.md"));

        let hibernated = runtime.handle(Command::Hibernate);
        assert_eq!(
            hibernated.body.status().expect("hibernate").state,
            VoiceState::Hibernate
        );
        assert!(!runtime.capture.is_running());
        let resumed = runtime.handle(Command::WakeFromUi);
        assert_eq!(
            resumed.body.status().expect("resume").state,
            VoiceState::Sleep
        );
        assert!(runtime.capture.is_running());
        let slept = runtime.handle(Command::Sleep);
        assert!(matches!(
            slept.body,
            ResponseBody::Err {
                error: IpcError::IllegalTransition { .. }
            }
        ));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
    }

    #[test]
    fn duplicate_glossary_alias_refuses_awake() {
        let soul = TestSoulDir::valid();
        std::fs::write(
            soul.path().join("glossary.md"),
            "docs → /path/to/docs\ndocs → /path/to/other\n",
        )
        .expect("glossary");
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.applied_instructions().is_none());
        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Sleep);
        let report = status.soul.expect("soul report");
        assert!(!report.ok);
        assert!(
            report
                .reason
                .unwrap_or_default()
                .contains("duplicate alias docs")
        );
    }

    #[test]
    fn empty_file_refuses_awake_until_reload() {
        let soul = TestSoulDir::valid();
        soul.write("   \n", "user\n");
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        match refused.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => assert!(message.contains("empty"), "{message}"),
            other => panic!("expected a soul refusal, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        soul.write("soul\n", "user\n");
        let reloaded = runtime.handle(Command::ReloadSoul);
        let expected = reload_message(true, None);
        assert_eq!(
            reloaded.body.status().expect("reload").message.as_deref(),
            Some(expected.as_str())
        );
        wake(&mut runtime);
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Awake);
        assert!(!runtime.soul.reload_pending());
    }

    #[test]
    fn reload_while_awake_waits_for_the_next_awake_entry() {
        let (mut runtime, soul) = valid_runtime();
        wake(&mut runtime);
        let original = runtime.applied_instructions().expect("applied").to_owned();
        assert!(original.contains("test soul"));

        soul.write("updated soul\n", "updated user\n");
        let reloaded = runtime.handle(Command::ReloadSoul);
        let status = reloaded.body.status().expect("reload");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(status.soul_reload_pending);
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));

        runtime.handle(Command::Sleep);
        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        let applied = runtime.applied_instructions().expect("new pack");
        assert!(applied.contains("updated soul"));
        assert!(applied.contains("updated user"));
        assert!(!applied.contains("test soul"));
        assert!(!runtime.soul.reload_pending());
    }

    #[test]
    fn invalid_reload_while_awake_keeps_the_session_and_blocks_the_next_wake() {
        let (mut runtime, soul) = valid_runtime();
        wake(&mut runtime);
        let original = runtime.applied_instructions().expect("applied").to_owned();

        std::fs::remove_file(soul.path().join("user.md")).expect("remove user");
        let reloaded = runtime.handle(Command::ReloadSoul);
        let status = reloaded.body.status().expect("reload");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(status.soul_reload_pending);
        assert!(status.soul.as_ref().is_some_and(|report| !report.ok));
        let reason = status
            .soul
            .as_ref()
            .and_then(|report| report.reason.clone());
        let expected = reload_message(false, reason.as_deref());
        assert_eq!(status.message.as_deref(), Some(expected.as_str()));
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));
        assert!(runtime.machine.permit_tool_dispatch().is_ok());

        runtime.handle(Command::Sleep);
        runtime.machine.advance(Duration::from_millis(800));
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.machine.permit_tool_dispatch().is_err());
        assert!(runtime.soul.reload_pending());
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());
    }

    #[test]
    fn echo_runs_only_while_awake() {
        let (mut runtime, _soul) = valid_runtime();

        let asleep = runtime.invoke_tool("echo", &["hello".to_owned()]);
        assert!(asleep.events.is_empty());
        assert!(matches!(
            asleep.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    name,
                    state: VoiceState::Sleep,
                }
            } if name == "echo"
        ));

        let unknown_asleep = runtime.invoke_tool("volume", &[]);
        assert!(unknown_asleep.events.is_empty());
        assert!(matches!(
            unknown_asleep.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));

        wake(&mut runtime);
        let ran = runtime.invoke_tool("echo", &["hello".to_owned(), "world".to_owned()]);
        let status = ran.body.status().expect("echo");
        assert_eq!(status.message.as_deref(), Some("echo: hello world"));
        assert_eq!(status.detail.as_deref(), Some("echo: hello world"));
        assert!(matches!(
            ran.events.as_slice(),
            [
                Event::ToolStarted { name: started },
                Event::ToolFinished {
                    name: finished,
                    detail: Some(detail),
                },
            ] if started == "echo" && finished == "echo" && detail == "echo: hello world"
        ));
        assert_eq!(
            runtime
                .invoke_tool("echo", &[])
                .body
                .status()
                .expect("pong")
                .message
                .as_deref(),
            Some("pong")
        );

        let unknown = runtime.invoke_tool("volume", &[]);
        assert!(unknown.events.is_empty());
        assert!(matches!(
            unknown.body,
            ResponseBody::Err {
                error: IpcError::UnknownTool { name }
            } if name == "volume"
        ));

        runtime.handle(Command::Sleep);
        assert!(matches!(
            runtime.invoke_tool("echo", &[]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        runtime.handle(Command::Hibernate);
        let hibernated = runtime.invoke_tool("echo", &["x".to_owned()]);
        assert!(hibernated.events.is_empty());
        assert!(matches!(
            hibernated.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Hibernate,
                    ..
                }
            }
        ));
    }

    #[test]
    fn session_opens_with_soul_instructions_and_closes_when_acting_stops() {
        let (mut runtime, soul) = valid_runtime();
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());

        wake(&mut runtime);
        assert_eq!(runtime.session_phase(), SessionPhase::Open);
        let instructions = runtime
            .session_instructions()
            .expect("session opened")
            .to_owned();
        assert!(instructions.contains("test soul"));
        assert!(instructions.contains("test user"));
        assert!(instructions.contains("echo (safe)"));
        assert!(instructions.contains("notify (confirm)"));
        assert!(instructions.contains("email_send (confirm)"));
        assert_eq!(runtime.applied_instructions(), Some(instructions.as_str()));

        soul.write("updated soul\n", "updated user\n");
        runtime.handle(Command::ReloadSoul);
        assert_eq!(runtime.session_instructions(), Some(instructions.as_str()));

        runtime.handle(Command::Sleep);
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());
        assert!(
            runtime
                .applied_instructions()
                .expect("last applied pack")
                .contains("test soul")
        );

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        let reopened = runtime.session_instructions().expect("reopened");
        assert!(reopened.contains("updated soul"));
        assert!(reopened.contains("updated user"));
        assert!(reopened.contains("echo (safe)"));
        assert!(reopened.contains("notify (confirm)"));
        assert!(reopened.contains("email_send (confirm)"));

        runtime.handle(Command::Hibernate);
        assert_eq!(
            runtime.machine.state(),
            softwake_state::VoiceState::Hibernate
        );
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());
    }

    #[test]
    fn notify_stays_pending_until_confirm_and_runs_once() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);

        let pending = runtime.invoke_tool("notify", &["hello".to_owned()]);
        let status = pending.body.status().expect("pending");
        assert_eq!(
            status.message.as_deref(),
            Some("pending confirmation 1 for notify")
        );
        let waiting = status.pending_tool.clone().expect("pending tool");
        assert_eq!(waiting.pending_id, "1");
        assert_eq!(waiting.name, "notify");
        assert_eq!(waiting.args, ["hello"]);
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            pending.events.as_slice(),
            [Event::ToolConfirmPending {
                pending_id,
                name,
                ..
            }] if pending_id == "1" && name == "notify"
        ));
        assert!(pending.events.iter().all(|event| !matches!(
            event,
            Event::ToolStarted { .. } | Event::ToolFinished { .. }
        )));

        let busy = runtime.invoke_tool("notify", &["other".to_owned()]);
        assert!(matches!(
            busy.body,
            ResponseBody::Err {
                error: IpcError::ConfirmationPending { ref pending_id }
            } if pending_id == "1"
        ));
        assert!(runtime.hands.notifications().is_empty());
        assert_eq!(runtime.hands.pending_id().as_deref(), Some("1"));

        let confirmed = runtime.confirm_tool("1", Some("notify"));
        let status = confirmed.body.status().expect("confirmed");
        assert_eq!(status.message.as_deref(), Some("hello"));
        assert!(status.pending_tool.is_none());
        assert_eq!(
            runtime.hands.notifications().iter().collect::<Vec<_>>(),
            ["hello"]
        );
        assert!(matches!(
            confirmed.events.as_slice(),
            [
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
                Event::ToolStarted { name: started },
                Event::ToolFinished {
                    name: finished,
                    detail: Some(detail),
                },
            ] if pending_id == "1"
                && started == "notify"
                && finished == "notify"
                && detail == "hello"
        ));

        let again = runtime.confirm_tool("1", None);
        assert!(matches!(
            again.body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { ref pending_id }
            } if pending_id == "1"
        ));
        assert_eq!(runtime.hands.notifications().len(), 1);
    }

    #[test]
    fn email_send_confirms_once_onto_the_outbox() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        let args = vec![
            "ada@example.com".to_owned(),
            "hello there".to_owned(),
            "line one".to_owned(),
        ];

        let pending = runtime.invoke_tool("email_send", &args);
        let status = pending.body.status().expect("pending");
        assert_eq!(
            status.message.as_deref(),
            Some("pending confirmation 1 for email_send")
        );
        let waiting = status.pending_tool.clone().expect("pending tool");
        assert_eq!(waiting.name, "email_send");
        assert_eq!(waiting.args, args);
        assert!(runtime.hands.outbox().is_empty());
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            pending.events.as_slice(),
            [Event::ToolConfirmPending { name, .. }] if name == "email_send"
        ));
        assert!(pending.events.iter().all(|event| !matches!(
            event,
            Event::ToolStarted { .. } | Event::ToolFinished { .. }
        )));

        let short = runtime.invoke_tool("email_send", &["ada@example.com".to_owned()]);
        assert!(matches!(
            short.body,
            ResponseBody::Err {
                error: IpcError::ConfirmationPending { .. }
            }
        ));
        assert!(runtime.hands.outbox().is_empty());

        let confirmed = runtime.confirm_tool("1", Some("email_send"));
        let status = confirmed.body.status().expect("confirmed");
        assert_eq!(status.message.as_deref(), Some("sent 1"));
        assert!(status.pending_tool.is_none());
        assert_eq!(runtime.hands.outbox().len(), 1);
        assert_eq!(runtime.hands.outbox()[0].to, "ada@example.com");
        assert_eq!(runtime.hands.outbox()[0].subject, "hello there");
        assert_eq!(runtime.hands.outbox()[0].body, "line one");
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            confirmed.events.as_slice(),
            [
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
                Event::ToolStarted { name: started },
                Event::ToolFinished {
                    name: finished,
                    detail: Some(detail),
                },
            ] if pending_id == "1"
                && started == "email_send"
                && finished == "email_send"
                && detail == "sent 1"
        ));

        let again = runtime.confirm_tool("1", None);
        assert!(matches!(
            again.body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { .. }
            }
        ));
        assert_eq!(runtime.hands.outbox().len(), 1);
    }

    fn email_body() -> Vec<String> {
        vec![
            "ada@example.com".to_owned(),
            "hello".to_owned(),
            "body".to_owned(),
        ]
    }

    #[test]
    fn email_send_cancel_sleep_and_short_args_do_not_send() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);

        let short = runtime.invoke_tool(
            "email_send",
            &["ada@example.com".to_owned(), "hello".to_owned()],
        );
        assert!(matches!(
            short.body,
            ResponseBody::Err {
                error: IpcError::Protocol { ref message }
            } if message == "email_send needs to, subject, and body"
        ));
        assert!(short.events.is_empty());
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.outbox().is_empty());

        runtime.invoke_tool("email_send", &email_body());
        let cancelled = runtime.cancel_tool("1", None);
        assert!(matches!(
            cancelled.events.as_slice(),
            [Event::ToolConfirmResolved {
                accepted: false,
                ..
            }]
        ));
        assert!(runtime.hands.outbox().is_empty());

        runtime.invoke_tool("email_send", &email_body());
        // Leave the pending record in place while the machine is no longer awake.
        // The sleep command itself clears that record; this is the confirm-forbidden case.
        runtime
            .machine
            .apply(softwake_state::Event::UiSleep)
            .expect("sleep without clearing");
        let refused = runtime.confirm_tool("2", None);
        assert!(matches!(
            refused.body,
            ResponseBody::Err {
                error: IpcError::ConfirmForbidden { .. }
            }
        ));
        assert!(runtime.hands.outbox().is_empty());
        assert!(runtime.hands.pending().is_some());
        runtime.cancel_tool("2", None);
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.outbox().is_empty());

        let forbidden = runtime.invoke_tool("email_send", &email_body());
        assert!(matches!(
            forbidden.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));
        assert!(runtime.hands.outbox().is_empty());
    }

    #[test]
    fn sleep_and_hibernate_clear_a_pending_email_without_sending() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.invoke_tool("email_send", &email_body());

        let slept = runtime.handle(Command::Sleep);
        assert!(slept.events.iter().any(|event| matches!(
            event,
            Event::ToolConfirmResolved {
                accepted: false,
                ..
            }
        )));
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.outbox().is_empty());

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        runtime.invoke_tool("email_send", &email_body());
        let hibernated = runtime.handle(Command::Hibernate);
        assert!(hibernated.events.iter().any(|event| matches!(
            event,
            Event::ToolConfirmResolved {
                accepted: false,
                ..
            }
        )));
        assert!(runtime.hands.outbox().is_empty());
        assert!(matches!(
            runtime.invoke_tool("email_send", &email_body()).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Hibernate,
                    ..
                }
            }
        ));
    }

    #[test]
    fn cancel_does_not_append_and_a_name_mismatch_keeps_pending() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.invoke_tool("notify", &["hello".to_owned()]);

        let mismatch = runtime.confirm_tool("1", Some("echo"));
        assert!(matches!(
            mismatch.body,
            ResponseBody::Err {
                error: IpcError::PendingMismatch { .. }
            }
        ));
        assert!(runtime.hands.notifications().is_empty());
        assert!(runtime.hands.pending().is_some());

        let cancelled = runtime.cancel_tool("1", None);
        let status = cancelled.body.status().expect("cancelled");
        assert_eq!(status.message.as_deref(), Some("cancelled 1"));
        assert!(status.pending_tool.is_none());
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            cancelled.events.as_slice(),
            [Event::ToolConfirmResolved {
                accepted: false,
                pending_id,
            }] if pending_id == "1"
        ));

        let gone = runtime.cancel_tool("1", None);
        assert!(matches!(
            gone.body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { .. }
            }
        ));
    }

    #[test]
    fn shell_is_denied_and_unknown_is_rejected_while_awake() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        let denied = runtime.invoke_tool("shell", &["echo".to_owned()]);
        assert!(denied.events.is_empty());
        assert!(matches!(
            denied.body,
            ResponseBody::Err {
                error: IpcError::ToolDenied { ref name }
            } if name == "shell"
        ));
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.notifications().is_empty());
        runtime.confirm_tool("1", None);
        assert!(runtime.hands.notifications().is_empty());

        let unknown = runtime.invoke_tool("volume", &[]);
        assert!(matches!(
            unknown.body,
            ResponseBody::Err {
                error: IpcError::UnknownTool { ref name }
            } if name == "volume"
        ));
    }

    #[test]
    fn sleep_and_hibernate_clear_pending_and_still_refuse_tools() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.invoke_tool("notify", &["hello".to_owned()]);
        assert!(runtime.hands.notifications().is_empty());

        let slept = runtime.handle(Command::Sleep);
        assert!(matches!(
            slept.events.as_slice(),
            [
                Event::StateChanged {
                    state: VoiceState::Sleep,
                    ..
                },
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: false,
                },
            ] if pending_id == "1"
        ));
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            runtime.confirm_tool("1", None).body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { .. }
            }
        ));
        assert!(matches!(
            runtime.invoke_tool("notify", &["x".to_owned()]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));
        assert!(matches!(
            runtime.invoke_tool("shell", &[]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        runtime.invoke_tool("notify", &["later".to_owned()]);
        let hibernated = runtime.handle(Command::Hibernate);
        assert!(hibernated.events.iter().any(|event| matches!(
            event,
            Event::ToolConfirmResolved {
                accepted: false,
                ..
            }
        )));
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            runtime.invoke_tool("echo", &[]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Hibernate,
                    ..
                }
            }
        ));
    }

    #[test]
    fn mock_stt_emits_partial_and_final_while_awake() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        wake(&mut runtime);

        let outcome = runtime.inject_transcript("hello there");
        assert!(outcome.body.status().is_some());
        assert_eq!(
            outcome.events,
            vec![
                Event::PartialTranscript {
                    text: "hello there".to_owned(),
                },
                Event::FinalTranscript {
                    text: "hello there".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn mock_stt_refused_while_asleep() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        let outcome = runtime.inject_transcript("nope");
        assert!(matches!(
            outcome.body,
            ResponseBody::Err {
                error: IpcError::Protocol { .. }
            }
        ));
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn mock_tts_records_while_awake_and_refuses_asleep() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        assert!(matches!(
            runtime.speak("early").body,
            ResponseBody::Err {
                error: IpcError::Protocol { .. }
            }
        ));
        wake(&mut runtime);
        let said = runtime.speak("hello");
        assert!(said.body.status().is_some());
        assert_eq!(runtime.spoken(), &["hello".to_owned()]);
    }

    #[test]
    fn ask_rejects_blank_text_before_a_post() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        wake(&mut runtime);
        runtime.install_chat_fixture(crate::chat::xai_key_fixture(
            true,
            Some("sk-test-secret"),
            "pong",
        ));
        let outcome = runtime.ask("   ");
        match outcome.body {
            ResponseBody::Err {
                error: IpcError::ChatRejected { ref message },
            } => assert_eq!(message, "ask needs text"),
            other => panic!("expected chat_rejected, got {other:?}"),
        }
        assert!(outcome.events.is_empty());
        assert!(runtime.chat_posts().is_empty());
        assert!(runtime.session_turns().is_empty());
        assert!(!format!("{outcome:?}").contains("sk-test-secret"));
    }
}
