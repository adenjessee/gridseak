//! S1-ε integration: persistent parse DB + incremental cache reuse.
//!
//! Proves the runner wiring that makes CLI-level scan-over-scan
//! speedup possible:
//!
//! 1. `persistent_parse_db` suppresses `--clear` so the `file_cache`
//!    table survives between runs.
//! 2. A second run against the same DB reuses cached file slices
//!    (`CacheStats.cached > 0`, `reparsed == 0`).
//! 3. `--no-incremental` forces a full reparse even when the cache
//!    is warm (`disabled: true`, `reparsed == total`).

mod common;

use common::{ensure_engine_binaries, pipeline_config};
use gridseak_engine_runner::{
    progress::{ProgressEvent, ProgressSink},
    run_pipeline, EngineEvent, Stage,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
struct RecordingSink {
    events: Arc<Mutex<Vec<ProgressEvent>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn stage_finished_ms(&self, stage: Stage, language: Option<&str>) -> Option<u64> {
        self.events.lock().unwrap().iter().find_map(|e| match e {
            ProgressEvent::StageFinished {
                stage: s,
                language: lang,
                elapsed_ms,
            } if *s == stage && lang.as_deref() == language => Some(*elapsed_ms),
            _ => None,
        })
    }

    fn cache_stats_for_language(&self, language: &str) -> Option<(usize, usize, usize, bool)> {
        self.events.lock().unwrap().iter().find_map(|e| match e {
            ProgressEvent::Engine {
                stage: Stage::Parsing,
                language: Some(lang),
                event:
                    EngineEvent::CacheStats {
                        cached,
                        reparsed,
                        total_files,
                        disabled,
                        ..
                    },
                ..
            } if lang == language => Some((*cached, *reparsed, *total_files, *disabled)),
            _ => None,
        })
    }
}

impl ProgressSink for RecordingSink {
    fn on_event(&mut self, event: ProgressEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn persistent_db_second_run_reuses_file_cache() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let persistent = scratch.path().join("parse.sqlite");

    // Cold run — populate the persistent DB + file_cache.
    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(scratch.path(), scan_a, vec!["rust".into()]);
    cfg_a.persistent_parse_db = Some(persistent.clone());
    cfg_a.incremental = true;
    run_pipeline(cfg_a)
        .await
        .expect("cold run against persistent DB succeeds");

    // Warm run — same DB, same fixture, incremental ON.
    let scan_b = Uuid::new_v4();
    let sink = RecordingSink::new();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.persistent_parse_db = Some(persistent);
    cfg_b.incremental = true;
    cfg_b.progress = Box::new(sink.clone());

    run_pipeline(cfg_b)
        .await
        .expect("warm run against persistent DB succeeds");

    let (cached, reparsed, total, disabled) = sink
        .cache_stats_for_language("rust")
        .expect("rust parsing pass must emit CacheStats");

    assert!(
        total > 0,
        "fixture must discover at least one rust file; total={total}"
    );
    assert_eq!(
        cached + reparsed,
        total,
        "CacheStats invariant: cached({cached}) + reparsed({reparsed}) == total({total})"
    );
    assert!(
        !disabled,
        "incremental run must not set disabled=true; got cached={cached} reparsed={reparsed}"
    );
    assert!(
        cached > 0,
        "warm run must reuse at least one cached file slice; got cached={cached} reparsed={reparsed}"
    );
    assert_eq!(
        reparsed, 0,
        "warm run with no fixture edits must reparse zero files; got cached={cached} reparsed={reparsed}"
    );
}

/// Regression: Apex managed-package `Import` edges point at virtual
/// `Module` nodes under `<external:managed_package>`. Those nodes must
/// be cached inside the consumer `.cls` slice — not only in a sentinel
/// bucket that is never written to `file_cache` — or warm rescans fail
/// graph validation with a dangling `to_id`.
#[tokio::test]
async fn warm_fully_cached_language_parse_skips_resolution_quickly() {
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
    let sink = RecordingSink::new();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.persistent_parse_db = Some(persistent);
    cfg_b.incremental = true;
    cfg_b.progress = Box::new(sink.clone());

    run_pipeline(cfg_b)
        .await
        .expect("warm fully cached run succeeds");

    let (_, reparsed, _, _) = sink
        .cache_stats_for_language("rust")
        .expect("rust CacheStats");
    assert_eq!(reparsed, 0, "warm run must not reparse any rust files");

    let rust_parse_ms = sink
        .stage_finished_ms(Stage::Parsing, Some("rust"))
        .expect("rust per-language parse stage timing");
    assert!(
        rust_parse_ms < 15_000,
        "fully cached rust parse pass should skip resolution; got {rust_parse_ms}ms"
    );
}

#[tokio::test]
async fn apex_warm_rescan_with_managed_package_import_edges_succeeds() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let persistent = scratch.path().join("parse.sqlite");
    let apex_fixture = common::workspace_root()
        .join("graphengine-parsing")
        .join("tests")
        .join("fixtures")
        .join("apex");

    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(scratch.path(), scan_a, vec!["apex".into()]);
    cfg_a.root = apex_fixture.clone();
    cfg_a.persistent_parse_db = Some(persistent.clone());
    run_pipeline(cfg_a)
        .await
        .expect("cold apex run against persistent DB succeeds");

    let scan_b = Uuid::new_v4();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["apex".into()]);
    cfg_b.root = apex_fixture;
    cfg_b.persistent_parse_db = Some(persistent);
    run_pipeline(cfg_b)
        .await
        .expect("warm apex run with managed-package edges succeeds");
}

