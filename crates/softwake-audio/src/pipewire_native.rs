//! Real `PipeWire` microphone capture.
//!
//! Opens the default input, requests 16 kHz mono `S16LE`, and queues
//! [`crate::AudioFrame`] windows for [`crate::AudioCapture::poll_frame`].
//! A dedicated thread runs the `PipeWire` main loop; stop sends a quit message
//! over [`pipewire::channel`].

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pipewire as pw;
use pw::spa::pod::Pod;
use pw::{properties::properties, spa};
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;

use crate::{AudioCapture, AudioFormat, AudioFrame};

/// Samples per queued window (10 ms at [`AudioFormat::WAKE`]).
const FRAME_SAMPLES: usize = 160;
/// Drop oldest frames when the daemon is not draining (about one second).
const MAX_QUEUED_FRAMES: usize = 100;

static PIPEWIRE_INIT: Once = Once::new();

/// Live `PipeWire` capture handle.
#[allow(clippy::module_name_repetitions)] // `PipeWireCapture` is the public name of this backend.
pub struct PipeWireCapture {
    running: bool,
    shared: Arc<Mutex<Shared>>,
    stop_tx: Option<pw::channel::Sender<Terminate>>,
    join: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PipeWireCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PipeWireCapture")
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct Shared {
    pending: VecDeque<AudioFrame>,
    remainder: Vec<i16>,
}

struct Terminate;

struct StreamData {
    shared: Arc<Mutex<Shared>>,
    format: spa::param::audio::AudioInfoRaw,
}

/// Failure from the `PipeWire` capture backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[allow(clippy::module_name_repetitions)] // `PipeWireError` is the public error for this backend.
pub enum PipeWireError {
    /// The `PipeWire` session could not be opened or the stream failed to connect.
    #[error("PipeWire capture failed: {0}")]
    Open(String),

    /// `poll_frame` or `stop` ran before a successful `start`.
    #[error("PipeWire capture is not running")]
    NotRunning,

    /// The capture worker thread panicked.
    #[error("PipeWire capture thread panicked")]
    Panicked,
}

impl Default for PipeWireCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl PipeWireCapture {
    /// Stopped capture that has not opened a device yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: false,
            shared: Arc::new(Mutex::new(Shared::default())),
            stop_tx: None,
            join: None,
        }
    }

    /// Whether `start` succeeded and `stop` has not.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running
    }

    fn clear_queue(shared: &Arc<Mutex<Shared>>) {
        if let Ok(mut guard) = shared.lock() {
            guard.pending.clear();
            guard.remainder.clear();
        }
    }

    fn push_samples(shared: &Arc<Mutex<Shared>>, samples: &[i16]) {
        let Ok(mut guard) = shared.lock() else {
            return;
        };
        guard.remainder.extend_from_slice(samples);
        while guard.remainder.len() >= FRAME_SAMPLES {
            let frame_samples: Vec<i16> = guard.remainder.drain(..FRAME_SAMPLES).collect();
            if guard.pending.len() >= MAX_QUEUED_FRAMES {
                guard.pending.pop_front();
            }
            guard
                .pending
                .push_back(AudioFrame::from_samples(frame_samples));
        }
    }

    // Owns the worker thread's PipeWire session; splitting it further obscures the stream setup.
    #[allow(
        clippy::too_many_lines,
        clippy::needless_pass_by_value,
        reason = "worker takes owned handles moved onto the capture thread"
    )]
    fn spawn_worker(
        shared: Arc<Mutex<Shared>>,
        ready_tx: mpsc::Sender<Result<(), String>>,
        stop_rx: pw::channel::Receiver<Terminate>,
    ) {
        PIPEWIRE_INIT.call_once(pw::init);

        let result = (|| -> Result<(), String> {
            let mainloop =
                pw::main_loop::MainLoopRc::new(None).map_err(|error| error.to_string())?;
            let context =
                pw::context::ContextRc::new(&mainloop, None).map_err(|error| error.to_string())?;
            let core = context
                .connect_rc(None)
                .map_err(|error| error.to_string())?;

            let props = properties! {
                *pw::keys::MEDIA_TYPE => "Audio",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Communication",
                *pw::keys::NODE_NAME => "softwake-capture",
                *pw::keys::NODE_DESCRIPTION => "Softwake wake capture",
            };
            let stream = pw::stream::StreamBox::new(&core, "softwake-capture", props)
                .map_err(|error| error.to_string())?;

            let data = StreamData {
                shared: Arc::clone(&shared),
                format: spa::param::audio::AudioInfoRaw::default(),
            };

            let _listener = stream
                .add_local_listener_with_user_data(data)
                .param_changed(|_, user_data, id, param| {
                    let Some(param) = param else {
                        return;
                    };
                    if id != pw::spa::param::ParamType::Format.as_raw() {
                        return;
                    }
                    let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                        return;
                    };
                    if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                        return;
                    }
                    let _ = user_data.format.parse(param);
                })
                .process(|stream, user_data| {
                    let Some(mut buffer) = stream.dequeue_buffer() else {
                        return;
                    };
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        return;
                    }
                    let data = &mut datas[0];
                    let chunk_size = data.chunk().size() as usize;
                    let channels = user_data.format.channels().max(1) as usize;
                    let rate = user_data.format.rate();
                    // Only accept the negotiated WAKE layout. PipeWire should
                    // convert when we request S16LE mono 16 kHz.
                    if rate != 0 && rate != AudioFormat::WAKE.sample_rate_hz() {
                        return;
                    }
                    let Some(bytes) = data.data() else {
                        return;
                    };
                    let byte_len = chunk_size.min(bytes.len());
                    if byte_len < 2 {
                        return;
                    }
                    let sample_bytes = &bytes[..byte_len - (byte_len % 2)];
                    let mut mono = Vec::with_capacity(sample_bytes.len() / 2 / channels.max(1));
                    let mut index = 0;
                    while index + channels * 2 <= sample_bytes.len() {
                        // Downmix: take channel 0 when more than one is present.
                        let sample =
                            i16::from_le_bytes([sample_bytes[index], sample_bytes[index + 1]]);
                        mono.push(sample);
                        index += channels * 2;
                    }
                    Self::push_samples(&user_data.shared, &mono);
                })
                .register()
                .map_err(|error| error.to_string())?;

            let mut audio_info = spa::param::audio::AudioInfoRaw::new();
            audio_info.set_format(spa::param::audio::AudioFormat::S16LE);
            audio_info.set_rate(AudioFormat::WAKE.sample_rate_hz());
            audio_info.set_channels(u32::from(AudioFormat::WAKE.channels()));
            let obj = pw::spa::pod::Object {
                type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                id: pw::spa::param::ParamType::EnumFormat.as_raw(),
                properties: audio_info.into(),
            };
            let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pw::spa::pod::Value::Object(obj),
            )
            .map_err(|error| format!("serialize format: {error}"))?
            .0
            .into_inner();
            let mut params =
                [Pod::from_bytes(&values)
                    .ok_or_else(|| "invalid PipeWire format pod".to_owned())?];

            stream
                .connect(
                    spa::utils::Direction::Input,
                    None,
                    pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                    &mut params,
                )
                .map_err(|error| error.to_string())?;

            let _attached = stop_rx.attach(mainloop.loop_(), {
                let mainloop = mainloop.clone();
                move |_| mainloop.quit()
            });

            let _ = ready_tx.send(Ok(()));
            mainloop.run();
            let _ = stream.disconnect();
            Ok(())
        })();

        if let Err(error) = result {
            let _ = ready_tx.send(Err(error));
        }
    }
}

