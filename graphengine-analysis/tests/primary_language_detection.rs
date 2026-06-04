//! Regression test for Stage-12 follow-up A3.
//!
//! The analyzer is the single source of truth for "what language is
//! this project?" because the polyglot parser orchestrator clobbers
//! the Project node's `properties.language` on every pass
//! (see containment_builder.rs comment near the `// NOTE: We
//! intentionally do NOT set properties.language` block).
//!
//! This test exercises three independent guarantees:
//!
//! 1. `graph::detect_primary_language` returns the File-node majority
//!    language regardless of what the Project node claims.
//! 2. `run_analysis` writes that canonical language back into the
//!    Project node, so any downstream consumer that reads the graph
//!    DB directly sees the corrected value.
//! 3. The same value is surfaced on `HealthReport.primary_language`
//!    so the CLI / desktop / local store can pin it without a second
//!    DB round-trip.

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
            r#"{"file": "src/x", "start_line": 1, "start_char": 0, "end_line": 5, "end_char": 0}"#,
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

fn populate_polyglot_repo(conn: &Connection) {
    // 4 Rust files, 1 JS file, with a Project node mis-labelled as
    // `javascript` (the exact failure mode from the pilot).
    insert_node(
        conn,
        "proj",
        "Project",
        "project",
        r#"{"language":"javascript"}"#,
    );
    for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
        insert_node(
            conn,
            &format!("f_r_{i}"),
            "File",
            &format!("src::{name}"),
            &format!(r#"{{"path_repo_rel":"src/{name}.rs","role":"source","language":"rust"}}"#),
        );
        insert_edge(conn, "proj", &format!("f_r_{i}"), "Contains");
    }
    insert_node(
        conn,
        "f_js",
        "File",
        "src::tool",
        r#"{"path_repo_rel":"scripts/tool.js","role":"source","language":"javascript"}"#,
    );
    insert_edge(conn, "proj", "f_js", "Contains");

    // Two functions and a call so the analyzer doesn't bail into the
    // empty-graph short-circuit (which intentionally returns
    // primary_language=None).
    insert_node(conn, "fn_a", "Function", "src::a::run", "{}");
    insert_node(conn, "fn_b", "Function", "src::b::helper", "{}");
    insert_edge(conn, "f_r_0", "fn_a", "Contains");
    insert_edge(conn, "f_r_1", "fn_b", "Contains");
    insert_edge(conn, "fn_a", "fn_b", "Call");
}

#[test]
fn detect_primary_language_returns_file_majority_not_project_label() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    populate_polyglot_repo(&conn);

    let lang = graphengine_analysis::health::graph::detect_primary_language(&conn);
    assert_eq!(
        lang.as_deref(),
        Some("rust"),
        "File-majority is 4 rust vs 1 javascript; Project label of `javascript` must NOT win"
    );
}

#[test]
fn detect_primary_language_returns_none_when_no_file_nodes_have_language() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    // Project node with language, but no File nodes carry one.
    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language":"javascript"}"#,
    );
    insert_node(
        &conn,
        "f1",
        "File",
        "src::x",
        r#"{"path_repo_rel":"src/x"}"#,
    );
    insert_edge(&conn, "proj", "f1", "Contains");

    let lang = graphengine_analysis::health::graph::detect_primary_language(&conn);
    assert!(
        lang.is_none(),
        "Without File-node language properties we must not fall back to the Project label \
         (that's the exact bug A3 is fixing). Got {lang:?}"
    );
}

#[test]
fn run_analysis_writes_canonical_primary_language_back_to_project_node() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("graph.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);
    populate_polyglot_repo(&conn);
    drop(conn);

    // 1. The HealthReport carries the canonical answer.
    let report = graphengine_analysis::health::run_analysis(db_path.to_str().unwrap()).unwrap();
    assert_eq!(
        report.primary_language.as_deref(),
        Some("rust"),
        "HealthReport.primary_language must reflect the analyzer's File-majority decision \
         so the CLI/desktop can pin it on `scan_runs.primary_language` without a second \
         DB round-trip"
    );

    // 2. The graph DB Project node now reads `rust` too — any external
    // consumer (desktop UI, future MCP tool, third-party script) gets
    // the corrected label from a direct SQL query.
    let conn = Connection::open(&db_path).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT properties FROM nodes WHERE kind = 'Project' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let props: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        props.get("language").and_then(|v| v.as_str()),
        Some("rust"),
        "run_analysis must overwrite Project.properties.language with the canonical \
         File-majority answer; the parser's pre-scan label must NOT survive a complete scan"
    );
}
