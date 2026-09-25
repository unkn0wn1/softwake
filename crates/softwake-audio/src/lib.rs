//! Audio capture boundary.
//!
//! Callers depend on [`AudioCapture`]. [`MockAudioCapture`] is the in-memory
//! backend for tests and the daemon demo. [`PipeWireCapture`] is compiled with
//! the default `pipewire` feature: it implements the same trait and reports
//! that native I/O is not linked. That feature does not pull a `PipeWire` crate
//! or a system library. The `pipewire-native` feature selects the same stub
//! with a distinct error and is off unless a developer opts in. CI does not
//! enable it and does not need a microphone.
//!
//! [`AudioFormat::WAKE`] is 16 kHz mono `i16`. [`AudioCapture::poll_frame`]
//! pulls one [`AudioFrame`] for the mock and for a future native stream.
//! [`rms_level`] turns a frame into a `0.0..=1.0` capture level for status/HUD.

mod frame;
mod level;
mod mock;
#[cfg(feature = "pipewire")]
mod pipewire;
mod traits;

pub use frame::{AudioFormat, AudioFrame};
pub use level::rms_level;
pub use mock::MockAudioCapture;
#[cfg(feature = "pipewire")]
pub use pipewire::{PipeWireCapture, PipeWireError};
pub use traits::AudioCapture;

#[cfg(all(test, feature = "pipewire"))]
mod tests {
    use super::{AudioCapture, AudioFormat, AudioFrame, MockAudioCapture, PipeWireCapture};

    fn start_then_stop<C: AudioCapture>(capture: &mut C) -> Result<(), C::Error> {
        capture.start()?;
        capture.stop()
    }

    #[test]
    fn mock_and_pipewire_stub_both_implement_the_trait() {
        let mut mock = MockAudioCapture::default();
        assert_eq!(mock.format(), AudioFormat::WAKE);
        start_then_stop(&mut mock).expect("mock cannot fail");
        assert!(!mock.is_running());
        assert!(AudioCapture::poll_frame(&mut mock).expect("poll").is_none());

        mock.start().expect("restart");
        assert!(mock.push_frame(&[0, 0]));
        let frame: AudioFrame = AudioCapture::poll_frame(&mut mock)
            .expect("poll")
            .expect("frame");
        assert_eq!(frame.samples(), &[0, 0]);

        let mut pipewire = PipeWireCapture;
        assert_eq!(pipewire.format(), AudioFormat::WAKE);
        let error = start_then_stop(&mut pipewire).expect_err("stub fails on start");
        #[cfg(not(feature = "pipewire-native"))]
        assert_eq!(
            error.to_string(),
            "PipeWire capture is not linked; enable the pipewire-native feature and install libpipewire-0.3-dev"
        );
        #[cfg(feature = "pipewire-native")]
        assert_eq!(
            error.to_string(),
            "PipeWire native capture has no device stream in this build"
        );
        assert!(
            AudioCapture::poll_frame(&mut pipewire)
                .expect_err("stub has no queue")
                .to_string()
                .contains("not running")
        );
    }
}
