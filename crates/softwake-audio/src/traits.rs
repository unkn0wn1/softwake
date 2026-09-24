//! Capture trait.
//!
//! [`AudioCapture::poll_frame`] is how the daemon pulls PCM from either the
//! mock backend or a future native backend. [`AudioCapture::format`] names the
//! layout of those samples.

use crate::{AudioFormat, AudioFrame};

/// Start and stop microphone capture, and pull PCM windows.
///
/// After [`AudioCapture::stop`] returns, the implementation must not deliver
/// further frames. Hibernate relies on that. [`AudioCapture::poll_frame`]
/// returns `Ok(None)` when capture is running and no window is waiting, and
/// also after `stop` (the queue is dropped). A backend that cannot read the
/// device returns `Err` instead of pretending the microphone is idle.
pub trait AudioCapture {
    /// Backend failure while opening, reading, or releasing the device.
    type Error: std::error::Error;

    /// Begin capture and allow frames to flow.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the device cannot be opened.
    fn start(&mut self) -> Result<(), Self::Error>;

    /// Stop capture and release the device.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the device cannot be released.
    fn stop(&mut self) -> Result<(), Self::Error>;

    /// Next window of PCM, and only while capture is running.
    ///
    /// `Ok(None)` means the backend is running and the queue is empty, or
    /// capture is stopped and any queued samples were dropped. `Err` means
    /// the backend cannot produce frames (for example the `PipeWire` stub,
    /// which never opens a device).
    ///
    /// # Errors
    ///
    /// Returns the backend error when a frame cannot be read.
    fn poll_frame(&mut self) -> Result<Option<AudioFrame>, Self::Error>;

    /// Layout of [`AudioFrame::samples`] for this backend.
    ///
    /// The wake path expects [`AudioFormat::WAKE`] (16 kHz, mono, `i16`).
    fn format(&self) -> AudioFormat;
}
