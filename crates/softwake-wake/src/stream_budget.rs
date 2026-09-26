//! Sample budget for one sherpa-onnx keyword online stream.
//!
//! The spotter resets on a detected keyword. sherpa-onnx also resets inside
//! `DecodeStreams` after about 1.5 s of trailing blanks. Awake conversation
//! does not produce that blank run, so the same stream is `accept_waveform`'d
//! for the whole session and eventually stops emitting keywords. This budget
//! is the Softwake side of that reset: a few seconds of accepted audio, then
//! the caller resets before the next window.

/// Accepted samples (16 kHz) before the caller must reset the online stream.
///
/// Three seconds is longer than a wake or sleep phrase, and short enough that
/// a stale awake stream recovers while the session is still in use.
pub(crate) const RESET_AFTER_SAMPLES: u64 = 16_000 * 3;

/// Samples accepted since the last stream reset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StreamBudget {
    samples_since_reset: u64,
}

impl StreamBudget {
    /// Empty budget, as for a freshly created stream.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            samples_since_reset: 0,
        }
    }

    /// Samples accepted since the last [`Self::begin_window`] or keyword reset.
    #[must_use]
    pub(crate) const fn samples_since_reset(self) -> u64 {
        self.samples_since_reset
    }

    /// Drop the count, as after a silence reset of the online stream.
    pub(crate) const fn reset(&mut self) {
        self.samples_since_reset = 0;
    }

    /// Whether to reset the online stream *before* accepting this window.
    ///
    /// On `true`, the counter is cleared so the window starts a new budget.
    /// The caller still has to reset the sherpa stream itself.
    #[must_use]
    pub(crate) fn begin_window(&mut self) -> bool {
        if self.samples_since_reset >= RESET_AFTER_SAMPLES {
            self.samples_since_reset = 0;
            true
        } else {
            false
        }
    }

    /// Record a decoded window.
    ///
    /// `keyword_reset` means the spotter already reset on a non-empty keyword.
    /// That drops stream state, so this window's samples are not kept.
    pub(crate) fn finish_window(&mut self, n_samples: usize, keyword_reset: bool) {
        if keyword_reset {
            self.samples_since_reset = 0;
            return;
        }
        let added = u64::try_from(n_samples).unwrap_or(u64::MAX);
        self.samples_since_reset = self.samples_since_reset.saturating_add(added);
    }
}

#[cfg(test)]
mod tests {
    use super::{RESET_AFTER_SAMPLES, StreamBudget};

    #[test]
    fn reset_clears_a_partial_budget() {
        let mut budget = StreamBudget::new();
        budget.finish_window(100, false);
        budget.reset();
        assert_eq!(budget.samples_since_reset(), 0);
        assert!(!budget.begin_window());
    }

    #[test]
    fn three_second_budget_matches_16khz() {
        assert_eq!(RESET_AFTER_SAMPLES, 48_000);
        assert_eq!(RESET_AFTER_SAMPLES, 16_000 * 3);
    }

    #[test]
    fn under_the_limit_does_not_reset() {
        let mut budget = StreamBudget::new();
        assert!(!budget.begin_window());
        let almost = usize::try_from(RESET_AFTER_SAMPLES - 1).expect("fits");
        budget.finish_window(almost, false);
        assert_eq!(budget.samples_since_reset(), RESET_AFTER_SAMPLES - 1);
        assert!(!budget.begin_window());
    }

    #[test]
    fn next_window_after_the_limit_resets_then_counts_only_itself() {
        let mut budget = StreamBudget::new();
        let fill = usize::try_from(RESET_AFTER_SAMPLES).expect("fits");
        assert!(!budget.begin_window());
        budget.finish_window(fill, false);
        assert_eq!(budget.samples_since_reset(), RESET_AFTER_SAMPLES);

        assert!(budget.begin_window());
        assert_eq!(budget.samples_since_reset(), 0);
        let frame = 320;
        budget.finish_window(frame, false);
        assert_eq!(
            budget.samples_since_reset(),
            u64::try_from(frame).expect("fits")
        );
        assert!(!budget.begin_window());
    }

    #[test]
    fn keyword_hit_clears_a_partial_budget() {
        let mut budget = StreamBudget::new();
        assert!(!budget.begin_window());
        budget.finish_window(40_000, false);
        assert!(!budget.begin_window());
        // Same order as `SherpaKwsDetector::push_samples_detailed`: a keyword
        // reset inside the window must not keep the pre-hit sample count.
        budget.finish_window(320, true);
        assert_eq!(budget.samples_since_reset(), 0);
        assert!(!budget.begin_window());
    }

    #[test]
    fn five_minutes_of_awake_frames_reset_about_every_three_seconds() {
        let mut budget = StreamBudget::new();
        // 20 ms at 16 kHz, the usual capture window.
        let frame: usize = 320;
        let frame_samples = u64::try_from(frame).expect("fits");
        let total_frames = 16_000 * 60 * 5 / frame;
        let mut resets: u32 = 0;
        let mut gap: u64 = 0;
        let mut max_gap: u64 = 0;
        for _ in 0..total_frames {
            if budget.begin_window() {
                resets += 1;
                max_gap = max_gap.max(gap);
                gap = 0;
            }
            budget.finish_window(frame, false);
            gap = gap.saturating_add(frame_samples);
        }
        // 5 min / 3 s = 100 periods. The reset fires on the following window,
        // so the last boundary is pending and the count is 99.
        assert_eq!(resets, 99, "max_gap={max_gap}");
        assert_eq!(max_gap, RESET_AFTER_SAMPLES);
        assert_eq!(budget.samples_since_reset(), RESET_AFTER_SAMPLES);
        assert!(max_gap <= RESET_AFTER_SAMPLES + frame_samples);
    }
}
