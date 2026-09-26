//! JSON messages shared by the daemon and its clients.
//!
//! Voice states use the same spellings as the state machine: `sleep`,
//! `awake`, and `hibernate`. Commands and events keep the names the UI and
//! the daemon already share. Payloads carry the state, a short detail line,
//! or a structured error.

use serde::de::Error as DeserializeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Protocol generation negotiated when a client connects.
///
/// The client and the daemon must send this exact value. There is no
/// compatibility range inside one generation.
pub const PROTOCOL_VERSION: u32 = 1;

/// Fixed voice-state vocabulary on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceState {
    /// Listening. The acting session and tools are off.
    Sleep,
    /// Acting session and allowlisted tools may run.
    Awake,
    /// Capture is off. Only a UI command can leave.
    Hibernate,
}

impl VoiceState {
    /// Stable protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::Awake => "awake",
            Self::Hibernate => "hibernate",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "sleep" => Some(Self::Sleep),
            "awake" => Some(Self::Awake),
            "hibernate" => Some(Self::Hibernate),
            _ => None,
        }
    }
}

impl std::fmt::Display for VoiceState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for VoiceState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VoiceState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <&str>::deserialize(deserializer)?;
        Self::parse(value).ok_or_else(|| {
            DeserializeError::unknown_variant(value, &["sleep", "awake", "hibernate"])
        })
    }
}

/// Request from a client to the daemon.
///
/// `set_config` is intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Ask for the current voice state.
    GetStatus,
    /// Enter hibernate from sleep or awake.
    Hibernate,
    /// Leave hibernate.
    ///
    /// The daemon lands in sleep, not awake.
    WakeFromUi,
    /// Ask the daemon to sleep from awake.
    Sleep,
    /// Re-read the soul pack from disk.
    ///
    /// The new text applies on the next transition into awake. An awake
    /// session keeps the instructions it already applied.
    ReloadSoul,
}

impl Command {
    /// Stable protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GetStatus => "get_status",
            Self::Hibernate => "hibernate",
            Self::WakeFromUi => "wake_from_ui",
            Self::Sleep => "sleep",
            Self::ReloadSoul => "reload_soul",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "get_status" => Some(Self::GetStatus),
            "hibernate" => Some(Self::Hibernate),
            "wake_from_ui" => Some(Self::WakeFromUi),
            "sleep" => Some(Self::Sleep),
            "reload_soul" => Some(Self::ReloadSoul),
            _ => None,
        }
    }

    const VARIANTS: &'static [&'static str] = &[
        "get_status",
        "hibernate",
        "wake_from_ui",
        "sleep",
        "reload_soul",
    ];
}

impl std::fmt::Display for Command {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Command {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Command {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <&str>::deserialize(deserializer)?;
        Self::parse(value).ok_or_else(|| DeserializeError::unknown_variant(value, Self::VARIANTS))
    }
}

/// Failure a client can show.
///
/// Command failures travel in a [`ResponseBody::Err`]. [`Event::Error`] is
/// for a failure that is not the reply to one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IpcError {
    /// The command is not legal from the current voice state.
    #[error("cannot apply {command} from {from}: {reason}")]
    IllegalTransition {
        /// State the daemon was in.
        from: VoiceState,
        /// Command that was rejected.
        command: Command,
        /// Why that pair is rejected.
        reason: String,
    },

