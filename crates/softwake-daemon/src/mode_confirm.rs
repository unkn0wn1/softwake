//! One pending sleep, hibernate, or fuzzy-wake confirm.
//!
//! Separate from Hands tool confirmation. The clock is passed in so tests
//! move `Instant` forward without sleeping.

use std::time::{Duration, Instant};

/// How long a confirm waits for yes or no, from the moment it is armed.
pub(crate) const CONFIRM_WINDOW: Duration = Duration::from_secs(15);

/// Minimum gap between fuzzy-wake questions, measured from arming.
pub(crate) const FUZZY_COOLDOWN: Duration = Duration::from_secs(45);

/// Fixed confirm prompts. Spoken with the profile voice, not rewritten.
pub(crate) const SLEEP_ASK: &str = "Sleep now?";
/// Fixed hibernate confirm.
pub(crate) const HIBERNATE_ASK: &str = "Hibernate now?";
/// Fixed fuzzy-wake clarifier.
pub(crate) const FUZZY_ASK: &str = "Were you trying to wake me?";
/// Spoken on an explicit no while awake.
pub(crate) const STAY_AWAKE: &str = "Okay, staying awake.";
/// Spoken on an explicit no while the fuzzy-wake question is open.
pub(crate) const STAY_ASLEEP: &str = "Okay, staying asleep.";

/// What a yes will apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmKind {
    /// Awake → sleep.
    Sleep,
    /// Awake → hibernate.
    Hibernate,
    /// Sleep → awake.
    FuzzyWake,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    kind: ConfirmKind,
    deadline: Instant,
}

/// At most one confirm, plus the fuzzy rate-limit clock.
#[derive(Debug, Default)]
pub(crate) struct ModeConfirm {
    pending: Option<Pending>,
    last_fuzzy_arm: Option<Instant>,
}

impl ModeConfirm {
    /// Arm `kind` at `now`. False when one is already waiting, or fuzzy is cooling.
    pub(crate) fn arm(&mut self, kind: ConfirmKind, now: Instant) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if kind == ConfirmKind::FuzzyWake && !self.fuzzy_cooldown_elapsed(now) {
            return false;
        }
        if kind == ConfirmKind::FuzzyWake {
            self.last_fuzzy_arm = Some(now);
        }
        self.pending = Some(Pending {
            kind,
            deadline: now + CONFIRM_WINDOW,
        });
        true
    }

    /// Drop the pending confirm. The fuzzy clock stays.
    pub(crate) fn clear(&mut self) {
        self.pending = None;
    }

    /// True when a confirm is waiting and `now` is at or past its deadline.
    #[must_use]
    pub(crate) fn expired(&self, now: Instant) -> bool {
        self.pending.is_some_and(|pending| now >= pending.deadline)
    }

    /// Kind waiting, if any.
    #[must_use]
    pub(crate) fn pending_kind(&self) -> Option<ConfirmKind> {
        self.pending.map(|pending| pending.kind)
    }

    /// Fuzzy may arm: nothing is pending and the cooldown has elapsed.
    #[must_use]
    pub(crate) fn fuzzy_allowed(&self, now: Instant) -> bool {
        self.pending.is_none() && self.fuzzy_cooldown_elapsed(now)
    }

    /// Put the deadline in the past. Tests only.
    #[cfg(test)]
    pub(crate) fn force_expired(&mut self, now: Instant) {
        if let Some(pending) = self.pending.as_mut() {
            pending.deadline = now;
        }
    }

    fn fuzzy_cooldown_elapsed(&self, now: Instant) -> bool {
        self.last_fuzzy_arm
            .is_none_or(|armed| now.saturating_duration_since(armed) >= FUZZY_COOLDOWN)
    }
}

/// Line spoken when `kind` is armed.
#[must_use]
pub(crate) const fn prompt_line(kind: ConfirmKind) -> &'static str {
    match kind {
        ConfirmKind::Sleep => SLEEP_ASK,
        ConfirmKind::Hibernate => HIBERNATE_ASK,
        ConfirmKind::FuzzyWake => FUZZY_ASK,
    }
}

/// Line spoken on an explicit no.
#[must_use]
pub(crate) const fn cancel_line(kind: ConfirmKind) -> &'static str {
    match kind {
        ConfirmKind::Sleep | ConfirmKind::Hibernate => STAY_AWAKE,
        ConfirmKind::FuzzyWake => STAY_ASLEEP,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ConfirmKind, ModeConfirm};

    #[test]
    fn window_is_open_at_14s_and_closed_at_15s() {
        let t0 = Instant::now();
        let mut gate = ModeConfirm::default();
        assert!(gate.arm(ConfirmKind::Sleep, t0));
        assert!(!gate.expired(t0 + Duration::from_secs(14)));
        assert!(gate.expired(t0 + Duration::from_secs(15)));
    }

    #[test]
    fn fuzzy_cooldown_blocks_a_second_arm_until_45s() {
        let t0 = Instant::now();
        let mut gate = ModeConfirm::default();
        assert!(gate.arm(ConfirmKind::FuzzyWake, t0));
        gate.clear();
        assert!(!gate.fuzzy_allowed(t0 + Duration::from_secs(44)));
        assert!(!gate.arm(ConfirmKind::FuzzyWake, t0 + Duration::from_secs(44)));
        assert!(gate.fuzzy_allowed(t0 + Duration::from_secs(45)));
        assert!(gate.arm(ConfirmKind::FuzzyWake, t0 + Duration::from_secs(45)));
    }

    #[test]
    fn sleep_arm_does_not_start_the_fuzzy_clock() {
        let t0 = Instant::now();
        let mut gate = ModeConfirm::default();
        assert!(gate.arm(ConfirmKind::Sleep, t0));
        gate.clear();
        assert!(gate.fuzzy_allowed(t0));
        assert!(gate.arm(ConfirmKind::FuzzyWake, t0));
        assert_eq!(gate.pending_kind(), Some(ConfirmKind::FuzzyWake));
    }

    #[test]
    fn clear_drops_pending_without_marking_it_expired() {
        let t0 = Instant::now();
        let mut gate = ModeConfirm::default();
        assert!(gate.arm(ConfirmKind::Hibernate, t0));
        gate.clear();
        assert_eq!(gate.pending_kind(), None);
        assert!(!gate.expired(t0 + Duration::from_secs(100)));
    }

    #[test]
    fn a_pending_confirm_blocks_another_arm() {
        let t0 = Instant::now();
        let mut gate = ModeConfirm::default();
        assert!(gate.arm(ConfirmKind::Sleep, t0));
        assert!(!gate.arm(ConfirmKind::Hibernate, t0));
        assert_eq!(gate.pending_kind(), Some(ConfirmKind::Sleep));
    }
}
