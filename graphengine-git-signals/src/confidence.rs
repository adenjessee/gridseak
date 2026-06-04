//! Confidence envelope for every signal this crate emits.
//!
//! The three-level shape intentionally matches
//! [`graphengine_analysis::health::report::Confidence`] so that
//! downstream consumers can merge Layer 0 signals into the same
//! confidence vocabulary as edges and findings without a cross-type
//! coercion. We do **not** depend on `graphengine-analysis` from this
//! crate (would create a cycle; analysis depends on this crate via
//! the HealthReport pipeline), so the enum is duplicated; the
//! identity of the variants and their `#[serde(rename_all)]` shape is
//! the cross-crate contract that keeps the two vocabularies aligned.

use serde::{Deserialize, Serialize};

/// How much weight a downstream classifier should give a Layer 0
/// signal.
///
/// - **High**: signal derives from a full-history working tree where
///   the [`crate::HistoryWindow`] bound covers at least the
///   configured `commits_back`. Authorship, churn, and co-change
///   numbers are all reliable.
/// - **Medium**: signal derives from a full-history tree but the
///   window exhausted its `max_wall_clock` budget before walking the
///   full `commits_back`. Numbers are lower bounds; the file may be
///   strictly hotter than the report implies.
/// - **Low**: signal derives from a shallow clone, a bare repository,
///   or a repository that only contains a single commit. Churn and
///   co-change are structurally impossible to compute and the
///   numbers should not drive a classifier downgrade.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    /// True when this confidence level is strong enough for a
    /// classifier to *act* on the signal (e.g. downgrade a dead-code
    /// verdict). Callers consuming git signals gated on "do I trust
    /// this number enough to change behaviour?" should read this
    /// predicate rather than comparing variants directly.
    pub fn is_actionable(self) -> bool {
        matches!(self, Self::High)
    }

    /// Downgrade one step toward [`Confidence::Low`]. Used by the
    /// shallow-clone guard to rescue a per-file signal from being
    /// falsely trusted.
    pub fn downgrade(self) -> Self {
        match self {
            Self::High => Self::Medium,
            Self::Medium => Self::Low,
            Self::Low => Self::Low,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Confidence;

    #[test]
    fn only_high_is_actionable() {
        assert!(Confidence::High.is_actionable());
        assert!(!Confidence::Medium.is_actionable());
        assert!(!Confidence::Low.is_actionable());
    }

    #[test]
    fn downgrade_walks_to_low_and_saturates() {
        assert_eq!(Confidence::High.downgrade(), Confidence::Medium);
        assert_eq!(Confidence::Medium.downgrade(), Confidence::Low);
        assert_eq!(Confidence::Low.downgrade(), Confidence::Low);
    }
}
