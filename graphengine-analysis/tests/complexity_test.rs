//! Tests for complexity analysis in the ge-analyze pipeline.
//!
//! Each test follows Arrange / Act / Assert. Names describe the behavior under test.
//! Helper functions keep tests focused on the single concept they verify.

use graphengine_analysis::health::complexity;
use graphengine_analysis::health::config::ThresholdConfig;
use graphengine_analysis::health::graph::*;
use graphengine_analysis::health::report::{FindingType, Severity};
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// Helpers — graph from in-memory SQLite
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
fn graph_with_functions(fns: &[(&str, Option<u32>, Option<u32>, u32, u32)]) -> AnalysisGraph {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "f1",
        "File",
        "src::module",
        r#"{"path_repo_rel":"src/module.ts","role":"source"}"#,
    );

    for (id, cc, cog, start, end) in fns {
        let fqn = format!("src::module::{id}");
        let location = format!(
            r#"{{"file":"src/module.ts","start_line":{start},"start_char":0,"end_line":{end},"end_char":0}}"#
        );
        let mut props = serde_json::Map::new();
        if let Some(v) = cc {
            props.insert("cyclomatic_complexity".into(), (*v).into());
        }
        if let Some(v) = cog {
            props.insert("cognitive_complexity".into(), (*v).into());
        }
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id, "Function", fqn, location,
                r#"{"source":"TreeSitter","confidence":"High"}"#,
                serde_json::to_string(&props).unwrap(),
            ],
        ).unwrap();
        insert_edge(&conn, "f1", id, "Contains");
    }

    AnalysisGraph::load(&conn).unwrap()
}

fn default_thresholds() -> ThresholdConfig {
    ThresholdConfig::default()
}

// ---------------------------------------------------------------------------
// Cyclomatic complexity — reading from graph properties
// ---------------------------------------------------------------------------

#[test]
fn simple_function_below_threshold_produces_no_findings() {
    let graph = graph_with_functions(&[("simple_fn", Some(3), Some(2), 1, 10)]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert!(result.findings.is_empty());
    assert_eq!(result.functions_measured, 1);
    assert_eq!(result.max_cyclomatic, 3);
}

#[test]
fn function_above_warning_threshold_produces_warning() {
    let graph = graph_with_functions(&[("moderate_fn", Some(12), Some(8), 1, 30)]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].severity, Severity::Warning);
    assert_eq!(
        result.findings[0].finding_type,
        FindingType::ExcessiveComplexity
    );
}

#[test]
fn function_above_high_threshold_produces_high_severity() {
    let graph = graph_with_functions(&[("complex_fn", Some(18), Some(14), 1, 50)]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].severity, Severity::High);
}

#[test]
fn function_above_critical_threshold_produces_critical_severity() {
    let graph = graph_with_functions(&[("god_fn", Some(30), Some(35), 1, 100)]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].severity, Severity::Critical);
}

#[test]
fn cognitive_alone_can_trigger_warning() {
    let graph = graph_with_functions(&[("deeply_nested", Some(5), Some(15), 1, 25)]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].severity, Severity::Warning);
}

// ---------------------------------------------------------------------------
// Aggregate metrics
// ---------------------------------------------------------------------------

#[test]
fn averages_reflect_all_measured_functions() {
    let graph = graph_with_functions(&[
        ("low", Some(2), Some(1), 1, 5),
        ("high", Some(10), Some(8), 6, 30),
    ]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert_eq!(result.functions_measured, 2);
    assert!((result.avg_cyclomatic - 6.0).abs() < 0.01);
    assert!((result.avg_cognitive - 4.5).abs() < 0.01);
    assert_eq!(result.max_cyclomatic, 10);
    assert_eq!(result.max_cognitive, 8);
}

#[test]
fn functions_without_complexity_data_are_not_measured() {
    let graph = graph_with_functions(&[
        ("measured", Some(5), Some(3), 1, 10),
        ("unmeasured", None, None, 11, 20),
    ]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert_eq!(result.functions_measured, 1);
    assert!((result.avg_cyclomatic - 5.0).abs() < 0.01);
}

#[test]
fn empty_graph_produces_zero_metrics() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);
    let graph = AnalysisGraph::load(&conn).unwrap();

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    assert!(result.findings.is_empty());
    assert_eq!(result.functions_measured, 0);
    assert_eq!(result.avg_cyclomatic, 0.0);
    assert_eq!(result.max_cyclomatic, 0);
}

