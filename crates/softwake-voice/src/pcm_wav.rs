//! Press-to-talk PCM buffer and WAV framing.
//!
//! Capture stays 16 kHz mono `i16`. The buffer is armed only while awake.
//! Default tests never open a microphone.

/// Shortest utterance accepted on release, in samples (0.3 s at 16 kHz).
pub const TALK_MIN_SAMPLES: usize = 4_800;

/// Longest utterance kept, in samples (15 s at 16 kHz).
pub const TALK_MAX_SAMPLES: usize = 16_000 * 15;

/// Samples collected while the mic button is held.
#[derive(Debug, Default, Clone)]
pub struct TalkBuffer {
    armed: bool,
    samples: Vec<i16>,
}

impl TalkBuffer {
    /// Empty, disarmed buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether [`Self::push`] is storing samples.
    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    /// Samples stored so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when no samples are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Start a new utterance. Drops anything already stored.
    pub fn arm(&mut self) {
        self.samples.clear();
        self.armed = true;
    }

    /// Stop accepting samples. The stored PCM stays until [`Self::take`].
    pub fn disarm(&mut self) {
        self.armed = false;
    }

    /// Drop stored samples and leave the buffer disarmed.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.armed = false;
    }

    /// Append samples while armed. Extra samples past the cap are dropped.
    pub fn push(&mut self, samples: &[i16]) {
        if !self.armed || samples.is_empty() {
            return;
        }
        let room = TALK_MAX_SAMPLES.saturating_sub(self.samples.len());
        if room == 0 {
            return;
        }
        let take = samples.len().min(room);
        self.samples.extend_from_slice(&samples[..take]);
    }

    /// Disarm and return the samples when they meet the minimum length.
    ///
    /// A short buffer is cleared and returns [`None`].
    #[must_use]
    pub fn take_if_long_enough(&mut self) -> Option<Vec<i16>> {
        self.armed = false;
        if self.samples.len() < TALK_MIN_SAMPLES {
            self.samples.clear();
            return None;
        }
        Some(std::mem::take(&mut self.samples))
    }
}

/// 16 kHz mono PCM WAV (RIFF) from little-endian `i16` samples.
#[must_use]
pub fn wav_from_pcm16(samples: &[i16]) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    let data_len = u32::try_from(samples.len().saturating_mul(2)).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(44 + samples.len().saturating_mul(2));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36u32.saturating_add(data_len)).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS / 8);
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = CHANNELS * (BITS / 8);
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&BITS.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{TALK_MAX_SAMPLES, TALK_MIN_SAMPLES, TalkBuffer, wav_from_pcm16};

    #[test]
    fn buffer_ignores_samples_until_armed_and_caps_length() {
        let mut buffer = TalkBuffer::new();
        buffer.push(&[1, 2, 3]);
        assert!(buffer.is_empty());
        buffer.arm();
        assert!(buffer.is_armed());
        buffer.push(&[1, 2, 3]);
        assert_eq!(buffer.len(), 3);
        let big = vec![7i16; TALK_MAX_SAMPLES];
        buffer.push(&big);
        assert_eq!(buffer.len(), TALK_MAX_SAMPLES);
        buffer.push(&[9]);
        assert_eq!(buffer.len(), TALK_MAX_SAMPLES);
    }

    #[test]
    fn short_release_clears_and_long_release_returns_pcm() {
        let mut buffer = TalkBuffer::new();
        buffer.arm();
        buffer.push(&[0; 100]);
        assert!(buffer.take_if_long_enough().is_none());
        assert!(!buffer.is_armed());
        assert!(buffer.is_empty());

        buffer.arm();
        buffer.push(&vec![1; TALK_MIN_SAMPLES]);
        let taken = buffer.take_if_long_enough().expect("long enough");
        assert_eq!(taken.len(), TALK_MIN_SAMPLES);
        assert!(buffer.is_empty());
    }

    #[test]
    fn wav_header_names_16k_mono() {
        let wav = wav_from_pcm16(&[0, 1]);
        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(wav[24..28].try_into().expect("rate")),
            16_000
        );
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().expect("ch")), 1);
    }
}
