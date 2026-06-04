//! Bounded window for the commit walk.
//!
//! Every extraction operates inside a [`HistoryWindow`]. The three
//! bounds are load-bearing:
//!
//! - `commits_back` is the **primary** bound — always enforced, always
//!   finite. Bounds the cost of the walk even on infinite-history
//!   repositories (the linux kernel, `gridseak-self` in 10 years,
//!   etc.).
//! - `days_back` is a **secondary** bound, optional. When present, the
//!   walk stops early if the current commit's authored timestamp is
//!   older than `now - days_back` — useful for "only last quarter's
//!   churn matters" signals.
//! - `max_wall_clock` is a **hard** budget. Checked between commits,
//!   not mid-object-decode, so runaway object decoding can still
//!   overshoot by a single commit's cost. Exceeding it returns
//!   [`crate::ExtractError::WallClockExceeded`] with the count of
//!   commits the walker did manage to visit.

use std::time::Duration;

/// Parameter bundle for a single [`crate::GitSignalExtractor::extract`]
/// call.
///
/// See the module doc for the semantics of each bound.
#[derive(Debug, Clone)]
pub struct HistoryWindow {
    /// Cap on commits visited by the walker. Must be > 0; a window of
    /// `0` is a configuration bug, not "walk everything".
    pub commits_back: usize,
    /// Secondary age bound, in days. When `Some`, the walk stops at
    /// the first commit whose authored time is older than
    /// `now - days_back` days. `None` disables the bound.
    pub days_back: Option<u32>,
    /// Hard wall-clock budget for the whole extraction. Exceeding it
    /// returns [`crate::ExtractError::WallClockExceeded`]. Typical
    /// values: 2 s on a CI scan, 30 s on a cold-cache developer run.
    pub max_wall_clock: Duration,
}

impl HistoryWindow {
    /// Default window for a typical CI scan: 500 commits, last 365
    /// days, 2 s budget. Matches the T7 §5.7 kill criterion.
    pub fn default_ci() -> Self {
        Self {
            commits_back: 500,
            days_back: Some(365),
            max_wall_clock: Duration::from_secs(2),
        }
    }

    /// Unbounded-time, unbounded-commit window clipped only by the
    /// wall-clock budget. Used by ad-hoc developer tooling that wants
    /// to see the whole history.
    pub fn unbounded(max_wall_clock: Duration) -> Self {
        Self {
            commits_back: usize::MAX,
            days_back: None,
            max_wall_clock,
        }
    }
}

impl Default for HistoryWindow {
    fn default() -> Self {
        Self::default_ci()
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryWindow;
    use std::time::Duration;

    #[test]
    fn default_ci_has_two_second_budget() {
        let w = HistoryWindow::default_ci();
        assert_eq!(w.commits_back, 500);
        assert_eq!(w.days_back, Some(365));
        assert_eq!(w.max_wall_clock, Duration::from_secs(2));
    }

    #[test]
    fn unbounded_passes_through_budget() {
        let w = HistoryWindow::unbounded(Duration::from_secs(30));
        assert_eq!(w.commits_back, usize::MAX);
        assert!(w.days_back.is_none());
        assert_eq!(w.max_wall_clock, Duration::from_secs(30));
    }
}
