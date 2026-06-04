//! Attach T7 Layer 0 git signals to a [`HealthReport`].
//!
//! This module is the **only** seam that couples the parse-DB-driven
//! health pipeline (in `mod.rs`) to the filesystem-driven git-signal
//! extractor (`graphengine_git_signals`). Keeping the coupling in a
//! single file is a deliberate application of the two-jobs rule:
//!
//! - `run_analysis_with_config` reads the parse database and nothing
//!   else. It does not know about git.
//! - `attach_git_signals` reads the working tree, runs the
//!   extractor, and writes the result onto a report the caller
//!   already built. It does not know about parse databases.
//!
//! A downstream orchestrator (the `ge-analyze` binary, the MCP
//! server's handler, tests) composes the two calls. This is the
//! measured-fallback discipline: if the git extractor fails the
//! report is still usable — the `git_signals` field is simply left
//! `None`.

use std::path::Path;

use graphengine_git_signals::{
    GitSignalExtractor, HistoryWindow, OpenError, RepoShape, CAVEAT_LAYER0_GIT_SIGNALS_V1,
    CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1, CAVEAT_LAYER0_UNSUPPORTED_VCS_V1,
};

use crate::health::report::HealthReport;

/// Outcome of [`attach_git_signals`]. A successful attach is not the
/// same as "signals are trustworthy" — the shape reported inside
/// [`HealthReport::git_signals`] is still the source of truth for
/// ranking logic. This enum just reports whether the extractor
/// reached the report or bailed, so the caller can log / telemetry
/// the skip without forking the return shape.
#[derive(Debug)]
pub enum GitSignalAttachOutcome {
    /// A [`graphengine_git_signals::GitSignalReport`] was produced
    /// and stored in `report.git_signals`. The relevant
    /// `CAVEAT_LAYER0_*` strings have been appended to
    /// `report.integrity_status.schema_caveats`.
    Attached { shape: RepoShape },
    /// The extractor could not even open the repository. The
    /// attach is a no-op: `report.git_signals` stays `None` and no
    /// caveats are stamped, so the report unambiguously carries
    /// "no measurement taken". The underlying [`OpenError`] is
    /// returned for logging.
    Skipped(OpenError),
}

/// Run [`GitSignalExtractor::open`] + `extract` against `repo_root`
/// with `window`, and install the result onto `report`. Stamps the
/// appropriate caveats on `report.integrity_status.schema_caveats`
/// so downstream readers can tell shallow-clone reports apart from
/// full-history reports without re-reading the nested block.
///
/// Never panics and never returns `Err`. The extractor's failure
/// modes are all degradation, not fatality: a corrupted repository
/// produces an `ExtractError` that we log and convert into a skip;
/// a missing directory produces an `OpenError` the caller already
/// knew was possible. The philosophy here is identical to the rest
/// of the sprint: absence of measurement is reported as absence,
/// never substituted with silence or with a fake default.
pub fn attach_git_signals(
    report: &mut HealthReport,
    repo_root: &Path,
    window: &HistoryWindow,
) -> GitSignalAttachOutcome {
    let extractor = match GitSignalExtractor::open(repo_root) {
        Ok(x) => x,
        Err(err) => {
            eprintln!(
                "[ge-analyze] git-signals open failed for {:?}: {err}; report.git_signals left None",
                repo_root
            );
            return GitSignalAttachOutcome::Skipped(err);
        }
    };
    let shape = extractor.repo_shape();

    let git_report = match extractor.extract(window) {
        Ok(r) => r,
        Err(err) => {
            eprintln!(
                "[ge-analyze] git-signals extract failed for {:?}: {err}; report.git_signals left None",
                repo_root
            );
            // `extract` failure is a real extractor error, not a
            // graceful "no history" — skip rather than stamp
            // misleading caveats.
            return GitSignalAttachOutcome::Skipped(OpenError::GixOpenFailed {
                path: repo_root.to_path_buf(),
                message: err.to_string(),
            });
        }
    };

    stamp_caveats(report, shape);
    report.git_signals = Some(git_report);
    GitSignalAttachOutcome::Attached { shape }
}

