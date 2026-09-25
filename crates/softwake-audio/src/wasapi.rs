//! WASAPI capture backend (Windows).
//!
//! Softwake v1 ships a trait-shaped stub so the Windows target compiles and the
//! daemon capture enum can name `wasapi`. Real WASAPI I/O is deferred; use
//! `mock` until a native backend lands. See
//! [ADR 0019](../../../docs/ADR-0019-multiplatform-releases.md).

use crate::{AudioCapture, AudioFormat, AudioFrame};

/// WASAPI capture handle. It does not open a device in this build.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `WasapiCapture` is the public name of this backend.
pub struct WasapiCapture;

impl WasapiCapture {
    /// Stopped stub. Same constructor shape as other backends.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Always `false` in the stub build.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        false
    }
}

/// Failure from the WASAPI capture backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[allow(clippy::module_name_repetitions)] // `WasapiError` is the public error for this backend.
pub enum WasapiError {
    /// Stub: no native WASAPI stream is linked yet.
    #[error(
        "WASAPI capture is not implemented yet; use --capture mock (or omit) until a native backend ships"
    )]
    NotImplemented,

    /// `poll_frame` or `stop` ran before a successful `start`.
    #[error("WASAPI capture is not running")]
    NotRunning,
}

impl AudioCapture for WasapiCapture {
    type Error = WasapiError;

    fn start(&mut self) -> Result<(), Self::Error> {
        Err(WasapiError::NotImplemented)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        Err(WasapiError::NotRunning)
    }

    fn poll_frame(&mut self) -> Result<Option<AudioFrame>, Self::Error> {
        Err(WasapiError::NotRunning)
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::WAKE
    }
}

#[cfg(test)]
mod tests {
    use super::{WasapiCapture, WasapiError};
    use crate::{AudioCapture, AudioFormat};

    #[test]
    fn stub_does_not_open_a_device() {
        let mut capture = WasapiCapture::new();
        assert!(!capture.is_running());
        assert_eq!(capture.format(), AudioFormat::WAKE);
        assert_eq!(
            capture.start().expect_err("stub"),
            WasapiError::NotImplemented
        );
        assert_eq!(capture.stop().expect_err("stub"), WasapiError::NotRunning);
        assert_eq!(
            capture.poll_frame().expect_err("stub"),
            WasapiError::NotRunning
        );
    }
}