// ---------------------------------------------------------------------------
// Custom thresholds
// ---------------------------------------------------------------------------

#[test]
fn custom_thresholds_change_severity_classification() {
    let mut thresholds = default_thresholds();
    thresholds.cyclomatic_warning = 3;
    thresholds.cyclomatic_high = 5;

    let graph = graph_with_functions(&[("fn_at_4", Some(4), Some(2), 1, 10)]);

    let result = complexity::analyze_complexity(&graph, &thresholds);

    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].severity, Severity::Warning);
}

// ---------------------------------------------------------------------------
// Finding content quality
// ---------------------------------------------------------------------------

#[test]
fn finding_description_includes_function_name_and_metrics() {
    let graph = graph_with_functions(&[("processOrder", Some(18), Some(22), 1, 60)]);

    let result = complexity::analyze_complexity(&graph, &default_thresholds());

    let finding = &result.findings[0];
    assert!(finding.description.contains("processOrder"));
    assert!(finding.description.contains("18"));
    assert!(finding.description.contains("22"));
    assert!(finding.node_ids.contains(&"processOrder".to_string()));
    assert!(finding.recommendation.is_some());
}

// ---------------------------------------------------------------------------
// Full pipeline — complexity data flows through SQLite into report
// ---------------------------------------------------------------------------

#[test]
fn complexity_flows_through_full_pipeline() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    create_schema(&conn);

    // Insert a function with high complexity in properties
    insert_node_with_complexity(
        &conn,
        "fn_complex",
        "Function",
        "src::app::handlePayment",
        "src/app.ts",
        1,
        60,
        22,
        28,
    );
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_edge(&conn, "f1", "fn_complex", "Contains");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    // Complexity annotation should be populated
    let ann = &report.node_annotations["fn_complex"];
    assert_eq!(ann.cyclomatic_complexity, Some(22));
    assert_eq!(ann.cognitive_complexity, Some(28));
    assert_eq!(ann.loc, 60);

    // Should have a complexity finding
    let complexity_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.finding_type == FindingType::ExcessiveComplexity)
        .collect();
    assert!(!complexity_findings.is_empty());

    // Metrics block should include complexity
    assert!(report.metrics.complexity.is_some());
    let cm = report.metrics.complexity.unwrap();
    assert_eq!(cm.max_cyclomatic, 22);
    assert!(cm.functions_above_threshold >= 1);
}

#[test]
fn loc_computed_from_location_data() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "f1",
        "File",
        "src::utils",
        r#"{"path_repo_rel":"src/utils.ts","role":"source"}"#,
    );
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            "fn_helper",
            "Function",
            "src::utils::helper",
            r#"{"file": "src/utils.ts", "start_line": 5, "start_char": 0, "end_line": 25, "end_char": 1}"#,
            r#"{"source": "TreeSitter", "confidence": "High"}"#,
            "{}",
        ],
    ).unwrap();
    insert_edge(&conn, "f1", "fn_helper", "Contains");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    let ann = &report.node_annotations["fn_helper"];
    assert_eq!(ann.loc, 21); // 25 - 5 + 1
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn create_schema(conn: &rusqlite::Connection) {
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
        ",
    )
    .unwrap();
}

fn insert_node(conn: &rusqlite::Connection, id: &str, kind: &str, fqn: &str, properties: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file": "src/test.ts", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}"#,
            r#"{"source": "TreeSitter", "confidence": "High"}"#,
            properties,
        ],
    ).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn insert_node_with_complexity(
    conn: &rusqlite::Connection,
    id: &str,
    kind: &str,
    fqn: &str,
    file: &str,
    start_line: u32,
    end_line: u32,
    cyclomatic: u32,
    cognitive: u32,
) {
    let location = format!(
        r#"{{"file": "{file}", "start_line": {start_line}, "start_char": 0, "end_line": {end_line}, "end_char": 0}}"#,
    );
    let properties = format!(
        r#"{{"cyclomatic_complexity": {cyclomatic}, "cognitive_complexity": {cognitive}}}"#,
    );
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            location,
            r#"{"source": "TreeSitter", "confidence": "High"}"#,
            properties,
        ],
    ).unwrap();
}

fn insert_edge(conn: &rusqlite::Connection, from_id: &str, to_id: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from_id,
            to_id,
            format!(r#"{{"kind":"{kind}"}}"#),
            r#"{"source": "TreeSitter", "confidence": "High"}"#,
        ],
    )
    .unwrap();
}
