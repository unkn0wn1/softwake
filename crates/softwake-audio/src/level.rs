//! Capture energy for HUD / status.
//!
//! [`rms_level`] maps a PCM window to `0.0..=1.0` so the daemon can put a
//! single number on status without shipping raw samples.

/// Peak-normalized RMS of `samples`, clamped to `0.0..=1.0`.
///
/// Empty input is `0.0`. The scale is full-scale `i16` (`1.0` ≈ rail-to-rail).
#[must_use]
pub fn rms_level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum_sq = 0.0_f32;
    for sample in samples {
        let x = f32::from(*sample) / f32::from(i16::MAX);
        sum_sq += x * x;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "frame length fits comfortably in f32 mantissa for wake windows"
    )]
    let mean = sum_sq / samples.len() as f32;
    mean.sqrt().clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::rms_level;

    #[test]
    fn silence_and_empty_are_zero() {
        assert!((rms_level(&[]) - 0.0).abs() < f32::EPSILON);
        assert!((rms_level(&[0, 0, 0, 0]) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn full_scale_square_is_near_one() {
        let samples = [i16::MAX, i16::MIN, i16::MAX, i16::MIN];
        let level = rms_level(&samples);
        assert!(level > 0.99, "{level}");
        assert!(level <= 1.0, "{level}");
    }

    #[test]
    fn quieter_tone_is_between_zero_and_one() {
        let mut samples = [0_i16; 160];
        for (i, sample) in samples.iter_mut().enumerate() {
            #[allow(
                clippy::cast_precision_loss,
                reason = "index 0..160 is exact in f32"
            )]
            let phase = (i as f32) * 2.0 * std::f32::consts::PI * 440.0 / 16_000.0;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "intentional PCM quantization"
            )]
            {
                *sample = (0.25 * phase.sin() * f32::from(i16::MAX)) as i16;
            }
        }
        let level = rms_level(&samples);
        assert!(level > 0.1, "{level}");
        assert!(level < 0.5, "{level}");
    }
}
