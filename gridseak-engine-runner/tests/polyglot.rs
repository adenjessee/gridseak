//! Polyglot + clear-error tests.
//!
//! Two assertions, each guarding a regression we just shipped:
//!
//! 1. Multi-language scans actually parse all requested languages.
//!    Pre-runner, `gridseak-cli` would silently fall back to in-process
//!    parsing of *only the first* language whenever the parser binary
//!    was missing — a polyglot codebase scanned via CLI looked smaller
//!    than the same codebase scanned via the desktop. Asserting both
//!    languages appear in `languages_parsed` (and that the report
//!    actually reflects both) locks that fix in.
//!
//! 2. A missing parser binary surfaces a typed `RunError::BinaryMissing`
//!    rather than a silent fallback to anything else. This test pins
//!    the contract that "no parser → loud error, never a half-result."

mod common;

use common::{ensure_engine_binaries, pipeline_config};
use gridseak_engine_runner::{run_pipeline, BinaryKind, RunError};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[tokio::test]
async fn multi_language_scan_parses_every_requested_language() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("create scratch dir");
    let scan_id = Uuid::new_v4();

    let cfg = pipeline_config(
        scratch.path(),
        scan_id,
        vec!["rust".into(), "typescript".into()],
    );
    let out = run_pipeline(cfg)
        .await
        .expect("polyglot pipeline run succeeds");

    assert_eq!(
        out.languages_parsed,
        vec!["rust".to_string(), "typescript".to_string()],
        "both requested languages must end up in languages_parsed in request order"
    );
    assert!(
        out.languages_skipped.is_empty(),
        "no languages should be skipped on a clean rust+ts scan: skipped = {:?}",
        out.languages_skipped
    );

    // Sanity-check the analyzer's report contains nodes from BOTH
    // parser passes. The HealthReport schema has no explicit "language"
    // tag — language is implicit in `file_path` extensions on
    // NodeAnnotation entries. So we walk the report, collect every
    // file extension that shows up under a `file_path` field, and
    // assert at least one `.rs` and one `.ts` are present.
    //
    // If the runner's `--clear` semantics regress (e.g. the second pass
    // also clears the DB), this assertion is what catches it: the
    // report would only contain TypeScript file paths, not Rust.
    let report = serde_json::to_value(&out.report).expect("serialize report");
    let extensions = file_extensions_in_report(&report);
    assert!(
        extensions.contains("rs"),
        "report should contain at least one node from a .rs file; \
         observed extensions = {extensions:?}"
    );
    assert!(
        extensions.contains("ts"),
        "report should contain at least one node from a .ts file; \
         observed extensions = {extensions:?} \
         (a missing .ts means the second parse pass didn't append \
         to the SQLite DB — likely a `--clear` regression)"
    );
}

#[tokio::test]
async fn missing_parser_binary_surfaces_typed_error() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("create scratch dir");
    let scan_id = Uuid::new_v4();

    let mut cfg = pipeline_config(scratch.path(), scan_id, vec!["rust".into()]);
    // Point at a path that absolutely cannot exist. `BinaryMissing` is
    // raised by `validate_binary` before the runner spawns anything,
    // so no subprocess is ever launched.
    cfg.parser_bin = PathBuf::from("/nonexistent/path/graphengine-parsing");

    let err = run_pipeline(cfg)
        .await
        .expect_err("missing parser binary must produce an error, not a degraded success");

    match err {
        RunError::BinaryMissing { which, path } => {
            assert_eq!(which, BinaryKind::Parser);
            assert_eq!(path, PathBuf::from("/nonexistent/path/graphengine-parsing"));
        }
        other => panic!(
            "expected RunError::BinaryMissing {{ which: Parser, .. }}, got {other:?} \
             — a different variant means the runner is doing something other than \
             failing fast at binary validation, which is the regression this test exists to catch"
        ),
    }
}

#[tokio::test]
async fn missing_analyzer_binary_surfaces_typed_error() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("create scratch dir");
    let scan_id = Uuid::new_v4();

    let mut cfg = pipeline_config(scratch.path(), scan_id, vec!["rust".into()]);
    cfg.analyzer_bin = PathBuf::from("/nonexistent/path/ge-analyze");

    let err = run_pipeline(cfg)
        .await
        .expect_err("missing analyzer binary must produce an error");

    match err {
        RunError::BinaryMissing { which, path } => {
            assert_eq!(which, BinaryKind::Analyzer);
            assert_eq!(path, PathBuf::from("/nonexistent/path/ge-analyze"));
        }
        other => panic!("expected BinaryMissing {{ which: Analyzer }}, got {other:?}"),
    }
}

/// Walk the report and collect every file extension that appears
/// under a `file_path` field. `NodeAnnotation` carries `file_path`
/// directly; `Finding` references file paths via the node ids it
/// points to (which the report schema also surfaces in
/// `node_annotations`), so a fan-out walk over the whole report
/// catches both.
///
/// The schema is allowed to evolve (Stage 1+ may add new path-bearing
/// fields). This walker is intentionally generous — anything spelled
/// `file_path: "<...>.<ext>"` counts.
fn file_extensions_in_report(report: &Value) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    walk_for_paths(report, &mut found);
    found
}

fn walk_for_paths(v: &Value, found: &mut std::collections::BTreeSet<String>) {
    match v {
        Value::Object(obj) => {
            if let Some(Value::String(path)) = obj.get("file_path") {
                if let Some(ext) = path.rsplit('.').next() {
                    if ext.len() <= 6 {
                        // Filter out garbage cases like "/path/to/file"
                        // (no extension would yield the full path here).
                        // 6 chars covers .tsx / .mjs / .yaml / .toml etc.
                        found.insert(ext.to_ascii_lowercase());
                    }
                }
            }
            for child in obj.values() {
                walk_for_paths(child, found);
            }
        }
        Value::Array(arr) => {
            for child in arr.iter() {
                walk_for_paths(child, found);
            }
        }
        _ => {}
    }
}