#[tokio::test]
async fn no_incremental_forces_full_reparse_on_warm_persistent_db() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let persistent = scratch.path().join("parse.sqlite");

    let scan_a = Uuid::new_v4();
    let mut cfg_a = pipeline_config(scratch.path(), scan_a, vec!["rust".into()]);
    cfg_a.persistent_parse_db = Some(persistent.clone());
    run_pipeline(cfg_a)
        .await
        .expect("seed run against persistent DB succeeds");

    let scan_b = Uuid::new_v4();
    let sink = RecordingSink::new();
    let mut cfg_b = pipeline_config(scratch.path(), scan_b, vec!["rust".into()]);
    cfg_b.persistent_parse_db = Some(persistent);
    cfg_b.incremental = false;
    cfg_b.progress = Box::new(sink.clone());

    run_pipeline(cfg_b)
        .await
        .expect("no-incremental run against warm persistent DB succeeds");

    let (cached, reparsed, total, disabled) = sink
        .cache_stats_for_language("rust")
        .expect("rust parsing pass must emit CacheStats");

    assert!(total > 0);
    assert!(
        disabled,
        "--no-incremental must set disabled=true on CacheStats"
    );
    assert_eq!(cached, 0, "no-incremental must not reuse cache slices");
    assert_eq!(
        reparsed, total,
        "no-incremental must reparse every discovered file"
    );
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// (TS nodes, TS file_cache rows, TS disclosure rows) in the parse DB.
fn typescript_rows(db: &std::path::Path) -> (i64, i64, i64) {
    let conn = rusqlite::Connection::open(db).unwrap();
    (
        count(
            &conn,
            "SELECT COUNT(*) FROM nodes WHERE json_extract(location, '$.file') LIKE '%.ts'",
        ),
        count(
            &conn,
            "SELECT COUNT(*) FROM file_cache WHERE language = 'typescript'",
        ),
        count(
            &conn,
            "SELECT COUNT(*) FROM metadata WHERE key = 'resolution_disclosure_typescript'",
        ),
    )
}

/// Regression: a language-restricted rescan of a persistent DB must drop
/// the rows of every language it left out. Before the fix, scanning
/// `rust,typescript` and then `rust` kept the TypeScript nodes, edges and
/// cache rows, so `gridseak audit` counted those stale heuristic edges
/// and refused to run.
#[tokio::test]
async fn language_restricted_rescan_prunes_rows_of_dropped_languages() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("scratch tempdir");
    let persistent = scratch.path().join("parse.sqlite");
    let rust_nodes =
        "SELECT COUNT(*) FROM nodes WHERE json_extract(location, '$.file') LIKE '%.rs'";

    let mut cfg_a = pipeline_config(
        scratch.path(),
        Uuid::new_v4(),
        vec!["rust".into(), "typescript".into()],
    );
    cfg_a.persistent_parse_db = Some(persistent.clone());
    run_pipeline(cfg_a)
        .await
        .expect("full polyglot scan succeeds");
    let (nodes, cache, disclosure) = typescript_rows(&persistent);
    assert!(
        nodes > 0 && cache > 0 && disclosure > 0,
        "fixture must leave TypeScript rows after the full scan; \
         nodes={nodes} cache={cache} disclosure={disclosure}"
    );
    let rust_before = count(
        &rusqlite::Connection::open(&persistent).unwrap(),
        rust_nodes,
    );

    let mut cfg_b = pipeline_config(scratch.path(), Uuid::new_v4(), vec!["rust".into()]);
    cfg_b.persistent_parse_db = Some(persistent.clone());
    let output = run_pipeline(cfg_b)
        .await
        .expect("rust-only rescan succeeds");

    assert_eq!(
        typescript_rows(&persistent),
        (0, 0, 0),
        "rust-only rescan must drop TypeScript nodes, cache rows and disclosure"
    );
    let conn = rusqlite::Connection::open(&persistent).unwrap();
    assert_eq!(
        count(&conn, rust_nodes),
        rust_before,
        "rust rows are untouched"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM edges WHERE from_id NOT IN (SELECT id FROM nodes)
             OR to_id NOT IN (SELECT id FROM nodes)",
        ),
        0,
        "no edge may point at a pruned node"
    );
    let disclosed: Vec<String> = output
        .report
        .resolution_quality
        .iter()
        .flat_map(|q| q.resolution_disclosure.iter().map(|d| d.language.clone()))
        .collect();
    assert!(
        !disclosed.iter().any(|l| l == "typescript"),
        "the report must not disclose a language the scan did not cover; got {disclosed:?}"
    );
}
