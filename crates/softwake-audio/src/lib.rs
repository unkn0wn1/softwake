//! Audio capture boundary.
//!
//! Callers depend on [`AudioCapture`]. [`MockAudioCapture`] is the in-memory
//! backend for tests and the daemon demo. [`PipeWireCapture`] is compiled with
//! the default `pipewire` feature: it implements the same trait and reports
//! that native I/O is not linked. That feature does not pull a `PipeWire` crate
//! or a system library.

mod mock;
#[cfg(feature = "pipewire")]
mod pipewire;
mod traits;

pub use mock::{AudioFrame, MockAudioCapture};
#[cfg(feature = "pipewire")]
pub use pipewire::{PipeWireCapture, PipeWireError};
pub use traits::AudioCapture;

#[cfg(all(test, feature = "pipewire"))]
mod tests {
    use super::{AudioCapture, MockAudioCapture, PipeWireCapture};

    fn start_then_stop<C: AudioCapture>(capture: &mut C) -> Result<(), C::Error> {
        capture.start()?;
        capture.stop()
    }

    #[test]
    fn mock_and_pipewire_stub_both_implement_the_trait() {
        let mut mock = MockAudioCapture::default();
        start_then_stop(&mut mock).expect("mock cannot fail");
        assert!(!mock.is_running());

        let mut pipewire = PipeWireCapture;
        assert_eq!(
            start_then_stop(&mut pipewire)
                .expect_err("stub fails on start")
                .to_string(),
            "PipeWire capture is not implemented"
        );
    }
}