    /// A phrase command arrived inside its cooldown.
    ///
    /// UI commands are not cooled down. The variant is part of the protocol
    /// so a later phrase command can use the same error.
    #[error("cannot apply {command} during cooldown ({remaining_ms} ms remaining)")]
    Cooldown {
        /// Command that was rejected.
        command: Command,
        /// Time left before the command can apply, in milliseconds.
        remaining_ms: u64,
        /// Extra text when the daemon has it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },

    /// A socket or file operation failed.
    #[error("io: {message}")]
    Io {
        /// What failed, without a socket path from another machine.
        message: String,
    },

    /// The peer did not follow this protocol generation.
    #[error("protocol: {message}")]
    Protocol {
        /// What the receiver objected to.
        message: String,
    },

    /// A tool call arrived while the daemon was not awake.
    ///
    /// Produced only in reply to [`ClientMessage::ToolRequest`]. Sleep and
    /// hibernate refuse every name, including confirm-gated and denied tools.
    #[error("cannot run {name} while {state}")]
    ToolForbidden {
        /// Tool the client named.
        name: String,
        /// Voice state that refused the call.
        state: VoiceState,
    },

    /// The tool name is not registered.
    ///
    /// Produced only in reply to [`ClientMessage::ToolRequest`] while awake.
    #[error("unknown tool: {name}")]
    UnknownTool {
        /// Name that was rejected.
        name: String,
    },

    /// The tool is registered as deny and never runs.
    ///
    /// Produced only in reply to [`ClientMessage::ToolRequest`] while awake.
    #[error("tool denied: {name}")]
    ToolDenied {
        /// Name that was rejected.
        name: String,
    },

    /// A confirm-gated tool is already waiting.
    ///
    /// Produced only in reply to [`ClientMessage::ToolRequest`]. The existing
    /// pending confirmation is left in place.
    #[error("a confirmation is already pending: {pending_id}")]
    ConfirmationPending {
        /// Id of the confirmation that is already waiting.
        pending_id: String,
    },

    /// No pending confirmation has this id.
    ///
    /// Produced in reply to [`ClientMessage::ConfirmTool`] or
    /// [`ClientMessage::CancelTool`].
    #[error("unknown pending confirmation: {pending_id}")]
    UnknownPending {
        /// Id the client sent.
        pending_id: String,
    },

    /// The optional name did not match the pending tool.
    ///
    /// The pending confirmation is left in place.
    #[error("pending confirmation {pending_id} is not {name}")]
    PendingMismatch {
        /// Id the client sent.
        pending_id: String,
        /// Name the client expected.
        name: String,
    },

    /// Confirm was refused because the daemon is not awake.
    ///
    /// The pending confirmation is left in place so a cancel can still clear it.
    /// Sleep and hibernate clear a pending confirmation themselves; a confirm
    /// after that is [`Self::UnknownPending`].
    #[error("cannot confirm {pending_id} while {state}")]
    ConfirmForbidden {
        /// Id the client sent.
        pending_id: String,
        /// Voice state that refused the confirm.
        state: VoiceState,
    },

    /// One chat turn was refused.
    ///
    /// Produced only in reply to [`ClientMessage::Ask`]. `message` is the
    /// operator sentence from the demo ask path. It does not include a bearer.
    #[error("{message}")]
    ChatRejected {
        /// Operator-facing sentence, with no `rejected:` prefix.
        message: String,
    },

    /// Press-to-talk was refused.
    ///
    /// Produced only in reply to [`ClientMessage::TalkStart`] or
    /// [`ClientMessage::TalkStop`]. `message` is an operator sentence. It does
    /// not include a bearer or a transcript body from the provider.
    #[error("{message}")]
    TalkRejected {
        /// Operator-facing sentence.
        message: String,
    },
}

impl IpcError {
    /// Protocol failure with a display message.
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol {
            message: message.into(),
        }
    }

    /// I/O failure with a display message.
    #[must_use]
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
        }
    }
}

/// Last soul-pack read.
///
/// Older peers omit this object. `ok` is false when any of `soul.md`,
/// `user.md`, `rules.md`, or `glossary.md` is missing, fails validation,
/// or the glossary map does not parse. `reason` is a short sentence in
/// that case. The fields are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulReport {
    /// The last read passed validation.
    pub ok: bool,
    /// Why `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A confirm-gated tool that has not run yet.
///
/// Older peers omit this object. The tool does not run until `confirm_tool`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingTool {
    /// Id to send with `confirm_tool` or `cancel_tool`.
    pub pending_id: String,
    /// Registered tool name.
    pub name: String,
    /// Arguments that will be passed if the operator confirms.
    #[serde(default)]
    pub args: Vec<String>,
    /// Short operator-facing description from the registry.
    pub description: String,
}

