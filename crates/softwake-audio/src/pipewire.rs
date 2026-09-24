//! Trait-shaped `PipeWire` backend.
//!
//! Native `PipeWire` I/O is not linked in this spike. [`PipeWireCapture`]
//! implements [`crate::AudioCapture`] and returns [`PipeWireError`] so a caller
//! cannot mistake the stub for an open microphone.

use crate::AudioCapture;

/// `PipeWire` capture handle. It does not open a device.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `PipeWireCapture` is the public name of this backend.
pub struct PipeWireCapture;

/// Native `PipeWire` capture is not built in this spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[allow(clippy::module_name_repetitions)] // `PipeWireError` is the public error for this backend.
#[error("PipeWire capture is not implemented")]
pub struct PipeWireError;

impl AudioCapture for PipeWireCapture {
    type Error = PipeWireError;

    fn start(&mut self) -> Result<(), Self::Error> {
        Err(PipeWireError)
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        Err(PipeWireError)
    }
}

#[cfg(test)]
mod tests {
    use super::{PipeWireCapture, PipeWireError};
    use crate::AudioCapture;

    #[test]
    fn stub_does_not_open_or_release_a_device() {
        let mut capture = PipeWireCapture;
        let started = capture.start().expect_err("start is a stub");
        assert_eq!(started, PipeWireError);
        assert_eq!(started.to_string(), "PipeWire capture is not implemented");

        let stopped = capture.stop().expect_err("stop is a stub");
        assert_eq!(stopped, PipeWireError);
    }
}
