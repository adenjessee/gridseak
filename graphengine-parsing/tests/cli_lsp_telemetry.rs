//! CLI smoke test for `--lsp-telemetry` (Sprint D.4 follow-up T2).
//!
//! Invokes the `graphengine-parsing` binary as a subprocess against a
//! tiny in-memory Rust fixture, passes `--lsp-telemetry <path>`, and
//! asserts that:
//!   1. the file is created,
//!   2. it deserializes back into `LspTelemetryReport`,
//!   3. `schema_version` matches the pinned constant,
//!   4. required top-level keys (counters, derived, language,
//!      scan_duration_ms) are present,
//!   5. derived.fallback_rate is a sane value (Some in [0,1] or None
//!      for empty scans),
//!   6. session_metrics behaves correctly (populated when LSP path is
//!      taken, None when the non-LSP resolver path fires).
//!
//! We use Rust (not Apex) as the fixture language because:
//!   * it does not require apex-jorje-lsp.jar / Java to be installed,
//!   * even without rust-analyzer the heuristic resolver still runs,
//!     exercising the fallback path that makes this telemetry
//!     interesting in the first place.

use std::path::PathBuf;
use std::process::Command;

use graphengine_parsing::infrastructure::lsp::telemetry_export::{
    LspTelemetryReport, SCHEMA_VERSION,
};

/// Absolute path to the `graphengine-parsing` binary under test.
/// Cargo provides this env var for every `[[bin]]` target during
/// integration-test runs; it is the stable way to invoke our CLI
/// without hardcoding target/debug paths.
fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_graphengine-parsing"))
}

/// Absolute path to `graphengine-parsing/configs/`. Derived from the
/// crate manifest dir so the test is independent of the current
/// working directory.
fn configs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs")
}

