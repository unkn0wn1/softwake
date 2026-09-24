//! PCM windows shared by every capture backend.
//!
//! [`AudioFormat::WAKE`] is 16 kHz mono `i16`. That is the layout the PCM wake
//! detector scores. This crate does not depend on the wake crate; the contract
//! is the sample rate, the channel count, and [`AudioFrame::samples`].

/// Layout of the samples inside an [`AudioFrame`].
///
/// Backends that capture another rate or channel count resample before a frame
/// is queued. Callers of [`crate::AudioCapture::poll_frame`] can treat every
/// frame as this format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    sample_rate_hz: u32,
    channels: u16,
}

impl AudioFormat {
    /// 16 kHz, one channel. The on-device wake engine consumes this layout.
    pub const WAKE: Self = Self {
        sample_rate_hz: 16_000,
        channels: 1,
    };

    /// Samples per second.
    #[must_use]
    pub const fn sample_rate_hz(self) -> u32 {
        self.sample_rate_hz
    }

    /// Interleaved channel count. `1` is mono.
    #[must_use]
    pub const fn channels(self) -> u16 {
        self.channels
    }
}

/// One captured window of interleaved 16-bit samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame {
    samples: Vec<i16>,
}

impl AudioFrame {
    pub(crate) fn from_samples(samples: Vec<i16>) -> Self {
        Self { samples }
    }

    /// Samples in this window, interleaved when [`AudioFormat::channels`] is
    /// greater than one. [`AudioFormat::WAKE`] frames are mono, so the slice
    /// is the waveform.
    #[must_use]
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
}
