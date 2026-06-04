//! S2-β developer-journey routing regression (Acts 2–4 from segmented-analysis plan).

mod common;

use common::{ensure_engine_binaries, pipeline_config};
use gridseak_engine_runner::run_pipeline;
use std::fs;
use uuid::Uuid;

/// Act 3: plain-language file impact routes to file blast radius tool id (CLI router).
#[test]
fn act3_router_maps_file_impact_question() {
    use gridseak_cli::intent_router::{route, RouteInput, RoutedTool};
    let q = "what breaks if I change gridseak-cli/src/main.rs";
    let decision = route(RouteInput {
        question: q,
        file_hint: None,
        symbol_hint: None,
    });
    assert_eq!(
        decision.tool,
        RoutedTool::GridseakGraphFileBlastRadius,
        "file-impact question should route to file blast radius; got {:?}",
        decision.tool
    );
}

/// Act 2 + 4: warm pipeline produces graph DB + report; analysis trust tests live in s2_incremental_analysis.
#[tokio::test]
async fn act2_warm_scan_produces_artifacts_for_graph_tools() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let persistent = scratch.path().join("parse.sqlite");

    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(scratch.path(), scan_a, vec!["rust".into()]);
    cfg_a.persistent_parse_db = Some(persistent.clone());
    cfg_a.incremental = true;
    let out_a = run_pipeline(cfg_a).await.expect("cold scan");
    assert!(
        out_a.report.analysis_duration_ms > 0,
        "cold scan must run full analysis"
    );

    let scan_b = Uuid::new_v4();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.persistent_parse_db = Some(persistent);
    cfg_b.incremental = true;
    let out_b = run_pipeline(cfg_b).await.expect("warm scan");
    assert!(
        out_b.report.analysis_duration_ms == 0,
        "zero-delta warm should reuse analysis cache"
    );
}

/// Act 2 (dirty file): one-file edit must not reuse cached analysis report.
#[tokio::test]
async fn act2_dirty_file_forces_full_analysis() {
    use graphengine_analysis::health::report::CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1;

    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = scratch.path().join("repo");
    std::process::Command::new("cp")
        .args([
            "-R",
            common::fixture_root().to_str().unwrap(),
            repo.to_str().unwrap(),
        ])
        .status()
        .expect("cp fixture");

    let persistent = scratch.path().join("parse.sqlite");
    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(scratch.path(), scan_a, vec!["rust".into()]);
    cfg_a.root = repo.clone();
    cfg_a.persistent_parse_db = Some(persistent.clone());
    cfg_a.incremental = true;
    run_pipeline(cfg_a).await.expect("cold");

    let lib = repo.join("src").join("lib.rs");
    let mut content = fs::read_to_string(&lib).unwrap();
    content.push_str("\n// routing-regression touch\n");
    fs::write(&lib, content).unwrap();

    let scan_b = Uuid::new_v4();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.root = repo;
    cfg_b.persistent_parse_db = Some(persistent);
    cfg_b.incremental = true;
    let out = run_pipeline(cfg_b).await.expect("warm dirty");

    assert!(out.report.analysis_duration_ms > 0);
    assert!(!out
        .report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1));
}
