//! Audio capture boundary.
//!
//! Callers depend on [`AudioCapture`]. [`MockAudioCapture`] is the in-memory
//! backend for tests and the daemon demo. [`PipeWireCapture`] is compiled with
//! the default `pipewire` feature: without `pipewire-native` it is a stub that
//! reports that native I/O is not linked. With `pipewire-native` it links
//! `libpipewire` and opens the default input at [`AudioFormat::WAKE`]. CI does
//! not enable `pipewire-native` and does not need a microphone.
//!
//! [`AudioFormat::WAKE`] is 16 kHz mono `i16`. [`AudioCapture::poll_frame`]
//! pulls one [`AudioFrame`] for the mock and for the native stream.
//! [`rms_level`] turns a frame into a `0.0..=1.0` capture level for status/HUD.

mod frame;
mod level;
mod mock;
#[cfg(feature = "pipewire")]
mod pipewire;
#[cfg(all(feature = "pipewire", feature = "pipewire-native"))]
mod pipewire_native;
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
    fn mock_implements_the_trait() {
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
    }

    #[cfg(not(feature = "pipewire-native"))]
    #[test]
    fn pipewire_stub_fails_on_start() {
        let mut pipewire = PipeWireCapture::new();
        assert_eq!(pipewire.format(), AudioFormat::WAKE);
        let error = start_then_stop(&mut pipewire).expect_err("stub fails on start");
        assert_eq!(
            error.to_string(),
            "PipeWire capture is not linked; enable the pipewire-native feature and install libpipewire-0.3-dev"
        );
        assert!(
            AudioCapture::poll_frame(&mut pipewire)
                .expect_err("stub has no queue")
                .to_string()
                .contains("not running")
        );
    }

    #[cfg(feature = "pipewire-native")]
    #[test]
    fn pipewire_native_reports_wake_format_when_stopped() {
        let mut pipewire = PipeWireCapture::new();
        assert_eq!(pipewire.format(), AudioFormat::WAKE);
        assert!(!pipewire.is_running());
        assert!(
            AudioCapture::poll_frame(&mut pipewire)
                .expect("stopped poll")
                .is_none()
        );
    }
}
