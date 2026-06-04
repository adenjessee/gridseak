//! T1 rework — P1.c regression fixture.
//!
//! `PersistedEdgeKind { Known(EdgeKind), Unknown(String) }` at the
//! SQLite read boundary exists to preserve forward-compatibility
//! without silently dropping edges. The contract it locks in:
//!
//! 1. A parse DB row whose `kind` column deserialises into a known
//!    `EdgeKind` variant is loaded verbatim into `AnalysisGraph`.
//! 2. A parse DB row whose `kind` column does NOT match any current
//!    variant (e.g. `{"kind":"Framework","sub":"LwcTemplate"}`
//!    written by a newer engine) is skipped, *and* its skip is
//!    counted on `AnalysisGraph::unknown_edge_kind_count()`.
//! 3. Any non-zero skip count causes the analysis pipeline to emit
//!    `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` on
//!    `MetricsReport.integrity_status.schema_caveats`, so downstream
//!    tools can surface "parse DB newer than engine; re-run with a
//!    newer binary" rather than treat the report as a regression.
//!
//! This is the behavioural counterpart to the `copy_invariant`
//! compile-time test in `graphengine-parsing/src/domain/edge.rs`;
//! together they pin both the domain-type purity (EdgeKind stays
//! `Copy`) and the wire-boundary robustness (unknown kinds surface
//! rather than silently drop) called out in the reviewer's Decision-1
//! reframing.

use graphengine_analysis::health::report::CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1;
use rusqlite::Connection;

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
        CREATE INDEX IF NOT EXISTS idx_nodes_fqn ON nodes(fqn);
        CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);

        CREATE TABLE IF NOT EXISTS edges (
            from_id TEXT NOT NULL REFERENCES nodes(id),
            to_id   TEXT NOT NULL REFERENCES nodes(id),
            kind    TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );

        CREATE TABLE IF NOT EXISTS parse_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT INTO parse_meta (key, value) VALUES ('schema_version', '3');
        ",
    )
    .unwrap();
}

fn insert_node(conn: &Connection, id: &str, kind: &str, fqn: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file":"src/test.ts","start_line":1,"start_char":0,"end_line":10,"end_char":0}"#,
            r#"{"source":"Lsp","confidence":"High"}"#,
            "{}",
        ],
    )
    .unwrap();
}

fn insert_edge_raw(conn: &Connection, from: &str, to: &str, kind_wire: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from,
            to,
            kind_wire,
            r#"{"source":"Lsp","confidence":"High"}"#,
        ],
    )
    .unwrap();
}

/// A DB that only contains rows with well-known `EdgeKind` wire
/// strings must not trigger the caveat.
#[test]
fn known_edge_kinds_do_not_emit_caveat() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&path).unwrap();
    create_schema(&conn);
    insert_node(&conn, "fA", "Function", "mod::a");
    insert_node(&conn, "fB", "Function", "mod::b");
    insert_edge_raw(&conn, "fA", "fB", r#"{"kind":"Call"}"#);
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&path).unwrap();

    assert!(
        !report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1),
        "known EdgeKind wire forms must NOT emit unknown_edge_kind_skip_v1; got: {:?}",
        report.integrity_status.schema_caveats
    );
}

/// A DB that contains one or more rows whose `kind` column is an
/// unknown tagged-serde form (e.g. `Framework(LwcTemplate)` shipped
/// by a future engine version) must:
///   1. skip the unknown rows (they do not become edges),
///   2. continue loading known rows verbatim, and
///   3. surface `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` on the report.
///
/// This locks the forward-compat contract: `PersistedEdgeKind` is
/// the one place an unknown-variant string is allowed to exist; any
/// future change that routes unknown strings elsewhere (e.g. panics
/// in `load_edges`, or silently accepts into `EdgeKind` via a new
/// variant without an explicit migration) will fail this test.
#[test]
fn unknown_edge_kind_emits_caveat_and_skips_edge() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&path).unwrap();
    create_schema(&conn);
    insert_node(&conn, "fA", "Function", "mod::a");
    insert_node(&conn, "fB", "Function", "mod::b");
    insert_node(&conn, "fC", "Function", "mod::c");
    insert_edge_raw(&conn, "fA", "fB", r#"{"kind":"Call"}"#);
    insert_edge_raw(
        &conn,
        "fA",
        "fC",
        r#"{"kind":"Framework","sub":"LwcTemplate"}"#,
    );
    insert_edge_raw(&conn, "fB", "fC", r#"{"kind":"MysteryKind"}"#);
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&path).unwrap();

    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1),
        "parse DB with unknown EdgeKind wire forms must emit unknown_edge_kind_skip_v1; \
         got caveats: {:?}",
        report.integrity_status.schema_caveats
    );
}
