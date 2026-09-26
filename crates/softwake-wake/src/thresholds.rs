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

/// Lowest accepted configured threshold (global / short fire).
pub const MIN_THRESHOLD: f32 = 0.05;

/// Lowest probe / near-miss floor (may sit below [`MIN_THRESHOLD`]).
pub const MIN_PROBE_THRESHOLD: f32 = 0.01;

/// Step used to keep probe strictly below the easiest fire threshold.
pub const PROBE_FIRE_GAP: f32 = 0.01;

/// Highest accepted configured threshold.
pub const MAX_THRESHOLD: f32 = 0.50;

/// Extra ease for the active profile name / `hey <name>` (subtracted from short).
///
/// Bare names like `sally` score weaker than product tokens like `softwake` at
/// the same short threshold (`GigaSpeech` KWS). Cap so we never go below
/// [`MIN_NAME_THRESHOLD`].
pub const NAME_THRESHOLD_EASE: f32 = 0.05;

/// Floor for profile-name per-keyword thresholds.
pub const MIN_NAME_THRESHOLD: f32 = 0.03;

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
    /// `probe` stays **strictly below** `short` (gap [`PROBE_FIRE_GAP`]) so a
    /// near-miss cannot tie the fire stream when Settings sets them equal.
    #[must_use]
    pub fn clamped(global: f32, short: f32, probe: f32) -> Self {
        let global = clamp_threshold(global);
        let short = clamp_threshold(short).min(global);
        let probe_cap = (short - PROBE_FIRE_GAP).max(MIN_PROBE_THRESHOLD);
        let probe = clamp_probe(probe).min(probe_cap);
        Self {
            global,
            short,
            probe,
        }
    }

    /// Per-keyword threshold for the active profile name and `hey <name>`.
    ///
    /// Easier than [`Self::short`] so profile wakes (e.g. `sally`) keep up with
    /// product tokens (`softwake`) without relaxing every short word.
    #[must_use]
    pub fn name_threshold(self) -> f32 {
        (self.short - NAME_THRESHOLD_EASE)
            .max(MIN_NAME_THRESHOLD)
            .min(self.short)
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

    /// Format the profile-name `#threshold` suffix.
    #[must_use]
    pub fn name_suffix(self) -> String {
        format!(" #{:.2}", self.name_threshold())
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

fn clamp_probe(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_PROBE_THRESHOLD;
    }
    value.clamp(MIN_PROBE_THRESHOLD, MAX_THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_GLOBAL_THRESHOLD, DEFAULT_PROBE_THRESHOLD, DEFAULT_SHORT_THRESHOLD, KwsThresholds,
        MAX_THRESHOLD, MIN_NAME_THRESHOLD, MIN_THRESHOLD,
    };

    #[test]
    fn defaults_match_product_numbers() {
        let t = KwsThresholds::default();
        assert!((t.global - DEFAULT_GLOBAL_THRESHOLD).abs() < f32::EPSILON);
        assert!((t.short - DEFAULT_SHORT_THRESHOLD).abs() < f32::EPSILON);
        assert!((t.probe - DEFAULT_PROBE_THRESHOLD).abs() < f32::EPSILON);
        assert!(t.short <= t.global);
        assert!(t.probe < t.short);
        assert!((t.name_threshold() - 0.05).abs() < 0.000_1);
    }

    #[test]
    fn probe_stays_strictly_below_fire_when_settings_tie() {
        let t = KwsThresholds::clamped(0.05, 0.05, 0.05);
        assert!((t.global - 0.05).abs() < f32::EPSILON);
        assert!((t.short - 0.05).abs() < f32::EPSILON);
        assert!(t.probe < t.short, "probe={} short={}", t.probe, t.short);
        assert!((t.probe - 0.04).abs() < 0.000_1);
        // Name ease at the fire floor still helps profile wakes.
        assert!(t.name_threshold() < t.short);
        assert!((t.name_threshold() - MIN_NAME_THRESHOLD).abs() < 0.000_1);
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
        assert!(t.probe < t.short);
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
