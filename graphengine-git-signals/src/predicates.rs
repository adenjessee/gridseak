//! Predicate trait that downstream classifiers consume instead of
//! reading raw [`crate::FileSignals`] fields.
//!
//! The trait mirrors the `EdgeKind::is_call_like()` pattern from
//! P1.a: adding a new signal type updates this trait once, and every
//! classifier that asks "is this file active?" / "is this file
//! hot?" automatically picks up the new signal. A classifier that
//! reads `signals.change_frequency` directly is a layering
//! violation — the numeric fields are data, the trait is the
//! consumption contract.
//!
//! The trait is implemented on [`crate::FileSignals`] here; adding a
//! new implementor (e.g. a synthetic "zero-evidence" sentinel used
//! by tests) requires implementing every predicate, which is
//! exhaustiveness the compiler enforces.

use crate::{Confidence, FileSignals};

/// Recency horizon (days) past which a file stops being "active."
/// Chosen at 30 days to match the dead-code-downgrade horizon named
/// in T7 §4.3. Not a configuration dial — predicates are a
/// contract, not a tunable.
pub const ACTIVE_RECENT_MAX_DAYS: u32 = 30;

/// Minimum commit count required before a file qualifies as
/// high-churn. Chosen at 5 (rather than e.g. 3) because three
/// commits over a quarter-long window is a single feature cycle on
/// a typical project; five starts to indicate a pattern.
pub const HIGH_CHURN_MIN_COMMITS: u32 = 5;

/// Minimum author count required before dispersion (rather than raw
/// churn) counts as a hotspot signal. Three authors keeps the rule
/// honest on small teams.
pub const HOTSPOT_MIN_AUTHORS: u32 = 3;

/// Consumption contract for Layer 0 signals.
///
/// Every classifier that wants to gate behaviour on git history
/// reads through this trait. Classifiers that read `FileSignals`
/// fields directly are silently coupled to the field shape and
/// break quietly when a new confidence rule lands — this trait is
/// the seam that keeps the two concerns decoupled.
pub trait GitSignalConsumer {
    /// True when the file has been committed within
    /// [`ACTIVE_RECENT_MAX_DAYS`] AND the signal carries
    /// [`Confidence::High`]. Consumers of this predicate may
    /// downgrade a `dead_code` verdict from `High` to `Medium`
    /// because "recently touched code is seldom truly dead."
    fn is_active_recent(&self) -> bool;

    /// True when the file has at least [`HIGH_CHURN_MIN_COMMITS`]
    /// commits in the window AND `Confidence::High`. Consumers may
    /// emit a `hotspot` finding regardless of any code-graph
    /// signal, because a frequently-changed file is risky by
    /// definition. Gated on confidence so a 1-commit shallow clone
    /// does not synthesise a false hotspot.
    fn is_high_churn(&self) -> bool;

    /// True when the file has a high ownership dispersion
    /// (≥ [`HOTSPOT_MIN_AUTHORS`] distinct authors and dispersion
    /// ≥ 0.5) AND `Confidence::High`. Consumers may emit a
    /// `shared_knowledge_risk` finding when this fires on a
    /// business-critical module.
    fn is_shared_ownership(&self) -> bool;

    /// True when the signal carries `Confidence::High` AND the
    /// caller should trust ANY of the numeric fields below for
    /// decision-making. Callers that want to report the numbers
    /// verbatim (e.g. in a UI) can ignore this gate; callers that
    /// want to *act* on them (downgrade / upgrade / emit a finding)
    /// MUST consult it.
    fn is_actionable(&self) -> bool;
}

impl GitSignalConsumer for FileSignals {
    fn is_active_recent(&self) -> bool {
        matches!(self.confidence, Confidence::High)
            && self
                .last_touched_days
                .map(|d| d <= ACTIVE_RECENT_MAX_DAYS)
                .unwrap_or(false)
    }

    fn is_high_churn(&self) -> bool {
        matches!(self.confidence, Confidence::High)
            && self.change_frequency >= HIGH_CHURN_MIN_COMMITS
    }

    fn is_shared_ownership(&self) -> bool {
        matches!(self.confidence, Confidence::High)
            && self.distinct_authors >= HOTSPOT_MIN_AUTHORS
            && self.ownership_dispersion >= 0.5
    }

    fn is_actionable(&self) -> bool {
        matches!(self.confidence, Confidence::High)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Confidence;

    fn bundle(
        commits: u32,
        authors: u32,
        last_touched: Option<u32>,
        dispersion: f32,
        confidence: Confidence,
    ) -> FileSignals {
        FileSignals {
            change_frequency: commits,
            distinct_authors: authors,
            last_touched_days: last_touched,
            ownership_dispersion: dispersion,
            hotspot_score: commits as f32,
            confidence,
        }
    }

    #[test]
    fn is_active_recent_requires_high_confidence() {
        let low_conf = bundle(1, 1, Some(5), 0.0, Confidence::Low);
        assert!(!low_conf.is_active_recent());
        let high_conf = bundle(1, 1, Some(5), 0.0, Confidence::High);
        assert!(high_conf.is_active_recent());
    }

    #[test]
    fn is_active_recent_requires_within_horizon() {
        let stale = bundle(1, 1, Some(120), 0.0, Confidence::High);
        assert!(!stale.is_active_recent());
        let untouched = bundle(0, 0, None, 0.0, Confidence::High);
        assert!(!untouched.is_active_recent());
    }

    #[test]
    fn is_high_churn_gates_on_commit_count() {
        let low_churn = bundle(3, 1, Some(10), 0.0, Confidence::High);
        assert!(!low_churn.is_high_churn());
        let above_threshold = bundle(HIGH_CHURN_MIN_COMMITS, 1, Some(10), 0.0, Confidence::High);
        assert!(above_threshold.is_high_churn());
    }

    #[test]
    fn is_high_churn_never_true_under_low_confidence() {
        let fake_high_churn = bundle(100, 5, Some(5), 0.9, Confidence::Low);
        assert!(!fake_high_churn.is_high_churn());
    }

    #[test]
    fn is_shared_ownership_requires_authors_and_dispersion() {
        let solo = bundle(20, 1, Some(5), 0.0, Confidence::High);
        assert!(!solo.is_shared_ownership());
        let low_dispersion = bundle(20, 3, Some(5), 0.2, Confidence::High);
        assert!(!low_dispersion.is_shared_ownership());
        let shared = bundle(20, 3, Some(5), 0.67, Confidence::High);
        assert!(shared.is_shared_ownership());
    }
}