/// Extend `report.integrity_status.schema_caveats` with the Layer 0
/// caveats appropriate to `shape`. Always stamps
/// `CAVEAT_LAYER0_GIT_SIGNALS_V1` so consumers know the attach path
/// ran; adds the shape-specific caveat when applicable.
fn stamp_caveats(report: &mut HealthReport, shape: RepoShape) {
    let caveats = &mut report.integrity_status.schema_caveats;
    push_once(caveats, CAVEAT_LAYER0_GIT_SIGNALS_V1);
    match shape {
        RepoShape::Full => {}
        RepoShape::Shallow { .. } | RepoShape::Bare => {
            push_once(caveats, CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1);
        }
        RepoShape::NonGit => {
            push_once(caveats, CAVEAT_LAYER0_UNSUPPORTED_VCS_V1);
        }
    }
}

fn push_once(caveats: &mut Vec<String>, token: &str) {
    if !caveats.iter().any(|c| c == token) {
        caveats.push(token.to_string());
    }
}

/// Walk every dead node annotation in `report`, and where the
/// file it lives in shows recent High-confidence churn in
/// `report.git_signals`, downgrade
/// `NodeAnnotation.dead_code_confidence` from `High` to `Medium`.
/// Returns the number of annotations whose confidence was actually
/// lowered.
///
/// This is the operational counterpart of
/// [`crate::health::dead_code_classifier::apply_git_signal_churn_downgrade`] —
/// same predicate, same threshold — but operates on the final
/// report shape so callers do not need to hold the
/// [`crate::health::graph::AnalysisGraph`] any more. The two
/// functions deliberately share their rules in lockstep (both
/// gate on `Confidence::High` from the git signal and on the
/// `ACTIVE_RECENT_MAX_DAYS` threshold) so the report's
/// `dead_code_confidence` always matches what the classifier
/// would have produced if given the same inputs.
///
/// No-op when `git_signals` is `None`, i.e. preserving the T7
/// "absent ≠ negative" contract.
pub fn apply_dead_code_churn_downgrade_to_annotations(
    report: &mut crate::health::report::HealthReport,
) -> usize {
    let Some(git_report) = report.git_signals.as_ref() else {
        return 0;
    };
    let mut downgraded: usize = 0;
    for ann in report.node_annotations.values_mut() {
        if !ann.is_dead {
            continue;
        }
        if !matches!(
            ann.dead_code_confidence,
            Some(crate::health::report::Confidence::High)
        ) {
            continue;
        }
        let Some(path_str) = ann.file_path.as_ref() else {
            continue;
        };
        let key = std::path::PathBuf::from(path_str);
        let Some(signals) = git_report.per_file.get(&key) else {
            continue;
        };
        if !matches!(
            signals.confidence,
            graphengine_git_signals::Confidence::High
        ) {
            continue;
        }
        let within_window = signals
            .last_touched_days
            .map(|d| d < graphengine_git_signals::predicates::ACTIVE_RECENT_MAX_DAYS)
            .unwrap_or(false);
        if !within_window {
            continue;
        }
        ann.dead_code_confidence = Some(crate::health::report::Confidence::Medium);
        downgraded += 1;
    }
    downgraded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::report::IntegrityStatus;
    use graphengine_git_signals::{FileSignals, GitSignalReport};
    use std::collections::BTreeMap;

    fn bare_report() -> HealthReport {
        // Build the minimum-viable HealthReport by round-tripping a
        // JSON literal. Avoids reproducing 40+ fields of default
        // scoring data here and stays robust to field additions.
        let json = serde_json::json!({
            "version": "1.0.0",
            "generated_at": "2026-04-18T00:00:00Z",
            "analysis_duration_ms": 0,
            "db_path": ":memory:",
            "health_score_components": {
                "cycle_severity": { "score": 100, "weight": 0.1 },
                "coupling_health": { "score": 100, "weight": 0.1 },
                "hotspot_concentration": { "score": 100, "weight": 0.1 },
                "dead_code_ratio": { "score": 100, "weight": 0.1 },
                "depth_complexity": { "score": 100, "weight": 0.1 },
                "complexity": { "score": 100, "weight": 0.1 },
                "cohesion": { "score": 100, "weight": 0.1 },
                "distance": { "score": 100, "weight": 0.1 },
                "temporal_coupling": { "score": 100, "weight": 0.2 }
            },
            "metrics": {
                "cycles": { "count": 0, "total": 0, "ratio": 0.0, "description": "" },
                "coupling": { "modules_measured": 0, "modules_above_070": 0, "modules_above_050": 0, "avg_coupling": 0.0, "description": "" },
                "hotspot_concentration": { "count": 0, "total": 0, "ratio": 0.0, "description": "" },
                "dead_code": { "count": 0, "total": 0, "ratio": 0.0, "description": "" },
                "depth": { "max_call_depth": 0, "description": "" },
                "tangle_index": { "count": 0, "total": 0, "ratio": 0.0, "description": "" }
            },
            "summary": {
                "total_nodes": 0, "total_edges": 0, "total_functions": 0, "total_modules": 0,
                "cycles_found": 0, "cycle_total_nodes": 0, "hotspot_count": 0,
                "hotspot_threshold_fan_in": 0, "high_coupling_modules": 0, "dead_functions": 0,
                "max_call_depth": 0, "tangle_index": 0.0, "avg_module_coupling": 0.0,
                "avg_fan_in": 0.0, "avg_fan_out": 0.0
            },
            "findings": [],
            "node_annotations": {},
            "module_annotations": {},
            "classifications": {},
            "boundary_violations": [],
            "integrity_status": { "engine_version": "test", "schema_caveats": [], "invariant_violations": false }
        });
        serde_json::from_value(json)
            .expect("test harness JSON should deserialize into HealthReport")
    }

    #[test]
    fn stamp_caveats_for_full_shape_adds_only_the_generic_caveat() {
        let mut r = bare_report();
        stamp_caveats(&mut r, RepoShape::Full);
        assert_eq!(
            r.integrity_status.schema_caveats,
            vec![CAVEAT_LAYER0_GIT_SIGNALS_V1.to_string()]
        );
    }

    #[test]
    fn stamp_caveats_for_shallow_adds_insufficient_history() {
        let mut r = bare_report();
        stamp_caveats(&mut r, RepoShape::Shallow { depth: Some(1) });
        assert!(r
            .integrity_status
            .schema_caveats
            .contains(&CAVEAT_LAYER0_GIT_SIGNALS_V1.to_string()));
        assert!(r
            .integrity_status
            .schema_caveats
            .contains(&CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1.to_string()));
    }

    #[test]
    fn stamp_caveats_for_non_git_adds_unsupported_vcs() {
        let mut r = bare_report();
        stamp_caveats(&mut r, RepoShape::NonGit);
        assert!(r
            .integrity_status
            .schema_caveats
            .contains(&CAVEAT_LAYER0_UNSUPPORTED_VCS_V1.to_string()));
    }

    #[test]
    fn push_once_is_idempotent() {
        let mut r = bare_report();
        stamp_caveats(&mut r, RepoShape::Full);
        stamp_caveats(&mut r, RepoShape::Full);
        let count = r
            .integrity_status
            .schema_caveats
            .iter()
            .filter(|c| c.as_str() == CAVEAT_LAYER0_GIT_SIGNALS_V1)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn report_round_trips_through_json_with_git_signals_attached() {
        let mut r = bare_report();
        let _ = &IntegrityStatus::default();
        r.git_signals = Some(GitSignalReport {
            repository_shape: RepoShape::Full,
            per_file: {
                let mut m = BTreeMap::new();
                m.insert(
                    std::path::PathBuf::from("a.rs"),
                    FileSignals {
                        change_frequency: 4,
                        distinct_authors: 2,
                        last_touched_days: Some(3),
                        ownership_dispersion: 0.5,
                        hotspot_score: 4.0,
                        confidence: graphengine_git_signals::Confidence::High,
                    },
                );
                m
            },
            co_change_clusters: Vec::new(),
            integrity_caveats: vec![CAVEAT_LAYER0_GIT_SIGNALS_V1.to_string()],
            commits_walked: 4,
            files_touched: 1,
        });
        let json = serde_json::to_string(&r).unwrap();
        let parsed: HealthReport = serde_json::from_str(&json).unwrap();
        assert!(parsed.git_signals.is_some());
        let gs = parsed.git_signals.unwrap();
        assert_eq!(gs.commits_walked, 4);
        assert_eq!(gs.files_touched, 1);
    }
}
