//! T7 §6.2 acceptance criterion #6 — shallow-clone end-to-end test.
//!
//! The T7 design doc (`docs/workstreams/universal-fidelity/tasks/T7-layer0-git-signals.md` §6.2)
//! specifies the test file at
//! `graphengine-diagnostic/tests/npsp_layer0_shallow_caveat.rs`.
//! The test is located here instead because the concrete seam it
//! exercises (`git_signals_attach::attach_git_signals`) lives in
//! `graphengine-analysis`, and the test body uses nothing from
//! `graphengine-diagnostic` beyond the shared
//! `HealthReport` type. Deviation recorded in `UF-FU-013` for
//! paper trail.
//!
//! What the test proves end-to-end:
//! - Building a fake shallow-clone fixture (a working tree with a
//!   forged `.git/shallow` file).
//! - Calling `attach_git_signals` against it.
//! - Asserting the attached report carries
//!   `CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1` and that every
//!   `FileSignals::confidence` is `Confidence::Low`.
//!
//! The "NPSP" naming in the doc is aspirational — the real NPSP
//! canary is a 1-commit shallow clone. This fixture reproduces
//! the *shape* of that canary (forged shallow file over a real
//! git repo) so the test can run without pulling NPSP bytes into
//! the workspace.

use std::fs;
use std::process::Command;

use graphengine_analysis::health::git_signals_attach::{self, GitSignalAttachOutcome};
use graphengine_analysis::health::report::{
    HealthReport, CAVEAT_LAYER0_GIT_SIGNALS_V1, CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1,
};
use graphengine_git_signals::{Confidence, HistoryWindow, RepoShape};

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

fn git_init_and_commit(root: &std::path::Path) {
    let run = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git binary must be available on PATH for tests");
        assert!(status.success(), "git {:?} must succeed", args);
    };
    run(&["init", "--quiet", "--initial-branch", "main"]);
    run(&["config", "commit.gpgsign", "false"]);
    run(&["config", "user.email", "alice@example.com"]);
    run(&["config", "user.name", "Alice"]);

    fs::write(root.join("a.rs"), "fn main() {}\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .status();
    let _ = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "seed",
        ])
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", "2026-04-10 12:00:00 +0000")
        .env("GIT_COMMITTER_DATE", "2026-04-10 12:00:00 +0000")
        .env("GIT_COMMITTER_EMAIL", "alice@example.com")
        .env("GIT_COMMITTER_NAME", "Alice")
        .status();
}

#[test]
fn shallow_clone_fixture_stamps_insufficient_history_caveat_and_low_confidence() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    git_init_and_commit(&root);

    // Forge shallow-file to mark this repo as shallow. The
    // shallow-clone guard in graphengine-git-signals must downgrade
    // every per-file signal to Confidence::Low regardless of the
    // numeric values the extractor computes.
    fs::write(
        root.join(".git").join("shallow"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .unwrap();

    let mut report = bare_report();
    let outcome = git_signals_attach::attach_git_signals(
        &mut report,
        root.as_path(),
        &HistoryWindow::default_ci(),
    );

    match outcome {
        GitSignalAttachOutcome::Attached { shape } => {
            assert_eq!(shape, RepoShape::Shallow { depth: Some(1) });
        }
        GitSignalAttachOutcome::Skipped(err) => {
            panic!("attach_git_signals must succeed on a forged-shallow repo, got {err}");
        }
    }

    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_LAYER0_GIT_SIGNALS_V1),
        "generic Layer-0 caveat must be stamped on the report, got {:?}",
        report.integrity_status.schema_caveats,
    );
    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1),
        "shallow-clone caveat must be stamped on the report, got {:?}",
        report.integrity_status.schema_caveats,
    );

    let git = report
        .git_signals
        .as_ref()
        .expect("git_signals block must be populated after attach");

    assert_eq!(git.repository_shape, RepoShape::Shallow { depth: Some(1) });
    assert!(
        !git.per_file.is_empty(),
        "per_file must still contain measured signals on shallow repos — only confidence downgrades"
    );
    for (path, signals) in &git.per_file {
        assert_eq!(
            signals.confidence,
            Confidence::Low,
            "shallow-clone guard violation: {:?} has Confidence {:?}",
            path,
            signals.confidence
        );
    }
}