impl AudioCapture for PipeWireCapture {
    type Error = PipeWireError;

    fn start(&mut self) -> Result<(), Self::Error> {
        if self.running {
            return Ok(());
        }
        Self::clear_queue(&self.shared);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = pw::channel::channel();
        let shared = Arc::clone(&self.shared);
        let join = thread::Builder::new()
            .name("softwake-pipewire".to_owned())
            .spawn(move || Self::spawn_worker(shared, ready_tx, stop_rx))
            .map_err(|error| PipeWireError::Open(format!("spawn: {error}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {
                self.stop_tx = Some(stop_tx);
                self.join = Some(join);
                self.running = true;
                Ok(())
            }
            Ok(Err(message)) => {
                let _ = join.join();
                Err(PipeWireError::Open(message))
            }
            Err(_) => {
                let _ = stop_tx.send(Terminate);
                let _ = join.join();
                Err(PipeWireError::Open(
                    "timed out waiting for PipeWire stream".to_owned(),
                ))
            }
        }
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        if !self.running {
            return Err(PipeWireError::NotRunning);
        }
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(Terminate);
        }
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                self.running = false;
                Self::clear_queue(&self.shared);
                return Err(PipeWireError::Panicked);
            }
        }
        self.running = false;
        Self::clear_queue(&self.shared);
        Ok(())
    }

    fn poll_frame(&mut self) -> Result<Option<AudioFrame>, Self::Error> {
        if !self.running {
            Self::clear_queue(&self.shared);
            return Ok(None);
        }
        let Ok(mut guard) = self.shared.lock() else {
            return Err(PipeWireError::Open(
                "capture queue lock poisoned".to_owned(),
            ));
        };
        Ok(guard.pending.pop_front())
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::WAKE
    }
}

impl Drop for PipeWireCapture {
    fn drop(&mut self) {
        if self.running {
            let _ = self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PipeWireCapture, PipeWireError};
    use crate::{AudioCapture, AudioFormat};

    #[test]
    fn starts_stopped_and_reports_wake_format() {
        let capture = PipeWireCapture::new();
        assert!(!capture.is_running());
        assert_eq!(capture.format(), AudioFormat::WAKE);
        assert_eq!(AudioFormat::WAKE.sample_rate_hz(), 16_000);
        assert_eq!(AudioFormat::WAKE.channels(), 1);
    }

    #[test]
    fn stop_before_start_is_not_running() {
        let mut capture = PipeWireCapture::new();
        assert_eq!(
            capture.stop().expect_err("not started"),
            PipeWireError::NotRunning
        );
        assert!(
            AudioCapture::poll_frame(&mut capture)
                .expect("poll while stopped")
                .is_none()
        );
    }
}
