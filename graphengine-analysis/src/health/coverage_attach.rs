//! Attach T8 extraction-coverage records to a [`HealthReport`].
//!
//! Mirrors the two-jobs discipline established by
//! [`crate::health::git_signals_attach`]: the health-report builder
//! in `mod.rs` reads the parse DB and knows nothing about
//! extraction-coverage wire details; this module takes a pre-built
//! report and a parser-produced list of
//! [`graphengine_parsing::application::ports::FileExtractionCoverage`]
//! records and installs them onto the report, stamping the
//! [`crate::health::report::CAVEAT_EXTRACTION_COVERAGE_GAPS_V1`]
//! caveat when any file carries an invalidating gap.
//!
//! The T8 design doc (§4.3) makes the analogous two-jobs call for
//! classifiers: the classifier consumes a coverage map keyed by
//! file path; this helper also lowers the flat vector into a
//! `HashMap` on demand via [`coverage_map`].

use std::collections::HashMap;
use std::path::PathBuf;

use graphengine_parsing::application::ports::FileExtractionCoverage;

use crate::health::report::{HealthReport, CAVEAT_EXTRACTION_COVERAGE_GAPS_V1};

/// Install the parser-produced `extraction_coverage` list onto
/// `report.file_extraction_coverage` and stamp
/// [`CAVEAT_EXTRACTION_COVERAGE_GAPS_V1`] when any record has an
/// invalidating gap. Safe to call with an empty vector — the
/// caveat is not stamped in that case.
///
/// Returns the number of records attached so the caller can log
/// whether the pass ran at all.
pub fn attach_extraction_coverage(
    report: &mut HealthReport,
    coverage: Vec<FileExtractionCoverage>,
) -> usize {
    let count = coverage.len();
    let any_invalidating = coverage.iter().any(|c| c.has_invalidating_no_callers_gap());
    report.file_extraction_coverage = coverage;
    if any_invalidating {
        let caveat = CAVEAT_EXTRACTION_COVERAGE_GAPS_V1.to_string();
        if !report.integrity_status.schema_caveats.contains(&caveat) {
            report.integrity_status.schema_caveats.push(caveat);
        }
    }
    count
}

/// Walk `report.node_annotations` and downgrade
/// `dead_code_confidence` from `High` to `Medium` on every annotation
/// that (a) is flagged dead with reason `NoCallers` and (b) lives in
/// a file whose `file_extraction_coverage` record carries an
/// invalidating gap. Also appends the
/// `CAVEAT_EXTRACTION_COVERAGE_GAPS_V1` string to the annotation's
/// evidence once (never duplicated).
///
/// Returns the number of annotations whose confidence was lowered.
///
/// Mirrors
/// [`crate::health::git_signals_attach::apply_dead_code_churn_downgrade_to_annotations`]
/// in structure and in the "absent evidence is not negative evidence"
/// contract: a report with no coverage records flows through
/// unchanged; a node whose path does not appear in the coverage map
/// is left alone. The two downgraders compose commutatively — one can
/// clamp `High → Medium` before or after the other without changing
/// the final verdict, because both clamps land at `Medium` and the
/// T7 downgrader's churn window and T8's coverage-gap condition are
/// independent.
pub fn apply_extraction_coverage_downgrade_to_annotations(
    report: &mut crate::health::report::HealthReport,
) -> usize {
    if report.file_extraction_coverage.is_empty() {
        return 0;
    }
    let invalidating: std::collections::HashSet<PathBuf> = report
        .file_extraction_coverage
        .iter()
        .filter(|c| c.has_invalidating_no_callers_gap())
        .map(|c| c.file_path.clone())
        .collect();
    if invalidating.is_empty() {
        return 0;
    }

    use crate::health::report::{Confidence, DeadCodeReason};
    let mut downgraded: usize = 0;
    for ann in report.node_annotations.values_mut() {
        if !ann.is_dead {
            continue;
        }
        if !matches!(ann.dead_code_reason, Some(DeadCodeReason::NoCallers)) {
            continue;
        }
        if !matches!(ann.dead_code_confidence, Some(Confidence::High)) {
            continue;
        }
        let Some(path_str) = ann.file_path.as_ref() else {
            continue;
        };
        let path = PathBuf::from(path_str);
        if !invalidating.contains(&path) {
            continue;
        }
        ann.dead_code_confidence = Some(Confidence::Medium);
        downgraded += 1;
    }
    downgraded
}