/// Voice state returned by a successful command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "additive wire flags (talking, auto_listening) stay independent bools on protocol 1"
)]
pub struct Status {
    /// Current voice state.
    pub state: VoiceState,
    /// Whether capture is running after the command.
    pub capture_running: bool,
    /// Peak-normalized RMS of the latest capture window, when PCM was scored.
    ///
    /// Absent when capture is stopped or no frame has been drained yet. Additive
    /// on protocol generation 1; older peers omit it and newer peers may skip it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_level: Option<f32>,
    /// A `reload_soul` has not yet been applied on an awake entry.
    ///
    /// Cleared only after a valid pack is applied while entering awake. A
    /// missing or invalid pack leaves the flag set.
    pub soul_reload_pending: bool,
    /// Last soul-pack read. Absent when the peer predates this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soul: Option<SoulReport>,
    /// Operator-facing sentence, when this reply has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Short transition note, when one is cheap to include.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Confirm-gated tool waiting for `confirm_tool`. Absent when nothing is waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_tool: Option<PendingTool>,
    /// One-line summary of the latest tool-log entry, such as `echo safe ran`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    /// True while press-to-talk is holding a PCM buffer.
    ///
    /// Additive on protocol generation 1. Older peers omit it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub talking: bool,
    /// True while awake free-speech energy gating is armed (not PTT).
    ///
    /// Additive on protocol generation 1. Older peers omit it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_listening: bool,
    /// Estimated context tokens in use for the awake session (char/4 v1).
    ///
    /// Additive on protocol generation 1. Absent when asleep or never asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u32>,
    /// Resolved context limit tokens for the selected model / Settings override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u32>,
    /// True when the latest ask compacted older turns.
    #[serde(default, skip_serializing_if = "is_false")]
    pub context_compacted: bool,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires fn(&T) -> bool"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

/// Successful status or a structured error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseBody {
    /// The command was applied, or status was read.
    Ok {
        /// Fields of [`Status`], inline so a client sees one object.
        ///
        /// The field name is not `status`: that would collide with the serde tag.
        #[serde(flatten)]
        snapshot: Status,
    },
    /// The command was rejected. The daemon's state is unchanged.
    Err {
        /// Why the command did not apply.
        error: IpcError,
    },
}

impl ResponseBody {
    /// Wrap a status snapshot.
    #[must_use]
    pub fn ok(snapshot: Status) -> Self {
        Self::Ok { snapshot }
    }

    /// Status when the command succeeded.
    #[must_use]
    pub fn status(&self) -> Option<&Status> {
        match self {
            Self::Ok { snapshot } => Some(snapshot),
            Self::Err { .. } => None,
        }
    }
}

