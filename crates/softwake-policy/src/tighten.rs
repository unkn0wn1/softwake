//! Risk lattice for policy decisions.
//!
//! `Safe` is weaker than `Confirm`. `Confirm` is weaker than `Deny`.
//! [`tighten`] returns the more restrictive of the two inputs.

use crate::PolicyDecision;

const fn rank(decision: PolicyDecision) -> u8 {
    match decision {
        PolicyDecision::Safe => 0,
        PolicyDecision::Confirm => 1,
        PolicyDecision::Deny => 2,
    }
}

/// More restrictive of `floor` and `requested`.
///
/// A requested decision that is weaker than `floor` is ignored.
#[must_use]
pub const fn tighten(floor: PolicyDecision, requested: PolicyDecision) -> PolicyDecision {
    if rank(requested) > rank(floor) {
        requested
    } else {
        floor
    }
}

/// Most restrictive request, if `requests` yields any.
pub(crate) fn strictest(requests: impl Iterator<Item = PolicyDecision>) -> Option<PolicyDecision> {
    requests.reduce(tighten)
}

#[cfg(test)]
mod tests {
    use super::tighten;
    use crate::PolicyDecision::{Confirm, Deny, Safe};

    #[test]
    fn tighten_keeps_the_more_restrictive_decision() {
        assert_eq!(tighten(Safe, Safe), Safe);
        assert_eq!(tighten(Confirm, Confirm), Confirm);
        assert_eq!(tighten(Deny, Deny), Deny);
        assert_eq!(tighten(Safe, Confirm), Confirm);
        assert_eq!(tighten(Confirm, Safe), Confirm);
        assert_eq!(tighten(Safe, Deny), Deny);
        assert_eq!(tighten(Deny, Safe), Deny);
        assert_eq!(tighten(Confirm, Deny), Deny);
        assert_eq!(tighten(Deny, Confirm), Deny);
    }
}