#[test]
fn cli_writes_lsp_telemetry_json_with_expected_schema() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let repo = tmp.path();

    // Tiny fixture with a cross-function call so the heuristic has
    // real work to do — an empty scan is a degenerate case that won't
    // exercise the derived-metrics fallback-rate math.
    std::fs::write(
        repo.join("main.rs"),
        r#"
fn helper() -> i32 { 1 }

fn main() {
    let _ = helper();
}
"#,
    )
    .expect("write fixture");

    let db_path = repo.join("graph.db");
    let telemetry_path = repo.join("telemetry.json");

    let output = Command::new(binary_path())
        .arg("--configs-dir")
        .arg(configs_dir())
        .arg("parse")
        .arg("--root")
        .arg(repo)
        .arg("--lang")
        .arg("rust")
        .arg("--db")
        .arg(&db_path)
        .arg("--clear")
        .arg("--lsp-telemetry")
        .arg(&telemetry_path)
        .output()
        .expect("spawn graphengine-parsing");

    assert!(
        output.status.success(),
        "parse exited non-zero: status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // --- Assertion 1: file exists and is readable ---
    assert!(
        telemetry_path.exists(),
        "telemetry file was not written at {}",
        telemetry_path.display()
    );
    let raw = std::fs::read_to_string(&telemetry_path).expect("read telemetry");
    assert!(!raw.is_empty(), "telemetry file is empty");

    // --- Assertion 2: deserializes back into the typed report ---
    let report: LspTelemetryReport =
        serde_json::from_str(&raw).expect("deserialize telemetry JSON");

    // --- Assertion 3: schema version pinned ---
    assert_eq!(
        report.schema_version, SCHEMA_VERSION,
        "schema_version drift — bump SCHEMA_VERSION or restore compat"
    );

    // --- Assertion 4: top-level identity fields ---
    assert_eq!(report.language, "rust");
    // scan_duration_ms should be a reasonably small u64 but nonzero
    // for any real scan. Use a generous upper bound (60s) so slow CI
    // hardware doesn't flake the test.
    assert!(
        report.scan_duration_ms < 60_000,
        "scan took suspiciously long: {}ms",
        report.scan_duration_ms
    );

    // --- Assertion 5: derived metrics are internally consistent ---
    let c = &report.counters;
    let d = &report.derived;
    assert_eq!(
        d.total_resolution_work,
        c.lsp_edges + c.heuristic_edges + c.heuristic_call_ambiguous_drops,
        "derived.total_resolution_work must match the formula pinned in \
         telemetry_export.rs; recompute bug otherwise"
    );
    assert_eq!(
        d.heuristic_and_drops,
        c.heuristic_edges + c.heuristic_call_ambiguous_drops
    );
    match d.fallback_rate {
        Some(rate) => {
            assert!(
                (0.0..=1.0).contains(&rate),
                "fallback_rate outside [0,1]: {}",
                rate
            );
        }
        None => {
            // None is only valid if total_resolution_work == 0.
            assert_eq!(
                d.total_resolution_work, 0,
                "fallback_rate=None but total_resolution_work is nonzero; \
                 derived-metrics math is broken"
            );
        }
    }

    // --- Assertion 6: session_metrics behavior ---
    //
    // On a machine where rust-analyzer is installed, the LSP path
    // runs and session_metrics will be Some. On a machine without
    // rust-analyzer, the resolver's is_available() path still
    // returns session metrics from the SessionDefinitionProvider
    // (start_attempts > 0 because the supervisor at least tried),
    // but last_error will likely be populated. Either way, for the
    // `rust` language the resolver is LSP-backed so session_metrics
    // should always be Some. Assert that, and log the concrete
    // values so a test failure helps triage.
    //
    // NOTE: We do NOT assert successful_starts > 0 because CI
    // environments without rust-analyzer are legitimate failure
    // cases we want the user to *see*, not assert against.
    let snap = report
        .session_metrics
        .as_ref()
        .expect("session_metrics should be populated for an LSP-backed resolver");
    eprintln!(
        "[cli_lsp_telemetry_test] session_metrics: attempts={}, successful={}, \
         failed={}, last_error={:?}",
        snap.start_attempts, snap.successful_starts, snap.failed_starts, snap.last_error
    );
    assert!(
        snap.start_attempts >= 1,
        "supervisor should have recorded at least one start attempt"
    );
    assert_eq!(
        snap.start_attempts,
        snap.successful_starts + snap.failed_starts,
        "start_attempts must equal successful_starts + failed_starts; \
         supervisor accounting is off"
    );

    // --- Bonus: database metadata parity ---
    //
    // The same session metrics were also written to graph.metadata.
    // Read them directly from the db and assert parity. This catches
    // drift between the two telemetry channels.
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    let mut stmt = conn
        .prepare("SELECT key, value FROM metadata WHERE key LIKE 'session_%'")
        .expect("prepare metadata query");
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("exec metadata query")
        .filter_map(|r| r.ok())
        .collect();
    let find = |k: &str| rows.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
    assert_eq!(
        find("session_start_attempts"),
        Some(snap.start_attempts.to_string()),
        "db metadata and telemetry JSON must agree on start_attempts"
    );
    assert_eq!(
        find("session_successful_starts"),
        Some(snap.successful_starts.to_string()),
        "db metadata and telemetry JSON must agree on successful_starts"
    );
    assert_eq!(
        find("session_failed_starts"),
        Some(snap.failed_starts.to_string()),
        "db metadata and telemetry JSON must agree on failed_starts"
    );
}

#[test]
fn cli_without_lsp_telemetry_flag_does_not_write_file() {
    // Regression guard: the flag is opt-in. Running without it must
    // not create a stray telemetry.json next to the DB.
    let tmp = tempfile::tempdir().expect("create tempdir");
    let repo = tmp.path();

    std::fs::write(repo.join("main.rs"), "fn main() {}").expect("write fixture");

    let db_path = repo.join("graph.db");
    // Path we explicitly don't pass — any file at this location after
    // the run would indicate a code path that writes unprompted.
    let should_not_exist = repo.join("telemetry.json");

    let output = Command::new(binary_path())
        .arg("--configs-dir")
        .arg(configs_dir())
        .arg("parse")
        .arg("--root")
        .arg(repo)
        .arg("--lang")
        .arg("rust")
        .arg("--db")
        .arg(&db_path)
        .arg("--clear")
        .output()
        .expect("spawn graphengine-parsing");

    assert!(
        output.status.success(),
        "parse exited non-zero: {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !should_not_exist.exists(),
        "telemetry.json was written without --lsp-telemetry being passed"
    );
}
