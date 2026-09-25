//! Trait-shaped `PipeWire` backend.
//!
//! The default `pipewire` feature compiles this module and does not link
//! `libpipewire`. `start` fails with [`PipeWireError::NotLinked`], so a caller
//! cannot mistake the stub for an open microphone. The `pipewire-native`
//! feature selects the same module and fails with
//! [`PipeWireError::StreamUnwired`]: the frame type and [`crate::AudioCapture`]
//! methods are in place, and the device stream is not. CI enables neither a
//! microphone nor `pipewire-native`.
//!
//! A later native build fills `start` with a `PipeWire` stream whose process
//! callback queues [`crate::AudioFrame`] values at [`crate::AudioFormat::WAKE`].
//! [`crate::AudioCapture::poll_frame`] is already the pull API that callback
//! will feed. When those frames exist, the daemon scores them with
//! [`crate::rms_level`] the same way as mock PCM. Enabling the feature today
//! does not open a device and does not require a `PipeWire` daemon.

use crate::{AudioCapture, AudioFormat, AudioFrame};

/// `PipeWire` capture handle. It does not open a device.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `PipeWireCapture` is the public name of this backend.
pub struct PipeWireCapture;

/// Failure from the `PipeWire` capture backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[allow(clippy::module_name_repetitions)] // `PipeWireError` is the public error for this backend.
pub enum PipeWireError {
    /// Default build: the trait is compiled and `libpipewire` is not linked.
    ///
    /// Install `libpipewire-0.3-dev` and rebuild with `--features pipewire-native`
    /// when a device stream exists. That feature is off in CI.
    #[cfg(not(feature = "pipewire-native"))]
    #[error(
        "PipeWire capture is not linked; enable the pipewire-native feature and install libpipewire-0.3-dev"
    )]
    NotLinked,

    /// `pipewire-native` is on, and this build still has no device stream.
    ///
    /// `libpipewire` is intentionally not linked here, so CI stays mic-free.
    #[cfg(feature = "pipewire-native")]
    #[error("PipeWire native capture has no device stream in this build")]
    StreamUnwired,

    /// `poll_frame` or `stop` ran before a successful `start`.
    #[error("PipeWire capture is not running")]
    NotRunning,
}

impl PipeWireCapture {
    fn unavailable() -> PipeWireError {
        #[cfg(feature = "pipewire-native")]
        {
            PipeWireError::StreamUnwired
        }
        #[cfg(not(feature = "pipewire-native"))]
        {
            PipeWireError::NotLinked
        }
    }
}

impl AudioCapture for PipeWireCapture {
    type Error = PipeWireError;

    fn start(&mut self) -> Result<(), Self::Error> {
        Err(Self::unavailable())
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        // Nothing was opened, so there is no device to release.
        Err(PipeWireError::NotRunning)
    }

    fn poll_frame(&mut self) -> Result<Option<AudioFrame>, Self::Error> {
        // `Ok(None)` would look like an idle microphone. This stub has no queue.
        Err(PipeWireError::NotRunning)
    }

    fn format(&self) -> AudioFormat {
        // The stream, once it exists, must deliver this layout.
        AudioFormat::WAKE
    }
}

#[cfg(test)]
mod tests {
    use super::{PipeWireCapture, PipeWireError};
    use crate::{AudioCapture, AudioFormat};

    #[test]
    fn stub_does_not_open_or_release_a_device() {
        let mut capture = PipeWireCapture;
        assert_eq!(capture.format(), AudioFormat::WAKE);
        assert_eq!(AudioFormat::WAKE.sample_rate_hz(), 16_000);
        assert_eq!(AudioFormat::WAKE.channels(), 1);

        let started = capture.start().expect_err("start is a stub");
        #[cfg(not(feature = "pipewire-native"))]
        {
            assert_eq!(started, PipeWireError::NotLinked);
            assert_eq!(
                started.to_string(),
                "PipeWire capture is not linked; enable the pipewire-native feature and install libpipewire-0.3-dev"
            );
        }
        #[cfg(feature = "pipewire-native")]
        {
            assert_eq!(started, PipeWireError::StreamUnwired);
            assert_eq!(
                started.to_string(),
                "PipeWire native capture has no device stream in this build"
            );
        }

        let stopped = capture.stop().expect_err("stop is a stub");
        assert_eq!(stopped, PipeWireError::NotRunning);
        assert_eq!(stopped.to_string(), "PipeWire capture is not running");

        let polled = capture.poll_frame().expect_err("poll is a stub");
        assert_eq!(polled, PipeWireError::NotRunning);
    }
}
