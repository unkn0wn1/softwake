//! KWS trigger thresholds for sherpa-onnx keyword spotting.
//!
//! Lower values fire more easily. Softwake defaults are looser than sherpa's
//! stock 0.25 so short wake/sleep words survive real mics and accents.
//! Operators can override via `softwake.json` / env (daemon wires that in).

/// Default global trigger threshold (multi-word phrases).
pub const DEFAULT_GLOBAL_THRESHOLD: f32 = 0.15;

/// Default per-keyword threshold for short single words (`#0.10`).
pub const DEFAULT_SHORT_THRESHOLD: f32 = 0.10;

/// Probe stream threshold for `-vv` near-miss logging.
///
/// sherpa's Rust API does not expose below-fire scores. A second stream at
/// this floor reports keywords that almost fired so operators can tune.
pub const DEFAULT_PROBE_THRESHOLD: f32 = 0.05;

/// Lowest accepted configured threshold.
pub const MIN_THRESHOLD: f32 = 0.05;

/// Highest accepted configured threshold.
pub const MAX_THRESHOLD: f32 = 0.50;

/// Trigger thresholds passed into the sherpa keyword spotter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KwsThresholds {
    /// Global `keywords_threshold` for multi-word phrases.
    pub global: f32,
    /// Per-keyword `#threshold` for [`crate::is_short_single_word`] phrases.
    pub short: f32,
    /// Probe-stream threshold used only for near-miss diagnostics.
    pub probe: f32,
}

impl Default for KwsThresholds {
    fn default() -> Self {
        Self {
            global: DEFAULT_GLOBAL_THRESHOLD,
            short: DEFAULT_SHORT_THRESHOLD,
            probe: DEFAULT_PROBE_THRESHOLD,
        }
    }
}

impl KwsThresholds {
    /// Build from raw floats, clamping each into [`MIN_THRESHOLD`]..=[`MAX_THRESHOLD`].
    ///
    /// `short` is also capped at `global` so short words stay easier (or equal).
    /// `probe` is capped at `short` so near-miss stays below fire.
    #[must_use]
    pub fn clamped(global: f32, short: f32, probe: f32) -> Self {
        let global = clamp_threshold(global);
        let short = clamp_threshold(short).min(global);
        let probe = clamp_threshold(probe).min(short);
        Self {
            global,
            short,
            probe,
        }
    }

    /// Convert milli-units (150 = 0.15) into thresholds.
    #[must_use]
    pub fn from_milli(global_milli: u16, short_milli: u16) -> Self {
        Self::clamped(
            f32::from(global_milli) / 1000.0,
            f32::from(short_milli) / 1000.0,
            DEFAULT_PROBE_THRESHOLD,
        )
    }

    /// Format the short-word `#threshold` suffix (e.g. `" #0.10"`).
    #[must_use]
    pub fn short_suffix(self) -> String {
        format!(" #{:.2}", self.short)
    }

    /// Format the probe `#threshold` suffix.
    #[must_use]
    pub fn probe_suffix(self) -> String {
        format!(" #{:.2}", self.probe)
    }
}

fn clamp_threshold(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_GLOBAL_THRESHOLD;
    }
    value.clamp(MIN_THRESHOLD, MAX_THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_GLOBAL_THRESHOLD, DEFAULT_PROBE_THRESHOLD, DEFAULT_SHORT_THRESHOLD, KwsThresholds,
        MAX_THRESHOLD, MIN_THRESHOLD,
    };

    #[test]
    fn defaults_match_product_numbers() {
        let t = KwsThresholds::default();
        assert!((t.global - DEFAULT_GLOBAL_THRESHOLD).abs() < f32::EPSILON);
        assert!((t.short - DEFAULT_SHORT_THRESHOLD).abs() < f32::EPSILON);
        assert!((t.probe - DEFAULT_PROBE_THRESHOLD).abs() < f32::EPSILON);
        assert!(t.short <= t.global);
        assert!(t.probe <= t.short);
    }

    #[test]
    fn from_milli_round_trips_common_values() {
        let t = KwsThresholds::from_milli(150, 100);
        assert!((t.global - 0.15).abs() < 0.000_1);
        assert!((t.short - 0.10).abs() < 0.000_1);
    }

    #[test]
    fn clamp_enforces_order_and_bounds() {
        let t = KwsThresholds::clamped(0.01, 0.40, 0.90);
        assert!((t.global - MIN_THRESHOLD).abs() < f32::EPSILON);
        assert!(t.short <= t.global);
        assert!(t.probe <= t.short);
        let high = KwsThresholds::clamped(0.9, 0.9, 0.9);
        assert!((high.global - MAX_THRESHOLD).abs() < f32::EPSILON);
    }

    #[test]
    fn suffixes_use_two_decimal_places() {
        let t = KwsThresholds::default();
        assert_eq!(t.short_suffix(), " #0.10");
        assert_eq!(t.probe_suffix(), " #0.05");
    }
}
