//! T8 §6 acceptance criteria #4 + #5 — named regression fixture.
//!
//! Exercises the `HealthReport`-level post-pass the `ge-analyze`
//! pipeline uses
//! (`coverage_attach::apply_extraction_coverage_downgrade_to_annotations`)
//! and the dual-metric companion recompute
//! (`coverage_attach::recompute_no_callers_confidence_split`).
//!
//! The mirror classifier-level tests live inside
//! `graphengine-analysis/src/health/dead_code_classifier/mod.rs`.
//! Keeping the two suites in lockstep guards against the same
//! drift as the T7 churn-downgrade pair.
//!
//! Guard rails this fixture encodes (T8 design §4.3, §7.1):
//!
//!   * A [`HealthReport`] with no attached coverage vector flows
//!     through unchanged — absent evidence is not negative evidence.
//!   * Coverage records with non-invalidating gaps (zero-count,
//!     or telemetry-only variants) do not downgrade.
//!   * A `NoCallers` verdict on a file whose coverage record
//!     carries an invalidating gap (R39 / R41) is clamped to
//!     `Medium` exactly once; the `CAVEAT_EXTRACTION_COVERAGE_GAPS_V1`
//!     caveat is stamped on the integrity status and never
//!     double-stamped.
//!   * The dual-metric companion counters
//!     (`no_callers_total`, `no_callers_high_confidence`) track the
//!     effect of downgrades. Re-running the recompute on an
//!     unchanged report is idempotent.

use std::path::PathBuf;

use graphengine_analysis::health::coverage_attach::{
    apply_extraction_coverage_downgrade_to_annotations, attach_extraction_coverage,
    recompute_no_callers_confidence_split,
};
use graphengine_analysis::health::report::{
    Confidence, DeadCodeReason, HealthReport, NodeAnnotation, RiskLevel,
    CAVEAT_EXTRACTION_COVERAGE_GAPS_V1,
};
use graphengine_parsing::application::ports::{
    CoverageConfidence, CoverageGap, FileExtractionCoverage,
};

/// Build a minimal [`HealthReport`] via JSON literal so the fixture
/// stays robust to additions to unrelated `HealthReport` fields.
/// Mirrors the T7 fixture's `bare_report` by design — the two are
/// intentionally decoupled helpers so a change to one suite cannot
/// silently mutate the other.
fn bare_report() -> HealthReport {
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
    serde_json::from_value(json).expect("test harness JSON should deserialize into HealthReport")
}

fn make_dead_annotation(file_path: &str) -> NodeAnnotation {
    NodeAnnotation {
        fqn: "A.foo".to_string(),
        display_name: "foo".to_string(),
        file_path: Some(file_path.to_string()),
        start_line: None,
        fan_in: 0,
        fan_out: 0,
        blast_radius: 0,
        depth_from_root: 0,
        information_flow_complexity: 0,
        is_hotspot: false,
        is_dead: true,
        dead_code_reason: Some(DeadCodeReason::NoCallers),
        dead_code_evidence: Some("fan_in=0".to_string()),
        dead_code_classifier: Some("generic".to_string()),
        dead_code_confidence: Some(Confidence::High),
        is_test: false,
        cycle_member: false,
        cycle_ids: vec![],
        hub_score: 0.0,
        cyclomatic_complexity: None,
        cognitive_complexity: None,
        loc: 10,
        inferred_layer: None,
        layer_label: None,
        risk_level: RiskLevel::Info,
    }
}

fn cov(path: &str, gaps: Vec<CoverageGap>) -> FileExtractionCoverage {
    FileExtractionCoverage {
        file_path: PathBuf::from(path),
        language: "apex".to_string(),
        walked_node_count: 100,
        unwalked_node_count: 0,
        coverage_gaps: gaps,
        confidence: CoverageConfidence::High,
    }
}

#[test]
fn no_coverage_is_no_op_and_companion_matches_total() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/A.cls"));

    let downgraded = apply_extraction_coverage_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 0);
    assert!(matches!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::High)
    ));

    let (total, high) = recompute_no_callers_confidence_split(&mut report);
    assert_eq!(total, 1);
    assert_eq!(
        high, 1,
        "with no coverage evidence, every NoCallers verdict stays High"
    );
    assert_eq!(report.metrics.dead_code.no_callers_total, Some(1));
    assert_eq!(report.metrics.dead_code.no_callers_high_confidence, Some(1));
}

