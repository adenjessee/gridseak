//! T3 zero-gap fixture: if every production edge carries
//! `Confidence::High`, every Layer-3 metric's dual-metric
//! `absolute_gap` MUST be exactly 0.0.
//!
//! This is the symmetric companion to
//! `t3f_fidelity_gap_regression.rs`. The divergent fixture proves the
//! dual-metric plumbing *can* report a non-zero gap; this fixture
//! proves it can also report *zero*. Without both, a "fidelity is
//! always non-zero" bug (e.g., off-by-one in the SwapGuard edge-set
//! swap) would read as success on the divergent test alone.
//!
//! The fixture is structurally identical to the divergent one — two
//! modules, cross-module call cycle, cross-module import — but every
//! edge is tagged `Confidence::High`. The expected invariant is that
//! the all-edges and high-only views produce identical numbers.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

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
            to_id TEXT NOT NULL REFERENCES nodes(id),
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
        CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
        CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
        ",
    )
    .unwrap();
}

fn insert_node(conn: &Connection, id: &str, kind: &str, fqn: &str, properties: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file": "src/test.ts", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}"#,
            r#"{"source": "Lsp", "confidence": "High"}"#,
            properties,
        ],
    )
    .unwrap();
}

fn insert_high_edge(conn: &Connection, from_id: &str, to_id: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from_id,
            to_id,
            format!(r#"{{"kind":"{kind}"}}"#),
            r#"{"source": "Lsp", "confidence": "High"}"#
        ],
    )
    .unwrap();
}

/// Two-module fixture where EVERY edge is `Confidence::High`.
///
/// Topology mirrors `t3f_fidelity_gap_regression.rs::build_divergent_db`
/// so the zero-gap assertion cannot be attributed to "the fixture
/// happened to produce trivially-zero metric values". The metrics
/// have real signal (a cycle, variable fan-in, cross-module import);
/// the dual-metric plumbing must just report the same number twice.
fn build_uniform_high_db(path: &std::path::Path) {
    let _ = fs::remove_file(path);
    let conn = Connection::open(path).unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language": "typescript"}"#,
    );

    insert_node(
        &conn,
        "folder_a",
        "Folder",
        "src::mod_a",
        r#"{"path_repo_rel": "src/mod_a", "role": "source"}"#,
    );
    insert_high_edge(&conn, "proj", "folder_a", "Contains");

    insert_node(
        &conn,
        "file_a",
        "File",
        "src::mod_a::file_a",
        r#"{"path_repo_rel": "src/mod_a/file_a.ts", "role": "source"}"#,
    );
    insert_high_edge(&conn, "folder_a", "file_a", "Contains");

    insert_node(
        &conn,
        "folder_b",
        "Folder",
        "src::mod_b",
        r#"{"path_repo_rel": "src/mod_b", "role": "source"}"#,
    );
    insert_high_edge(&conn, "proj", "folder_b", "Contains");

    insert_node(
        &conn,
        "file_b",
        "File",
        "src::mod_b::file_b",
        r#"{"path_repo_rel": "src/mod_b/file_b.ts", "role": "source"}"#,
    );
    insert_high_edge(&conn, "folder_b", "file_b", "Contains");

    for name in &["fn_a", "fn_caller"] {
        insert_node(
            &conn,
            name,
            "Function",
            &format!("src::mod_a::{name}"),
            r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
        );
        insert_high_edge(&conn, "file_a", name, "Contains");
    }

    for name in &["fn_b", "fn_c"] {
        insert_node(
            &conn,
            name,
            "Function",
            &format!("src::mod_b::{name}"),
            r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
        );
        insert_high_edge(&conn, "file_b", name, "Contains");
    }

    // Same topology as the divergent fixture, all High.
    insert_high_edge(&conn, "fn_a", "fn_b", "Call");
    insert_high_edge(&conn, "fn_b", "fn_a", "Call");
    insert_high_edge(&conn, "fn_caller", "fn_c", "Call");
    insert_high_edge(&conn, "fn_a", "fn_caller", "Call");
    insert_high_edge(&conn, "file_a", "file_b", "Import");
}

