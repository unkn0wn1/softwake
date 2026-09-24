//! Audio capture boundary.
//!
//! Backends, including `PipeWire`, are not implemented here. Callers depend on
//! [`AudioCapture`] so a later backend can plug in without changing who decides
//! when capture is allowed.

/// Start and stop microphone capture.
///
/// After [`AudioCapture::stop`] returns, the implementation must not deliver
/// further frames. Hibernate relies on that.
pub trait AudioCapture {
    /// Backend failure while opening or releasing the device.
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
}
