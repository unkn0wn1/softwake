//! `PipeWire` capture backend.
//!
//! With the default `pipewire` feature this module is a trait-shaped stub that
//! does not link `libpipewire`. With `pipewire-native` it re-exports the real
//! capture implementation that opens the default input.

#[cfg(feature = "pipewire-native")]
mod native {
    pub use crate::pipewire_native::{PipeWireCapture, PipeWireError};
}

#[cfg(not(feature = "pipewire-native"))]
mod stub {
    use crate::{AudioCapture, AudioFormat, AudioFrame};

    /// `PipeWire` capture handle. It does not open a device in this build.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    #[allow(clippy::module_name_repetitions)] // `PipeWireCapture` is the public name of this backend.
    pub struct PipeWireCapture;

    impl PipeWireCapture {
        /// Stopped stub. Same constructor shape as the native backend.
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

    /// Failure from the `PipeWire` capture backend.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    #[allow(clippy::module_name_repetitions)] // `PipeWireError` is the public error for this backend.
    pub enum PipeWireError {
        /// Default build: the trait is compiled and `libpipewire` is not linked.
        ///
        /// Install `libpipewire-0.3-dev` and rebuild with `--features pipewire-native`
        /// (daemon: `pipewire-capture`) when a device stream is needed. That
        /// feature is off in CI.
        #[error(
            "PipeWire capture is not linked; enable the pipewire-native feature and install libpipewire-0.3-dev"
        )]
        NotLinked,

        /// `poll_frame` or `stop` ran before a successful `start`.
        #[error("PipeWire capture is not running")]
        NotRunning,
    }

    impl AudioCapture for PipeWireCapture {
        type Error = PipeWireError;

        fn start(&mut self) -> Result<(), Self::Error> {
            Err(PipeWireError::NotLinked)
        }

        fn stop(&mut self) -> Result<(), Self::Error> {
            Err(PipeWireError::NotRunning)
        }

        fn poll_frame(&mut self) -> Result<Option<AudioFrame>, Self::Error> {
            Err(PipeWireError::NotRunning)
        }

        fn format(&self) -> AudioFormat {
            AudioFormat::WAKE
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{PipeWireCapture, PipeWireError};
        use crate::{AudioCapture, AudioFormat};

        #[test]
        fn stub_does_not_open_or_release_a_device() {
            let mut capture = PipeWireCapture::new();
            assert!(!capture.is_running());
            assert_eq!(capture.format(), AudioFormat::WAKE);
            assert_eq!(AudioFormat::WAKE.sample_rate_hz(), 16_000);
            assert_eq!(AudioFormat::WAKE.channels(), 1);

            let started = capture.start().expect_err("start is a stub");
            assert_eq!(started, PipeWireError::NotLinked);
            assert_eq!(
                started.to_string(),
                "PipeWire capture is not linked; enable the pipewire-native feature and install libpipewire-0.3-dev"
            );

            let stopped = capture.stop().expect_err("stop is a stub");
            assert_eq!(stopped, PipeWireError::NotRunning);
            assert_eq!(stopped.to_string(), "PipeWire capture is not running");

            let polled = capture.poll_frame().expect_err("poll is a stub");
            assert_eq!(polled, PipeWireError::NotRunning);
        }
    }
}

#[cfg(feature = "pipewire-native")]
pub use native::{PipeWireCapture, PipeWireError};
#[cfg(not(feature = "pipewire-native"))]
pub use stub::{PipeWireCapture, PipeWireError};
