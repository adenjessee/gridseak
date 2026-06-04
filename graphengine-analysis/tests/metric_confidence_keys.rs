//! Regression test for Stage-12 follow-up A2.
//!
//! The CLI's hero metric table reads `report.metrics.metric_confidence`
//! to flip metric rows to `Low-confidence · …` when the analyzer says
//! the underlying signal is weak. That mapping (in
//! `gridseak-cli/src/render/view.rs::confidence_key_for_row`) hard-codes
//! the four keys the analyzer is expected to populate today:
//!
//!   - `depth`
//!   - `dead_code`
//!   - `coupling`
//!   - `blast_radius`
//!
//! If a refactor silently drops one, the CLI silently loses confidence
//! signaling on the matching row without any test catching it. Pinning
//! the contract here is cheap and means the lookup in `view.rs` is
//! never out of sync with the analyzer's output.

use rusqlite::Connection;

fn wire_kind(kind: &str) -> String {
    format!(r#"{{"kind":"{kind}"}}"#)
}

fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL,
            provenance TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);

        CREATE TABLE IF NOT EXISTS edges (
            from_id TEXT NOT NULL REFERENCES nodes(id),
            to_id TEXT NOT NULL REFERENCES nodes(id),
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        ",
    )
    .unwrap();
}

fn insert_node(conn: &Connection, id: &str, kind: &str, fqn: &str, properties: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file": "src/lib.ts", "start_line": 1, "start_char": 0, "end_line": 5, "end_char": 0}"#,
            r#"{"source": "Lsp", "confidence": "High"}"#,
            properties,
        ],
    )
    .unwrap();
}

fn insert_edge(conn: &Connection, from: &str, to: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from,
            to,
            wire_kind(kind),
            r#"{"source": "Lsp", "confidence": "High"}"#,
        ],
    )
    .unwrap();
}

#[test]
fn metric_confidence_always_emits_the_four_known_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("graph.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    // Minimal two-function, single-file graph. `metric_confidence` is
    // populated unconditionally when the report is built; we just need
    // something non-empty so the analyzer doesn't bail into the
    // empty-graph fallback (which is the one place that sets
    // metric_confidence: None).
    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language":"typescript"}"#,
    );
    insert_node(
        &conn,
        "f_lib",
        "File",
        "src::lib",
        r#"{"path_repo_rel":"src/lib.ts","role":"source","language":"typescript"}"#,
    );
    insert_node(&conn, "fn_a", "Function", "src::lib::a", "{}");
    insert_node(&conn, "fn_b", "Function", "src::lib::b", "{}");
    insert_edge(&conn, "proj", "f_lib", "Contains");
    insert_edge(&conn, "f_lib", "fn_a", "Contains");
    insert_edge(&conn, "f_lib", "fn_b", "Contains");
    insert_edge(&conn, "fn_a", "fn_b", "Call");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(db_path.to_str().unwrap()).unwrap();

    let map = report
        .metrics
        .metric_confidence
        .as_ref()
        .expect("metric_confidence must be populated for a non-empty graph");

    // The contract: every key the CLI's `confidence_key_for_row`
    // depends on (see gridseak-cli/src/render/view.rs) must be present.
    let expected = ["depth", "dead_code", "coupling", "blast_radius"];
    for key in &expected {
        assert!(
            map.contains_key(*key),
            "metric_confidence missing required key `{key}`; got {:?}. \
             If the analyzer renames or drops this key, update \
             `confidence_key_for_row` in gridseak-cli/src/render/view.rs \
             at the same time so the CLI's metric-row override keeps \
             working.",
            map.keys().collect::<Vec<_>>()
        );
    }
}
