//! S2 v1 + S2-γ: incremental analysis trust ladder integration tests.

mod common;

use common::{ensure_engine_binaries, fixture_root, pipeline_config};
use graphengine_analysis::health::report::{
    CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1,
    CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1,
};
use graphengine_parsing::infrastructure::storage::parse_meta_store::{
    compute_structure_fingerprint, read_incremental_scan_stats, write_incremental_scan_stats,
};
use gridseak_engine_runner::{
    progress::ProgressEvent, progress::ProgressSink, run_pipeline, Stage,
};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uuid::Uuid;

fn copy_fixture_repo(scratch: &Path) -> PathBuf {
    let repo = scratch.join("repo");
    std::process::Command::new("cp")
        .args([
            "-R",
            fixture_root().to_str().unwrap(),
            repo.to_str().unwrap(),
        ])
        .status()
        .expect("copy fixture");
    inflate_fixture_file_count(&repo, 120);
    repo
}

/// polyglot-tiny alone (~2 files) makes a one-file edit exceed SEGMENTED_RATIO_THRESHOLD.
fn inflate_fixture_file_count(repo: &Path, extra_files: usize) {
    let src = repo.join("src");
    std::fs::create_dir_all(&src).expect("src dir");
    for i in 0..extra_files {
        let path = src.join(format!("pad_{i:03}.rs"));
        if !path.exists() {
            std::fs::write(&path, format!("pub fn pad_{i}() -> i32 {{ {i} }}\n"))
                .expect("pad file");
        }
    }
}

fn reset_parse_stats_zero_delta(persistent: &Path) {
    let conn = Connection::open(persistent).expect("open parse db");
    let Some(mut stats) = read_incremental_scan_stats(&conn).expect("read stats") else {
        return;
    };
    stats.reparsed = 0;
    stats.removed = 0;
    stats.changed_paths.clear();
    stats.removed_paths.clear();
    write_incremental_scan_stats(&conn, &stats).expect("write stats");
}

async fn cold_scan(repo: &Path, persistent: &Path) {
    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(repo.parent().unwrap(), scan_a, vec!["rust".into()]);
    cfg_a.root = repo.to_path_buf();
    cfg_a.persistent_parse_db = Some(persistent.to_path_buf());
    cfg_a.incremental = true;
    run_pipeline(cfg_a)
        .await
        .expect("cold run against persistent DB succeeds");
}

async fn warm_scan(repo: &Path, persistent: &Path) -> gridseak_engine_runner::RunPipelineOutput {
    let scan_b = Uuid::new_v4();
    let mut cfg_b = pipeline_config(repo.parent().unwrap(), scan_b, vec!["rust".into()]);
    cfg_b.root = repo.to_path_buf();
    cfg_b.persistent_parse_db = Some(persistent.to_path_buf());
    cfg_b.incremental = true;
    run_pipeline(cfg_b)
        .await
        .expect("warm run against persistent DB succeeds")
}

fn structure_fp_at(persistent: &Path) -> String {
    let conn = Connection::open(persistent).expect("open parse db");
    compute_structure_fingerprint(&conn).expect("structure fp")
}

