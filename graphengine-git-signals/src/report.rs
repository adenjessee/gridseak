//! Report shapes returned by [`crate::GitSignalExtractor::extract`].
//!
//! Every field is serde-round-trippable so the report can be
//! embedded in the analysis `HealthReport` without a second
//! serialization path. Field names are stable cross-version
//! contracts — rename, don't repurpose.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Confidence, RepoShape};

/// Per-file signal bundle extracted from the commit walk.
///
/// Every field describes the same file over the same
/// [`crate::HistoryWindow`]; cross-window comparison requires two
/// extractions. `confidence` is the single field classifiers must
/// consult before acting on any of the numeric fields — a `Low`
/// confidence signal is *data*, not *actionable evidence*.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileSignals {
    /// Number of commits in the window that touched this file. Zero
    /// on shallow clones where the file has existed for longer than
    /// the shallow history but has not been touched within it.
    pub change_frequency: u32,
    /// Count of distinct commit authors within the window. One
    /// author with many commits is *not* the same signal as ten
    /// authors each with a few commits — ownership dispersion
    /// (below) captures that shape.
    pub distinct_authors: u32,
    /// Days since the most recent commit that touched this file,
    /// floor-rounded. `None` when the file had no commits in the
    /// window (shallow-clone rescue case: a file that exists in the
    /// working tree but was never modified inside the window).
    pub last_touched_days: Option<u32>,
    /// Normalized Herfindahl-Hirschman complement over per-author
    /// commit share. `0.0` = a single author owns every commit;
    /// `~1.0` = commits are uniformly distributed across many
    /// authors. Computed as `1 - Σ(share_i^2)` with the result
    /// clamped to `[0.0, 1.0]`. See
    /// [`crate::ownership::ownership_dispersion`].
    pub ownership_dispersion: f32,
    /// Synthetic hotspot signal: `change_frequency * ln(1 + loc)` is
    /// the standard shape, but because this crate does not know the
    /// LoC of the file (that is Layer 1 territory), we emit
    /// `change_frequency` scaled linearly by an `ln` stabiliser and
    /// leave the LoC multiplication for the classifier, which has
    /// Layer 1 context. Documented here so consumers know the
    /// numeric range is bounded by `change_frequency`, not by
    /// `change_frequency * file_complexity`.
    pub hotspot_score: f32,
    /// How much weight a classifier should give this bundle. Forced
    /// to [`Confidence::Low`] on any non-[`RepoShape::Full`] shape
    /// even if the raw numbers look strong; see
    /// [`crate::RepoShape`] for the guard semantics.
    pub confidence: Confidence,
}

/// Co-change cluster — a set of files that change together in the
/// same commit at least `co_commit_count` times within the window.
///
/// Emission rules:
///
/// - Minimum cluster size is two files (singletons are redundant
///   with [`FileSignals`]).
/// - Threshold is fixed at `co_commit_count >= 3` for `Full` repos;
///   on shallow clones the single available commit still yields a
///   cluster if two or more files were touched in it, with
///   `confidence = Low` and `co_commit_count = 1`. Classifiers
///   gated on `Confidence::High` silently skip the low-confidence
///   clusters.
/// - Same file set with different orders is deduplicated before
///   emission (the `files` vec is sorted ascending).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoChangeCluster {
    pub files: Vec<PathBuf>,
    pub co_commit_count: u32,
    pub confidence: Confidence,
}

/// Full report emitted by the extractor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GitSignalReport {
    /// Shape of the repository the report was extracted from. Used
    /// by downstream classifiers to interpret confidence bounds.
    pub repository_shape: RepoShape,
    /// Per-file signal bundles, keyed by working-tree-relative path.
    /// Keyed by [`BTreeMap`] (not `HashMap`) so the serialized
    /// output is deterministic for golden-file testing and byte-
    /// identical trend comparison.
    pub per_file: BTreeMap<PathBuf, FileSignals>,
    /// Co-change clusters above the emission threshold. Sorted by
    /// (`-co_commit_count`, `files.len()`, `files[0]`) for stable
    /// output.
    pub co_change_clusters: Vec<CoChangeCluster>,
    /// Caveat tokens stamped onto this report. See
    /// [`crate::caveats`] for the vocabulary; downstream consumers
    /// match on the constants, never on the raw strings. Stored as
    /// owned `String` so the report round-trips through `serde`'s
    /// deserialization path — the caveat constants are `&'static
    /// str` at the source of truth but the report itself is
    /// self-contained.
    pub integrity_caveats: Vec<String>,
    /// Number of commits the walker actually visited. Useful for
    /// diagnosing "report looks empty" — if `commits_walked = 0`
    /// the repository's HEAD was unreachable or the window hit
    /// `max_wall_clock` immediately.
    pub commits_walked: u32,
    /// Number of distinct files the walker saw across every visited
    /// commit. `per_file.len()` and this number should agree on a
    /// `Full` repo; they can diverge on shallow repos where
    /// co-change emission is gated differently.
    pub files_touched: u32,
}

impl GitSignalReport {
    /// Empty report shaped for "this directory is not a git tree."
    /// The `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1` caveat is stamped so
    /// downstream consumers can distinguish this empty report from
    /// a `Full`-shape repo that happens to have zero changes in the
    /// window.
    pub fn non_git() -> Self {
        Self {
            repository_shape: RepoShape::NonGit,
            per_file: BTreeMap::new(),
            co_change_clusters: Vec::new(),
            integrity_caveats: vec![
                crate::CAVEAT_LAYER0_GIT_SIGNALS_V1.to_owned(),
                crate::CAVEAT_LAYER0_UNSUPPORTED_VCS_V1.to_owned(),
            ],
            commits_walked: 0,
            files_touched: 0,
        }
    }

    /// Collect the set of files that appear in at least one
    /// co-change cluster. Used by classifiers that want to treat
    /// co-change membership as an independent signal from raw
    /// `FileSignals`.
    pub fn files_in_co_change_clusters(&self) -> BTreeSet<PathBuf> {
        self.co_change_clusters
            .iter()
            .flat_map(|c| c.files.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::GitSignalReport;
    use crate::{RepoShape, CAVEAT_LAYER0_UNSUPPORTED_VCS_V1};

    #[test]
    fn non_git_report_carries_vcs_unsupported_caveat() {
        let report = GitSignalReport::non_git();
        assert_eq!(report.repository_shape, RepoShape::NonGit);
        assert!(report.per_file.is_empty());
        assert!(report.co_change_clusters.is_empty());
        assert!(report
            .integrity_caveats
            .iter()
            .any(|c| c == CAVEAT_LAYER0_UNSUPPORTED_VCS_V1));
        assert_eq!(report.commits_walked, 0);
    }
}