/// Notification from the daemon to connected clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The voice state changed.
    StateChanged {
        /// State after the transition.
        state: VoiceState,
        /// State before the transition.
        previous: VoiceState,
        /// Capture flag after the transition's effects.
        capture_running: bool,
        /// Short note such as `sleep -> hibernate`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// Partial transcript. Awake only; do not emit this while asleep or hibernating.
    PartialTranscript {
        /// Transcript text received so far.
        text: String,
    },
    /// Final transcript for one utterance. Awake only; additive on protocol 1.
    FinalTranscript {
        /// Completed transcript text.
        text: String,
    },
    /// A tool started. Safe tools emit this immediately. Confirm-gated tools
    /// emit it only after `confirm_tool`.
    ToolStarted {
        /// Registered tool name.
        name: String,
    },
    /// A tool finished.
    ToolFinished {
        /// Registered tool name.
        name: String,
        /// Optional result summary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// A confirm-gated tool is waiting. It has not run.
    ToolConfirmPending {
        /// Id to send with `confirm_tool` or `cancel_tool`.
        pending_id: String,
        /// Registered tool name.
        name: String,
        /// Arguments that will be passed if the operator confirms.
        #[serde(default)]
        args: Vec<String>,
        /// Short operator-facing description from the registry.
        description: String,
    },
    /// A pending confirmation was confirmed or cleared.
    ///
    /// `accepted` is true only when the tool then runs. Cancel, and a sleep
    /// or hibernate that drops the pending record, use false.
    ToolConfirmResolved {
        /// Id that was resolved.
        pending_id: String,
        /// Whether the tool was accepted and run.
        accepted: bool,
    },
    /// A failure that is not the reply to a specific request.
    Error {
        /// What failed.
        error: IpcError,
    },
}

/// First and later messages from a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Must be the first message on a connection.
    Hello {
        /// [`PROTOCOL_VERSION`] the client speaks.
        protocol_version: u32,
    },
    /// One command. `id` is copied onto the response.
    Request {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
        /// Command to apply.
        command: Command,
    },
    /// Run one tool, or stage it when the registry marks it confirm-gated.
    /// `id` is copied onto the response.
    ///
    /// The daemon runs a safe tool only while awake. A confirm-gated tool does
    /// not run; the response carries [`Status::pending_tool`] and the daemon
    /// emits [`Event::ToolConfirmPending`]. `args` may be omitted; it is then
    /// an empty list. This variant is additive: a client that never sends it
    /// still speaks protocol generation 1.
    ToolRequest {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
        /// Tool name.
        name: String,
        /// Arguments passed through to the tool. They are not interpreted.
        #[serde(default)]
        args: Vec<String>,
    },
    /// Run the pending confirm-gated tool once, then clear it.
    ///
    /// Refused when the daemon is not awake. `name`, when present, must match
    /// the pending tool. Additive on protocol generation 1.
    ConfirmTool {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
        /// Id from [`Event::ToolConfirmPending`] or [`Status::pending_tool`].
        pending_id: String,
        /// When set, the pending tool name must match.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Clear the pending confirmation without running the tool.
    ///
    /// Allowed in any voice state when the id matches. `name`, when present,
    /// must match the pending tool. Additive on protocol generation 1.
    CancelTool {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
        /// Id from [`Event::ToolConfirmPending`] or [`Status::pending_tool`].
        pending_id: String,
        /// When set, the pending tool name must match.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Send one typed line to the selected provider while the daemon is awake.
    ///
    /// `ctl chat` sends this same message. Blank text is rejected. Sleep and
    /// hibernate refuse the turn before Settings are read. Success puts the
    /// assistant text on [`Status::message`]. A refusal is
    /// [`IpcError::ChatRejected`]. No event is broadcast. Additive on protocol
    /// generation 1: a client that never sends `ask` still speaks this generation.
    Ask {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
        /// User line.
        text: String,
    },
    /// Enter awake from sleep when the loaded four-file pack is valid.
    ///
    /// Additive on protocol generation 1. The daemon runs the same wake-phrase
    /// path as the typed demo. `wake_from_ui` is a different message and lands
    /// in sleep. A client that never sends `wake` still speaks this generation.
    Wake {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
    },
    /// Arm press-to-talk and start buffering PCM.
    ///
    /// Awake only. From sleep the daemon enters awake first, the same gate as
    /// HUD ask. Hibernate refuses. Additive on protocol generation 1.
    TalkStart {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
    },
    /// Stop buffering, transcribe, ask, and speak the reply when TTS is configured.
    ///
    /// Additive on protocol generation 1. A client that never sends `talk_stop`
    /// still speaks this generation.
    TalkStop {
        /// Client-chosen id. The daemon echoes it and does not interpret it.
        id: u64,
    },
}

/// Daemon messages after a client connects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The versions matched. Further messages may be requests.
    HelloOk {
        /// [`PROTOCOL_VERSION`] the daemon speaks.
        protocol_version: u32,
    },
    /// The versions differed, or the first message was not a hello.
    ///
    /// The daemon closes the connection after this message.
    HelloRejected {
        /// Version the client sent, when the first message was a hello.
        protocol_version: u32,
        /// Version the daemon speaks.
        expected: u32,
        /// Why the connection will close.
        message: String,
    },
    /// Reply to one client request, tool call, ask, or wake.
    Response {
        /// Id copied from the request.
        id: u64,
        /// Status or error.
        body: ResponseBody,
    },
    /// Broadcast while the client is connected.
    Event {
        /// What happened.
        body: Event,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        ClientMessage, Command, Event, IpcError, PROTOCOL_VERSION, PendingTool, ResponseBody,
        ServerMessage, SoulReport, Status, VoiceState,
    };

    fn assert_round_trip<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("encode");
        assert!(
            !json.contains('\n'),
            "one message must stay on one line: {json}"
        );
        let decoded: T = serde_json::from_str(&json).expect("decode");
        assert_eq!(&decoded, value);
    }

    fn status() -> Status {
        Status {
            state: VoiceState::Sleep,
            capture_running: true,
            capture_level: None,
            soul_reload_pending: false,
            soul: Some(SoulReport {
                ok: true,
                reason: None,
            }),
            message: None,
            detail: Some("sleep -> hibernate".to_owned()),
            pending_tool: None,
            last_tool: None,
            talking: false,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        }
    }

    #[test]
    fn commands_round_trip_as_snake_case_strings() {
        for command in [
            Command::GetStatus,
            Command::Hibernate,
            Command::WakeFromUi,
            Command::Sleep,
            Command::ReloadSoul,
        ] {
            let message = ClientMessage::Request { id: 7, command };
            assert_round_trip(&message);
            let json = serde_json::to_string(&message).expect("encode");
            assert!(json.contains(&format!("\"command\":\"{}\"", command.as_str())));
        }
    }

    #[test]
    fn responses_and_state_changes_round_trip() {
        let ok = ServerMessage::Response {
            id: 3,
            body: ResponseBody::ok(status()),
        };
        assert_round_trip(&ok);
        assert_eq!(ok_status(&ok).expect("status").state, VoiceState::Sleep);

        let rejected = ServerMessage::Response {
            id: 4,
            body: ResponseBody::Err {
                error: IpcError::IllegalTransition {
                    from: VoiceState::Sleep,
                    command: Command::Sleep,
                    reason: "already asleep".to_owned(),
                },
            },
        };
        assert_round_trip(&rejected);
        assert!(rejected_error(&rejected).is_some());

        let changed = ServerMessage::Event {
            body: Event::StateChanged {
                state: VoiceState::Hibernate,
                previous: VoiceState::Awake,
                capture_running: false,
                detail: Some("awake -> hibernate".to_owned()),
            },
        };
        assert_round_trip(&changed);

        let cooldown = IpcError::Cooldown {
            command: Command::WakeFromUi,
            remaining_ms: 800,
            detail: Some("phrase gate".to_owned()),
        };
        assert_round_trip(&cooldown);
        assert_eq!(
            cooldown.to_string(),
            "cannot apply wake_from_ui during cooldown (800 ms remaining)"
        );
    }

    #[test]
    fn other_events_and_hello_round_trip() {
        assert_round_trip(&ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        });
        assert_round_trip(&ServerMessage::HelloOk {
            protocol_version: PROTOCOL_VERSION,
        });
        assert_round_trip(&ServerMessage::HelloRejected {
            protocol_version: 0,
            expected: PROTOCOL_VERSION,
            message: "protocol version 0 is not supported".to_owned(),
        });
        assert_round_trip(&Event::PartialTranscript {
            text: "hello\nsoftwake".to_owned(),
        });
        assert_round_trip(&Event::FinalTranscript {
            text: "hello world".to_owned(),
        });
        assert_round_trip(&Event::ToolStarted {
            name: "volume".to_owned(),
        });
        assert_round_trip(&Event::ToolFinished {
            name: "volume".to_owned(),
            detail: None,
        });
        assert_round_trip(&Event::Error {
            error: IpcError::io("read failed"),
        });
        assert_round_trip(&IpcError::protocol("expected hello"));
        assert_round_trip(&IpcError::ToolForbidden {
            name: "echo".to_owned(),
            state: VoiceState::Sleep,
        });
        assert_round_trip(&IpcError::UnknownTool {
            name: "volume".to_owned(),
        });
        assert_eq!(
            IpcError::ToolForbidden {
                name: "echo".to_owned(),
                state: VoiceState::Hibernate,
            }
            .to_string(),
            "cannot run echo while hibernate"
        );
        assert_eq!(
            IpcError::UnknownTool {
                name: "volume".to_owned(),
            }
            .to_string(),
            "unknown tool: volume"
        );
        assert_round_trip(&IpcError::ToolDenied {
            name: "shell".to_owned(),
        });
        assert_round_trip(&IpcError::ConfirmationPending {
            pending_id: "1".to_owned(),
        });
        assert_round_trip(&IpcError::UnknownPending {
            pending_id: "4".to_owned(),
        });
        assert_round_trip(&IpcError::PendingMismatch {
            pending_id: "1".to_owned(),
            name: "echo".to_owned(),
        });
        assert_round_trip(&IpcError::ConfirmForbidden {
            pending_id: "1".to_owned(),
            state: VoiceState::Sleep,
        });
        assert_eq!(
            IpcError::ToolDenied {
                name: "shell".to_owned(),
            }
            .to_string(),
            "tool denied: shell"
        );
        assert_eq!(
            IpcError::ConfirmForbidden {
                pending_id: "1".to_owned(),
                state: VoiceState::Hibernate,
            }
            .to_string(),
            "cannot confirm 1 while hibernate"
        );
    }

    #[test]
    fn tool_request_round_trips_and_defaults_omitted_args() {
        let with_args = ClientMessage::ToolRequest {
            id: 9,
            name: "echo".to_owned(),
            args: vec!["hello".to_owned(), "world".to_owned()],
        };
        assert_round_trip(&with_args);
        let json = serde_json::to_string(&with_args).expect("encode");
        assert!(json.contains("\"type\":\"tool_request\""));

        let omitted: ClientMessage =
            serde_json::from_str(r#"{"type":"tool_request","id":4,"name":"echo"}"#)
                .expect("omitted args");
        assert_eq!(
            omitted,
            ClientMessage::ToolRequest {
                id: 4,
                name: "echo".to_owned(),
                args: Vec::new(),
            }
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one table covers confirm/cancel wire shapes"
    )]
    fn confirm_and_cancel_round_trip_and_default_an_omitted_name() {
        let confirm = ClientMessage::ConfirmTool {
            id: 3,
            pending_id: "1".to_owned(),
            name: Some("notify".to_owned()),
        };
        assert_round_trip(&confirm);
        let json = serde_json::to_string(&confirm).expect("encode");
        assert!(json.contains("\"type\":\"confirm_tool\""));
        assert!(json.contains("\"pending_id\":\"1\""));

        let omitted: ClientMessage =
            serde_json::from_str(r#"{"type":"confirm_tool","id":3,"pending_id":"1"}"#)
                .expect("omitted name");
        assert_eq!(
            omitted,
            ClientMessage::ConfirmTool {
                id: 3,
                pending_id: "1".to_owned(),
                name: None,
            }
        );

        let cancel = ClientMessage::CancelTool {
            id: 8,
            pending_id: "1".to_owned(),
            name: None,
        };
        assert_round_trip(&cancel);
        let cancel_json = serde_json::to_string(&cancel).expect("encode");
        assert!(cancel_json.contains("\"type\":\"cancel_tool\""));
        assert!(!cancel_json.contains("\"name\""));

        let ask = ClientMessage::Ask {
            id: 11,
            text: "hello there".to_owned(),
        };
        assert_round_trip(&ask);
        let ask_json = serde_json::to_string(&ask).expect("encode");
        assert!(ask_json.contains("\"type\":\"ask\""));
        assert!(ask_json.contains("\"text\":\"hello there\""));

        let talk_start = ClientMessage::TalkStart { id: 12 };
        assert_round_trip(&talk_start);
        let talk_start_json = serde_json::to_string(&talk_start).expect("encode");
        assert!(talk_start_json.contains("\"type\":\"talk_start\""));
        let talk_stop = ClientMessage::TalkStop { id: 13 };
        assert_round_trip(&talk_stop);
        let talk_stop_json = serde_json::to_string(&talk_stop).expect("encode");
        assert!(talk_stop_json.contains("\"type\":\"talk_stop\""));
        let talk_rejected = IpcError::TalkRejected {
            message: "hold the mic a little longer".to_owned(),
        };
        assert_round_trip(&talk_rejected);
        assert_eq!(talk_rejected.to_string(), "hold the mic a little longer");
        let talk_rejected_json = serde_json::to_string(&talk_rejected).expect("encode");
        assert!(talk_rejected_json.contains("\"kind\":\"talk_rejected\""));

        let wake = ClientMessage::Wake { id: 1 };
        assert_round_trip(&wake);
        let wake_json = serde_json::to_string(&wake).expect("encode");
        assert!(
            !wake_json.contains('\n'),
            "one message must stay on one line"
        );
        assert!(wake_json.contains("\"type\":\"wake\""));
        assert!(wake_json.contains("\"id\":1"));

        let rejected = IpcError::ChatRejected {
            message: "ask while sleep (chat acts only while awake)".to_owned(),
        };
        assert_round_trip(&rejected);
        assert_eq!(
            rejected.to_string(),
            "ask while sleep (chat acts only while awake)"
        );
        let rejected_json = serde_json::to_string(&rejected).expect("encode");
        assert!(rejected_json.contains("\"kind\":\"chat_rejected\""));

        let pending = Event::ToolConfirmPending {
            pending_id: "1".to_owned(),
            name: "notify".to_owned(),
            args: vec!["hello".to_owned()],
            description: "Append a notification to the in-memory sink.".to_owned(),
        };
        assert_round_trip(&pending);
        assert_round_trip(&Event::ToolConfirmResolved {
            pending_id: "1".to_owned(),
            accepted: false,
        });

        let with_pending = Status {
            state: VoiceState::Awake,
            capture_running: true,
            capture_level: None,
            soul_reload_pending: false,
            soul: None,
            message: Some("pending confirmation 1 for notify".to_owned()),
            detail: None,
            pending_tool: Some(PendingTool {
                pending_id: "1".to_owned(),
                name: "notify".to_owned(),
                args: vec!["hello".to_owned()],
                description: "Append a notification to the in-memory sink.".to_owned(),
            }),
            last_tool: Some("notify confirm pending".to_owned()),
            talking: true,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        };
        assert_round_trip(&with_pending);
        let pending_json = serde_json::to_string(&with_pending).expect("encode");
        assert!(pending_json.contains("\"pending_tool\""));
        assert!(pending_json.contains("\"last_tool\":\"notify confirm pending\""));
        assert!(pending_json.contains("\"talking\":true"));
    }

    #[test]
    fn unknown_spellings_are_rejected() {
        let error = serde_json::from_str::<Command>("\"GetStatus\"").expect_err("capital");
        assert!(error.to_string().contains("unknown variant"));
        assert!(serde_json::from_str::<VoiceState>("\"Sleep\"").is_err());
        assert!(serde_json::from_str::<Command>("\"set_config\"").is_err());
    }

    #[test]
    fn omitted_optional_fields_decode_as_absent() {
        let status: Status = serde_json::from_str(
            r#"{"state":"awake","capture_running":true,"soul_reload_pending":true}"#,
        )
        .expect("status");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(status.message.is_none());
        assert!(status.detail.is_none());
        assert!(status.soul.is_none());
        assert!(status.pending_tool.is_none());
        assert!(status.last_tool.is_none());
        assert!(status.soul_reload_pending);
        assert!(status.capture_level.is_none());
        assert!(!status.talking);
    }

    #[test]
    fn capture_level_round_trips_and_omits_when_absent() {
        let with_level = Status {
            state: VoiceState::Sleep,
            capture_running: true,
            capture_level: Some(0.42),
            soul_reload_pending: false,
            soul: None,
            message: None,
            detail: None,
            pending_tool: None,
            last_tool: None,
            talking: false,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        };
        assert_round_trip(&with_level);
        let json = serde_json::to_string(&with_level).expect("encode");
        assert!(json.contains("capture_level"));

        let without = Status {
            state: VoiceState::Sleep,
            capture_running: true,
            capture_level: None,
            soul_reload_pending: false,
            soul: None,
            message: None,
            detail: None,
            pending_tool: None,
            last_tool: None,
            talking: false,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        };
        let json = serde_json::to_string(&without).expect("encode");
        assert!(!json.contains("capture_level"));
        assert!(!json.contains("talking"));
        assert_round_trip(&without);
    }

    #[test]
    fn soul_report_round_trips_and_omits_an_empty_reason() {
        let missing = Status {
            state: VoiceState::Sleep,
            capture_running: true,
            capture_level: None,
            soul_reload_pending: true,
            soul: Some(SoulReport {
                ok: false,
                reason: Some("missing soul.md".to_owned()),
            }),
            message: None,
            detail: None,
            pending_tool: None,
            last_tool: None,
            talking: false,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        };
        assert_round_trip(&missing);

        let ok = Status {
            state: VoiceState::Sleep,
            capture_running: true,
            capture_level: None,
            soul_reload_pending: false,
            soul: Some(SoulReport {
                ok: true,
                reason: None,
            }),
            message: None,
            detail: None,
            pending_tool: None,
            last_tool: None,
            talking: false,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        };
        let json = serde_json::to_string(&ok).expect("encode");
        assert!(json.contains("\"soul\":{\"ok\":true}"));
        assert!(!json.contains("reason"));
        assert_round_trip(&ok);
    }

    fn ok_status(message: &ServerMessage) -> Option<&Status> {
        match message {
            ServerMessage::Response { body, .. } => body.status(),
            _ => None,
        }
    }

    fn rejected_error(message: &ServerMessage) -> Option<&IpcError> {
        match message {
            ServerMessage::Response {
                body: ResponseBody::Err { error },
                ..
            } => Some(error),
            _ => None,
        }
    }
}