fn segment_cache_exists(persistent: &Path, segment: &str) -> bool {
    let conn = Connection::open(persistent).expect("open parse db");
    conn.query_row(
        "SELECT COUNT(*) FROM analysis_segment_cache WHERE segment_id = ?1",
        [segment],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

#[derive(Clone)]
struct StageTimingSink {
    analyze_ms: Arc<Mutex<Option<u64>>>,
}

impl StageTimingSink {
    fn new() -> Self {
        Self {
            analyze_ms: Arc::new(Mutex::new(None)),
        }
    }

    fn analyzing_elapsed_ms(&self) -> Option<u64> {
        *self.analyze_ms.lock().unwrap()
    }
}

impl ProgressSink for StageTimingSink {
    fn on_event(&mut self, event: ProgressEvent) {
        if let ProgressEvent::StageFinished {
            stage: Stage::Analyzing,
            elapsed_ms,
            ..
        } = event
        {
            *self.analyze_ms.lock().unwrap() = Some(elapsed_ms);
        }
    }
}

#[tokio::test]
async fn warm_zero_delta_rescan_uses_analysis_fast_path() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let persistent = scratch.path().join("parse.sqlite");

    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(scratch.path(), scan_a, vec!["rust".into()]);
    cfg_a.persistent_parse_db = Some(persistent.clone());
    cfg_a.incremental = true;
    run_pipeline(cfg_a)
        .await
        .expect("cold run against persistent DB succeeds");

    let scan_b = Uuid::new_v4();
    let sink = StageTimingSink::new();
    let warm_start = Instant::now();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.persistent_parse_db = Some(persistent.clone());
    cfg_b.incremental = true;
    cfg_b.progress = Box::new(sink.clone());

    let out = run_pipeline(cfg_b)
        .await
        .expect("warm run against persistent DB succeeds");
    let warm_total_ms = warm_start.elapsed().as_millis() as u64;

    let analyze_ms = sink
        .analyzing_elapsed_ms()
        .expect("analyzing stage must finish");

    assert!(
        out.report.analysis_duration_ms == 0,
        "fast path must reuse cached report (analysis_duration_ms=0); got {}",
        out.report.analysis_duration_ms
    );
    assert!(
        analyze_ms < 30_000,
        "warm analyzing stage should finish in seconds, not minutes; got {analyze_ms}ms (total warm {warm_total_ms}ms)"
    );
}

#[tokio::test]
async fn warm_one_file_delta_does_not_reuse_cached_report() {
    use graphengine_analysis::health::report::{
        CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1, CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1,
    };

    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;
    reset_parse_stats_zero_delta(&persistent);

    let target = repo.join("src").join("lib.rs");
    let mut content = std::fs::read_to_string(&target).expect("read lib.rs");
    content.push_str("\n// s2-gamma warm delta touch\n");
    std::fs::write(&target, content).expect("touch lib.rs");

    let scan_b = Uuid::new_v4();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.root = repo;
    cfg_b.persistent_parse_db = Some(persistent);
    cfg_b.incremental = true;

    let out = run_pipeline(cfg_b)
        .await
        .expect("warm one-file delta run succeeds");

    assert!(
        out.report.analysis_duration_ms > 0,
        "one-file delta must run analysis work, not whole-report reuse (analysis_duration_ms=0)"
    );
    assert!(
        !out.report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1),
        "one-file delta must not stamp incremental_analysis_reused_v1"
    );

    let l1_merge = out
        .report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1);
    let trust_l1 = out
        .report
        .analysis_provenance
        .as_ref()
        .map(|p| p.trust_level == "L1")
        .unwrap_or(false);
    assert!(
        l1_merge || !trust_l1,
        "L1 fast merge must stamp incremental_analysis_segments_merged_v1 when trust_level=L1"
    );
    if l1_merge {
        assert!(
            out.report
                .analysis_provenance
                .as_ref()
                .and_then(|p| p.query_trust_note.as_ref())
                .is_some(),
            "L1 merge must include query-scoped trust note for changed_paths"
        );
    }
}

#[tokio::test]
#[ignore = "dogfood timing budget; run locally via ./scripts/dogfood-profile.sh"]
async fn dogfood_one_file_warm_analyze_under_120s() {
    ensure_engine_binaries();

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(&repo_root, scan_a, vec!["rust".into()]);
    cfg_a.root = repo_root.clone();
    cfg_a.incremental = true;
    run_pipeline(cfg_a).await.expect("dogfood cold scan");

    let touch = repo_root.join("graphengine-analysis/src/health/pipeline/mod.rs");
    let mut content = std::fs::read_to_string(&touch).unwrap();
    let original = content.clone();
    content.push_str("\n// dogfood-timing-touch\n");
    std::fs::write(&touch, &content).unwrap();

    let sink = StageTimingSink::new();
    let scan_b = Uuid::new_v4();
    let mut cfg_b = pipeline_config(&repo_root, scan_b, vec!["rust".into()]);
    cfg_b.root = repo_root.clone();
    cfg_b.incremental = true;
    cfg_b.progress = Box::new(sink.clone());
    let out = run_pipeline(cfg_b)
        .await
        .expect("dogfood warm one-file scan");

    let analyze_ms = sink
        .analyzing_elapsed_ms()
        .unwrap_or(out.report.analysis_duration_ms);
    assert!(
        analyze_ms <= 120_000,
        "dogfood one-file warm analyze should finish within 120s; got {analyze_ms}ms"
    );

    let _ = std::fs::write(&touch, original);
}

#[tokio::test]
async fn warm_cosmetic_triggers_l1_merge() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;

    let target = repo.join("src").join("lib.rs");
    let mut content = std::fs::read_to_string(&target).expect("read lib.rs");
    content.push_str("\n// E1 cosmetic-only touch\n");
    std::fs::write(&target, content).expect("touch lib.rs");

    let out = warm_scan(&repo, &persistent).await;

    let provenance = out.report.analysis_provenance.as_ref().expect("provenance");
    assert_eq!(provenance.trust_level, "L1");
    assert!(
        out.report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1),
        "E1 cosmetic edit must stamp segment-merge caveat"
    );
    assert!(
        provenance.query_trust_note.is_some(),
        "L1 merge must include query-scoped trust note"
    );
    assert!(
        provenance.segments_reused.iter().any(|s| s == "Cycles"),
        "L1 must reuse wiring segments; got {:?}",
        provenance.segments_reused
    );
}