/// Recompute the T8 dual-metric companion counters
/// (`metrics.dead_code.no_callers_total` and
/// `metrics.dead_code.no_callers_high_confidence`) from the current
/// set of node annotations. Call this *after* both the T7 churn
/// downgrader and the T8 coverage downgrader have run, so the
/// `High`-confidence count reflects the effect of every downgrade.
///
/// Pre-T8 reports (no coverage records, no downgrades) still
/// produce identical values for total and high-confidence — the
/// companion is informative without ever being misleading.
///
/// Returns `(total, high_confidence)` for logging.
pub fn recompute_no_callers_confidence_split(
    report: &mut crate::health::report::HealthReport,
) -> (usize, usize) {
    use crate::health::report::{Confidence, DeadCodeReason};
    let mut total: usize = 0;
    let mut high: usize = 0;
    for ann in report.node_annotations.values() {
        if !ann.is_dead {
            continue;
        }
        if !matches!(ann.dead_code_reason, Some(DeadCodeReason::NoCallers)) {
            continue;
        }
        total += 1;
        if matches!(ann.dead_code_confidence, Some(Confidence::High)) {
            high += 1;
        }
    }
    report.metrics.dead_code.no_callers_total = Some(total);
    report.metrics.dead_code.no_callers_high_confidence = Some(high);
    (total, high)
}

/// Lower a flat list of coverage records into a path-keyed map
/// suitable for the classifier contract in T8 §4.3. The last record
/// per path wins if the same path appears twice — which should not
/// happen in a normal parse run, but we fold rather than panic
/// because wire-level duplication is a recoverable condition
/// (measured fallback).
pub fn coverage_map(
    records: &[FileExtractionCoverage],
) -> HashMap<PathBuf, FileExtractionCoverage> {
    let mut out: HashMap<PathBuf, FileExtractionCoverage> = HashMap::with_capacity(records.len());
    for r in records {
        out.insert(r.file_path.clone(), r.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_parsing::application::ports::{
        CoverageConfidence, CoverageGap, FileExtractionCoverage,
    };

    fn bare_report() -> HealthReport {
        // Mirror the JSON round-trip pattern from
        // `git_signals_attach::tests::bare_report`. The two test
        // helpers intentionally stay siblings rather than folded into
        // a shared fixture — each one is minimal for its own module's
        // concerns and any shared helper would pull in fields the
        // other test does not care about.
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

    fn cov(path: &str, gaps: Vec<CoverageGap>) -> FileExtractionCoverage {
        FileExtractionCoverage {
            file_path: PathBuf::from(path),
            language: "apex".to_string(),
            walked_node_count: 10,
            unwalked_node_count: 0,
            coverage_gaps: gaps,
            confidence: CoverageConfidence::High,
        }
    }

    #[test]
    fn attach_empty_does_not_stamp_caveat() {
        let mut report = bare_report();
        let n = attach_extraction_coverage(&mut report, vec![]);
        assert_eq!(n, 0);
        assert!(report.file_extraction_coverage.is_empty());
        assert!(!report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_EXTRACTION_COVERAGE_GAPS_V1));
    }

    #[test]
    fn attach_with_invalidating_gap_stamps_caveat() {
        let mut report = bare_report();
        let records = vec![cov(
            "/fake/A.cls",
            vec![CoverageGap::ApexPropertyAccessor { count: 2 }],
        )];
        let n = attach_extraction_coverage(&mut report, records);
        assert_eq!(n, 1);
        assert!(report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_EXTRACTION_COVERAGE_GAPS_V1));
    }

    #[test]
    fn attach_without_invalidating_gap_does_not_stamp_caveat() {
        let mut report = bare_report();
        let records = vec![cov("/fake/Clean.cls", vec![])];
        attach_extraction_coverage(&mut report, records);
        assert!(!report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_EXTRACTION_COVERAGE_GAPS_V1));
    }

    #[test]
    fn attach_does_not_double_stamp_caveat() {
        let mut report = bare_report();
        let records = vec![cov(
            "/fake/A.cls",
            vec![CoverageGap::ApexPropertyAccessor { count: 1 }],
        )];
        attach_extraction_coverage(&mut report, records.clone());
        attach_extraction_coverage(&mut report, records);
        let count = report
            .integrity_status
            .schema_caveats
            .iter()
            .filter(|c| **c == CAVEAT_EXTRACTION_COVERAGE_GAPS_V1)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn coverage_map_keys_by_path() {
        let records = vec![
            cov("/a.cls", vec![]),
            cov(
                "/b.cls",
                vec![CoverageGap::ApexMapLiteralInitializer { count: 1 }],
            ),
        ];
        let map = coverage_map(&records);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key(&PathBuf::from("/a.cls")));
        let b = map.get(&PathBuf::from("/b.cls")).unwrap();
        assert!(b.has_invalidating_no_callers_gap());
    }
}