/// Assert `absolute_gap == 0.0` exactly (no epsilon). `FidelityGap`
/// computes `all_edges_value - high_only_value`; when those two
/// numbers are produced by the same algorithm over the same edge
/// set, the subtraction must be bit-identical zero. If an epsilon is
/// needed to pass, the plumbing is off and the test should fail.
fn assert_zero_gap(
    metric_name: &str,
    fidelity: &graphengine_analysis::health::report::FidelityGap,
) {
    assert_eq!(
        fidelity.absolute_gap,
        0.0,
        "{metric_name}.absolute_gap must be exactly 0.0 on uniform-High fixture, got {}; \
         all_edges_value={}, high_only_value={}, all_edges_count={}, high_only_edges_count={}",
        fidelity.absolute_gap,
        fidelity.all_edges_value,
        fidelity.high_only_value,
        fidelity.all_edges_count,
        fidelity.high_only_edges_count,
    );
    assert_eq!(
        fidelity.all_edges_count, fidelity.high_only_edges_count,
        "{metric_name}: all_edges_count ({}) must equal high_only_edges_count ({}) \
         when every edge is Confidence::High",
        fidelity.all_edges_count, fidelity.high_only_edges_count,
    );
    assert_eq!(
        fidelity.all_edges_value, fidelity.high_only_value,
        "{metric_name}: all_edges_value ({}) must equal high_only_value ({}) \
         when every edge is Confidence::High",
        fidelity.all_edges_value, fidelity.high_only_value,
    );
    // relative_gap is Some(0.0) when denominator is non-zero, None when zero.
    match (fidelity.relative_gap, fidelity.all_edges_value) {
        (Some(g), v) if v.abs() > f64::EPSILON => {
            assert_eq!(
                g, 0.0,
                "{metric_name}: relative_gap must be 0.0 (got {g}) when absolute_gap is 0.0 and \
                 all_edges_value is non-zero ({v})"
            );
        }
        (None, v) => {
            assert!(
                v.abs() <= f64::EPSILON,
                "{metric_name}: relative_gap is None but all_edges_value ({v}) is non-zero; \
                 FidelityGap::from_values should emit Some(0.0) here"
            );
        }
        _ => {}
    }
}

#[test]
fn every_metric_reports_zero_gap_on_uniform_high_graph() {
    let tmp_dir = std::env::temp_dir();
    let db_path: PathBuf = tmp_dir.join(format!(
        "ge_analyze_t3_uniform_high_{}.sqlite",
        std::process::id()
    ));
    build_uniform_high_db(&db_path);
    let db_str = db_path.to_str().unwrap();

    let report = graphengine_analysis::health::run_analysis(db_str)
        .expect("run_analysis on uniform-High fixture failed");

    let metrics = &report.metrics;

    // Directly-carried fidelity.
    assert_zero_gap(
        "cycles",
        metrics
            .cycles
            .fidelity
            .as_ref()
            .expect("cycles.fidelity must be populated"),
    );
    assert_zero_gap(
        "dead_code",
        metrics
            .dead_code
            .fidelity
            .as_ref()
            .expect("dead_code.fidelity must be populated"),
    );
    assert_zero_gap(
        "depth",
        metrics
            .depth
            .fidelity
            .as_ref()
            .expect("depth.fidelity must be populated"),
    );
    assert_zero_gap(
        "tangle_index",
        metrics
            .tangle_index
            .fidelity
            .as_ref()
            .expect("tangle_index.fidelity must be populated"),
    );
    assert_zero_gap(
        "hotspot_concentration",
        metrics
            .hotspot_concentration
            .fidelity
            .as_ref()
            .expect("hotspot_concentration.fidelity must be populated"),
    );
    assert_zero_gap(
        "coupling",
        metrics
            .coupling
            .fidelity
            .as_ref()
            .expect("coupling.fidelity must be populated"),
    );

    // Option-wrapped metrics: if present, must have zero gap.
    if let Some(cohesion) = metrics.cohesion.as_ref() {
        assert_zero_gap(
            "cohesion",
            cohesion
                .fidelity
                .as_ref()
                .expect("cohesion.fidelity must be populated when cohesion is Some"),
        );
    }
    if let Some(distance) = metrics.distance_from_main_sequence.as_ref() {
        assert_zero_gap(
            "distance_from_main_sequence",
            distance.fidelity.as_ref().expect(
                "distance_from_main_sequence.fidelity must be populated when metric is Some",
            ),
        );
    }

    // T4 sanity: uniform-High → Authoritative.
    let rq = report
        .resolution_quality
        .as_ref()
        .expect("resolution_quality must be populated on non-empty DB");
    assert_eq!(
        rq.measured_fidelity.tier,
        graphengine_analysis::health::report::MeasuredFidelityTier::Authoritative,
        "uniform-High fixture must classify as Authoritative; got {:?} with ratio {:?}",
        rq.measured_fidelity.tier,
        rq.measured_fidelity.high_ratio_on_calls,
    );

    let _ = fs::remove_file(&db_path);
}