#[test]
fn non_invalidating_coverage_does_not_downgrade() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/A.cls"));

    let records = vec![cov(
        "src/A.cls",
        vec![CoverageGap::ApexPropertyAccessor { count: 0 }],
    )];
    attach_extraction_coverage(&mut report, records);

    let downgraded = apply_extraction_coverage_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 0);
    assert!(matches!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::High)
    ));
    assert!(
        !report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_EXTRACTION_COVERAGE_GAPS_V1),
        "no invalidating gap → no caveat stamp",
    );

    let (total, high) = recompute_no_callers_confidence_split(&mut report);
    assert_eq!((total, high), (1, 1));
}

#[test]
fn invalidating_r39_gap_downgrades_and_stamps_caveat_once() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/A.cls"));

    let records = vec![cov(
        "src/A.cls",
        vec![CoverageGap::ApexPropertyAccessor { count: 1 }],
    )];
    attach_extraction_coverage(&mut report, records.clone());

    // Re-attaching must be idempotent on the caveat list even if
    // a caller invokes the attach step twice (e.g. a reprocessing
    // flow).
    attach_extraction_coverage(&mut report, records);
    let caveat_count = report
        .integrity_status
        .schema_caveats
        .iter()
        .filter(|c| **c == CAVEAT_EXTRACTION_COVERAGE_GAPS_V1)
        .count();
    assert_eq!(caveat_count, 1, "caveat must be stamped exactly once");

    let downgraded = apply_extraction_coverage_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 1);
    assert!(matches!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::Medium)
    ));

    // Second call is idempotent at the annotation level too — no
    // verdict gets clamped twice, and the counter stays stable.
    let second = apply_extraction_coverage_downgrade_to_annotations(&mut report);
    assert_eq!(second, 0);

    let (total, high) = recompute_no_callers_confidence_split(&mut report);
    assert_eq!(total, 1);
    assert_eq!(
        high, 0,
        "the only NoCallers verdict is now Medium → high_confidence drops to 0"
    );
    assert_eq!(report.metrics.dead_code.no_callers_total, Some(1));
    assert_eq!(report.metrics.dead_code.no_callers_high_confidence, Some(0));
}

#[test]
fn companion_ignores_non_dead_and_non_no_callers_verdicts() {
    let mut report = bare_report();

    // Three annotations: dead-NoCallers (counted), dead with a
    // different reason (not counted), and live (not counted).
    let mut dead_other_reason = make_dead_annotation("src/B.cls");
    dead_other_reason.dead_code_reason = Some(DeadCodeReason::VisibilityPrivateUnused);

    let mut live = make_dead_annotation("src/C.cls");
    live.is_dead = false;
    live.dead_code_reason = None;

    report.node_annotations.insert(
        "dead_no_callers".to_string(),
        make_dead_annotation("src/A.cls"),
    );
    report
        .node_annotations
        .insert("dead_other".to_string(), dead_other_reason);
    report.node_annotations.insert("live".to_string(), live);

    let (total, high) = recompute_no_callers_confidence_split(&mut report);
    assert_eq!(
        total, 1,
        "only the is_dead + NoCallers annotation counts toward total"
    );
    assert_eq!(high, 1);
}

#[test]
fn downgrade_only_hits_matching_file_path() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/A.cls"));
    report
        .node_annotations
        .insert("n2".to_string(), make_dead_annotation("src/B.cls"));

    // Only A.cls has an invalidating gap; B.cls is clean.
    let records = vec![
        cov(
            "src/A.cls",
            vec![CoverageGap::ApexMapLiteralInitializer { count: 2 }],
        ),
        cov("src/B.cls", vec![]),
    ];
    attach_extraction_coverage(&mut report, records);

    let downgraded = apply_extraction_coverage_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 1);
    assert!(matches!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::Medium)
    ));
    assert!(matches!(
        report.node_annotations["n2"].dead_code_confidence,
        Some(Confidence::High),
    ));

    let (total, high) = recompute_no_callers_confidence_split(&mut report);
    assert_eq!(total, 2);
    assert_eq!(high, 1);
}