#[tokio::test]
async fn warm_one_file_wiring_delta_triggers_l2() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;

    let target = repo.join("src").join("lib.rs");
    let mut content = std::fs::read_to_string(&target).expect("read lib.rs");
    content = content.replace("for _ in 0..n {", "let _ = add(0, 0);\n    for _ in 0..n {");
    std::fs::write(&target, content).expect("wire new call");

    let out = warm_scan(&repo, &persistent).await;

    let provenance = out.report.analysis_provenance.as_ref().expect("provenance");
    assert_eq!(provenance.trust_level, "L2");
    assert!(provenance.structure_changed);
    assert!(
        out.report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1),
        "E3 wiring edit must stamp structure-changed caveat"
    );
    assert!(
        provenance.segments_rerun.iter().any(|s| s == "FanMetrics"),
        "L2 must rerun FanMetrics; got {:?}",
        provenance.segments_rerun
    );
    assert!(
        provenance.segments_rerun.iter().any(|s| s == "BlastRadius"),
        "L2 must rerun BlastRadius; got {:?}",
        provenance.segments_rerun
    );
    assert!(
        provenance
            .segments_reused
            .iter()
            .any(|s| s == "AuxiliaryMetrics"),
        "L2 should reuse AuxiliaryMetrics; got {:?}",
        provenance.segments_reused
    );
}

#[tokio::test]
async fn structure_fp_stable_e1_e2_differ_e3() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;
    let fp_cold = structure_fp_at(&persistent);

    let target = repo.join("src").join("lib.rs");

    let mut e1 = std::fs::read_to_string(&target).unwrap();
    e1.push_str("\n// E1 comment\n");
    std::fs::write(&target, &e1).unwrap();
    warm_scan(&repo, &persistent).await;
    let fp_e1 = structure_fp_at(&persistent);

    let mut e2 = std::fs::read_to_string(&target).unwrap();
    e2 = e2.replace("a + b", "a + b + 0");
    std::fs::write(&target, &e2).unwrap();
    warm_scan(&repo, &persistent).await;
    let fp_e2 = structure_fp_at(&persistent);

    let mut e3 = std::fs::read_to_string(&target).unwrap();
    e3 = e3.replace(
        "self.count += 1;",
        "let _ = add(1, 1);\n        self.count += 1;",
    );
    std::fs::write(&target, &e3).unwrap();
    warm_scan(&repo, &persistent).await;
    let fp_e3 = structure_fp_at(&persistent);

    assert_eq!(fp_cold, fp_e1, "E1 cosmetic must not change topology FP");
    assert_eq!(fp_e1, fp_e2, "E2 body-only must not change topology FP");
    assert_ne!(fp_e2, fp_e3, "E3 wiring must change topology FP");
}

#[tokio::test]
async fn l1_merge_uses_segment_cache() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;
    assert!(
        segment_cache_exists(&persistent, "HealthScore"),
        "cold scan must seed HealthScore segment cache"
    );

    let target = repo.join("src").join("lib.rs");
    let mut content = std::fs::read_to_string(&target).unwrap();
    content.push_str("\n// cache-hit cosmetic\n");
    std::fs::write(&target, content).unwrap();

    let out = warm_scan(&repo, &persistent).await;
    assert_eq!(
        out.report
            .analysis_provenance
            .as_ref()
            .map(|p| p.trust_level.as_str()),
        Some("L1")
    );
    assert!(out
        .report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1));
}

#[tokio::test]
async fn analyze_background_once_seeds_segment_cache() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;

    graphengine_analysis::health::pipeline::run_analysis_pipeline(
        persistent.to_str().unwrap(),
        None,
        None,
        None,
        None,
        true,
    )
    .expect("background analyze pass");

    assert!(
        segment_cache_exists(&persistent, "Cycles"),
        "background analyze must write segment cache rows"
    );
}

#[tokio::test]
async fn gridseak_status_after_edit_predicts_l1() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch");
    let repo = copy_fixture_repo(scratch.path());
    let persistent = scratch.path().join("parse.sqlite");

    cold_scan(&repo, &persistent).await;
    reset_parse_stats_zero_delta(&persistent);

    let target = repo.join("src").join("lib.rs");
    let mut content = std::fs::read_to_string(&target).unwrap();
    content.push_str("\n// status-plan cosmetic pre-rescan\n");
    std::fs::write(&target, &content).unwrap();

    let conn = Connection::open(&persistent).unwrap();
    let plan_dirty = graphengine_analysis::health::pipeline::predict_incremental_plan(
        &conn,
        Some(&["src/lib.rs".into()]),
    )
    .unwrap();
    assert_eq!(plan_dirty.predicted_trust_level, "L1");
    assert_eq!(plan_dirty.structure_fp_match, None);
    assert_eq!(plan_dirty.stale_reason.as_deref(), Some("rescan_required"));
    assert!(
        plan_dirty.segments_to_reuse.iter().any(|s| s == "Cycles"),
        "L1 plan must reuse Cycles per scope.rs"
    );
    assert!(
        plan_dirty
            .segments_to_rerun
            .iter()
            .any(|s| s == "Complexity"),
        "L1 plan must rerun Complexity per scope.rs"
    );
}
