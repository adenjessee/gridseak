//! T7 §6.2 acceptance criterion #5 — named regression fixture.
//!
//! Proves the load-bearing behavioural claim of T7: when a node is
//! classified as dead-code at `Confidence::High`, but the
//! underlying file shows recent churn in a full-shape
//! [`GitSignalReport`], the confidence is downgraded to `Medium`.
//! Without the git signal, the same verdict stays at `High`. The
//! test exercises the `HealthReport`-level post-pass
//! (`apply_dead_code_churn_downgrade_to_annotations`) because that
//! is the operational seam the `ge-analyze` pipeline uses.
//!
//! The mirror unit test inside
//! `graphengine-analysis/src/health/dead_code_classifier/mod.rs`
//! covers the classifier-level function
//! (`apply_git_signal_churn_downgrade`) on raw verdicts. Both
//! functions share the same predicate; keeping them in lockstep
//! with two separate tests guards against drift.

use std::collections::BTreeMap;
use std::path::PathBuf;

use graphengine_analysis::health::git_signals_attach::apply_dead_code_churn_downgrade_to_annotations;
use graphengine_analysis::health::report::{
    Confidence, DeadCodeReason, HealthReport, NodeAnnotation, RiskLevel,
};
use graphengine_git_signals::{
    Confidence as GitConfidence, FileSignals, GitSignalReport, RepoShape,
    CAVEAT_LAYER0_GIT_SIGNALS_V1,
};

/// Build a minimal [`HealthReport`] via JSON literal so the fixture
/// stays robust to additions to unrelated `HealthReport` fields.
/// The per-field dance isn't worth hand-writing here — every field
/// defaults to `null` / `0` / empty, which is exactly what the
/// downgrade function expects to see.
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

/// Dead function on `src/recent.rs`. Initially stamped as
/// `dead_code_confidence = High`, which mirrors what the classifier
/// pipeline produces for a verdict on a never-called private fn.
fn make_dead_annotation(file_path: &str) -> NodeAnnotation {
    NodeAnnotation {
        fqn: "recent::helper".to_string(),
        display_name: "helper".to_string(),
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

fn make_git_report(path: &str, last_touched_days: u32, conf: GitConfidence) -> GitSignalReport {
    let mut per_file = BTreeMap::new();
    per_file.insert(
        PathBuf::from(path),
        FileSignals {
            change_frequency: 4,
            distinct_authors: 2,
            last_touched_days: Some(last_touched_days),
            ownership_dispersion: 0.5,
            hotspot_score: 6.0,
            confidence: conf,
        },
    );
    GitSignalReport {
        repository_shape: RepoShape::Full,
        per_file,
        co_change_clusters: Vec::new(),
        integrity_caveats: vec![CAVEAT_LAYER0_GIT_SIGNALS_V1.to_string()],
        commits_walked: 4,
        files_touched: 1,
    }
}

#[test]
fn recent_high_churn_downgrades_dead_code_confidence_from_high_to_medium() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/recent.rs"));
    report.git_signals = Some(make_git_report("src/recent.rs", 5, GitConfidence::High));

    let downgraded = apply_dead_code_churn_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 1, "exactly one annotation should be downgraded");
    assert_eq!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::Medium),
        "recent high-confidence churn must downgrade dead-code confidence High -> Medium"
    );
}

#[test]
fn absence_of_git_signals_leaves_dead_code_confidence_high() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/recent.rs"));
    // No git_signals attached. This is the "absent, not negative"
    // contract — without measurement we must not downgrade.
    assert!(report.git_signals.is_none());

    let downgraded = apply_dead_code_churn_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 0);
    assert_eq!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::High),
        "with no git signal, dead-code confidence must remain at High"
    );
}

#[test]
fn stale_file_does_not_trigger_downgrade_even_with_high_confidence_signal() {
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/stale.rs"));
    // 90 days out - well outside ACTIVE_RECENT_MAX_DAYS (30).
    report.git_signals = Some(make_git_report("src/stale.rs", 90, GitConfidence::High));

    let downgraded = apply_dead_code_churn_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 0);
    assert_eq!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::High),
    );
}

#[test]
fn low_confidence_signal_never_triggers_downgrade_on_recent_file() {
    // This is the shallow-clone guard at the classifier layer:
    // even if the file looks recently touched, the git-signal
    // extractor reports `Confidence::Low` on non-Full repo shapes,
    // and we MUST NOT act on that evidence.
    let mut report = bare_report();
    report
        .node_annotations
        .insert("n1".to_string(), make_dead_annotation("src/shallow.rs"));
    report.git_signals = Some(make_git_report("src/shallow.rs", 3, GitConfidence::Low));

    let downgraded = apply_dead_code_churn_downgrade_to_annotations(&mut report);
    assert_eq!(
        downgraded, 0,
        "shallow-clone guard: low-confidence signals MUST NOT downgrade verdicts"
    );
    assert_eq!(
        report.node_annotations["n1"].dead_code_confidence,
        Some(Confidence::High),
    );
}

#[test]
fn non_dead_annotations_are_never_touched() {
    let mut report = bare_report();
    let mut live = make_dead_annotation("src/recent.rs");
    live.is_dead = false;
    live.dead_code_confidence = None;
    report.node_annotations.insert("live".to_string(), live);
    report.git_signals = Some(make_git_report("src/recent.rs", 5, GitConfidence::High));

    let downgraded = apply_dead_code_churn_downgrade_to_annotations(&mut report);
    assert_eq!(downgraded, 0);
    assert!(report.node_annotations["live"]
        .dead_code_confidence
        .is_none());
}
