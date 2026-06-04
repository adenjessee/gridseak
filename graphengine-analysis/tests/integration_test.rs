//! Integration tests for ge-analyze using in-memory SQLite databases.

use graphengine_analysis::health::config::{AnalysisConfig, DeadCodeConfig, Ecosystem};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

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
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file": "src/test.ts", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}"#,
            r#"{"source": "Lsp", "confidence": "High"}"#,
            properties,
        ],
    ).unwrap();
}

/// Serialise a short wire-kind name (e.g. `"Call"`, `"Contains"`)
/// into the serde-tagged JSON object the analysis loader expects
/// post-P1.b (`{"kind":"Call"}` rather than the pre-P1.b hand-rolled
/// plain-string form `"Call"`). Kept as a helper so every pre-P1.b
/// fixture in this file continues to read naturally while exercising
/// the current wire format.
fn wire_kind(kind: &str) -> String {
    format!(r#"{{"kind":"{kind}"}}"#)
}

fn insert_edge(conn: &Connection, from_id: &str, to_id: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from_id,
            to_id,
            wire_kind(kind),
            r#"{"source": "Lsp", "confidence": "High"}"#,
        ],
    )
    .unwrap();
}

#[test]
fn empty_database() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    let graph = graphengine_analysis::health::graph::AnalysisGraph::load(&conn).unwrap();
    assert_eq!(graph.total_nodes(), 0);
    assert_eq!(graph.total_edges(), 0);
}

#[test]
fn linear_chain_no_cycles() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    // File → contains 3 functions in a chain
    insert_node(
        &conn,
        "file1",
        "File",
        "src::utils",
        r#"{"path_repo_rel": "src/utils.ts", "role": "source"}"#,
    );
    insert_node(&conn, "fn_a", "Function", "src::utils::a", "{}");
    insert_node(&conn, "fn_b", "Function", "src::utils::b", "{}");
    insert_node(&conn, "fn_c", "Function", "src::utils::c", "{}");

    insert_edge(&conn, "file1", "fn_a", "Contains");
    insert_edge(&conn, "file1", "fn_b", "Contains");
    insert_edge(&conn, "file1", "fn_c", "Contains");
    insert_edge(&conn, "fn_a", "fn_b", "Call");
    insert_edge(&conn, "fn_b", "fn_c", "Call");

    let graph = graphengine_analysis::health::graph::AnalysisGraph::load(&conn).unwrap();

    assert_eq!(graph.total_nodes(), 4);
    assert_eq!(graph.total_edges(), 5); // 3 Contains + 2 Call
    assert_eq!(graph.total_functions(), 3);
    assert_eq!(graph.total_structural_edges(), 2);

    // Fan-in/out
    assert_eq!(graph.fan_in("fn_a"), 0);
    assert_eq!(graph.fan_out("fn_a"), 1);
    assert_eq!(graph.fan_in("fn_b"), 1);
    assert_eq!(graph.fan_in("fn_c"), 1);

    // Module resolution
    assert_eq!(graph.module_of("fn_a"), Some(&"file1".to_string()));
}

#[test]
fn cyclic_graph_detected() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    // Create a 3-node cycle: auth → db → config → auth
    insert_node(&conn, "proj", "Project", "project", "{}");
    insert_node(
        &conn,
        "f_auth",
        "File",
        "src::auth",
        r#"{"path_repo_rel": "src/auth.ts", "role": "source"}"#,
    );
    insert_node(
        &conn,
        "f_db",
        "File",
        "src::db",
        r#"{"path_repo_rel": "src/db.ts", "role": "source"}"#,
    );
    insert_node(
        &conn,
        "f_config",
        "File",
        "src::config",
        r#"{"path_repo_rel": "src/config.ts", "role": "source"}"#,
    );
    insert_node(&conn, "fn_auth", "Function", "src::auth::verify", "{}");
    insert_node(&conn, "fn_db", "Function", "src::db::connect", "{}");
    insert_node(&conn, "fn_config", "Function", "src::config::load", "{}");

    insert_edge(&conn, "proj", "f_auth", "Contains");
    insert_edge(&conn, "proj", "f_db", "Contains");
    insert_edge(&conn, "proj", "f_config", "Contains");
    insert_edge(&conn, "f_auth", "fn_auth", "Contains");
    insert_edge(&conn, "f_db", "fn_db", "Contains");
    insert_edge(&conn, "f_config", "fn_config", "Contains");

    // The cycle
    insert_edge(&conn, "fn_auth", "fn_db", "Call");
    insert_edge(&conn, "fn_db", "fn_config", "Call");
    insert_edge(&conn, "fn_config", "fn_auth", "Call");

    let mut graph = graphengine_analysis::health::graph::AnalysisGraph::load(&conn).unwrap();
    graph.finalize_production_edges();

    let cycles = graphengine_analysis::health::cycles::detect_cycles(&graph);
    assert_eq!(cycles.cycles.len(), 1);
    assert_eq!(cycles.cycles[0].node_ids.len(), 3);
    assert_eq!(cycles.total_cycle_nodes, 3);

    let cycle_ids: HashSet<&str> = cycles.cycles[0]
        .node_ids
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert!(cycle_ids.contains("fn_auth"));
    assert!(cycle_ids.contains("fn_db"));
    assert!(cycle_ids.contains("fn_config"));
}

#[test]
fn full_pipeline_on_realistic_graph() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    // Build a small but realistic graph:
    // Project
    //   ├── src/auth/ (File)
    //   │   ├── verify (Function) — entry point (handler suffix)
    //   │   └── hash (Function) — internal
    //   ├── src/db/ (File)
    //   │   ├── connect (Function) — called by auth and api
    //   │   └── query (Function) — internal
    //   ├── src/api/ (File)
    //   │   ├── handleRequest (Function) — entry point (handler suffix)
    //   │   └── parseBody (Function) — called by handleRequest
    //   └── src/utils/ (File)
    //       └── format (Function) — dead code (no callers, no handler)

    let nodes = vec![
        ("proj", "Project", "project", "{}"),
        (
            "f_auth",
            "File",
            "src::auth",
            r#"{"path_repo_rel":"src/auth.ts","role":"source"}"#,
        ),
        (
            "f_db",
            "File",
            "src::db",
            r#"{"path_repo_rel":"src/db.ts","role":"source"}"#,
        ),
        (
            "f_api",
            "File",
            "src::api",
            r#"{"path_repo_rel":"src/api.ts","role":"source"}"#,
        ),
        (
            "f_utils",
            "File",
            "src::utils",
            r#"{"path_repo_rel":"src/utils/helpers.ts","role":"source"}"#,
        ),
        ("fn_verify", "Function", "src::auth::verifyHandler", "{}"),
        ("fn_hash", "Function", "src::auth::hash", "{}"),
        ("fn_connect", "Function", "src::db::connect", "{}"),
        ("fn_query", "Function", "src::db::query", "{}"),
        ("fn_handleReq", "Function", "src::api::handleRequest", "{}"),
        ("fn_parseBody", "Function", "src::api::parseBody", "{}"),
        ("fn_format", "Function", "src::utils::format", "{}"),
    ];

    for (id, kind, fqn, props) in &nodes {
        insert_node(&conn, id, kind, fqn, props);
    }

    // Containment
    let contains = vec![
        ("proj", "f_auth"),
        ("proj", "f_db"),
        ("proj", "f_api"),
        ("proj", "f_utils"),
        ("f_auth", "fn_verify"),
        ("f_auth", "fn_hash"),
        ("f_db", "fn_connect"),
        ("f_db", "fn_query"),
        ("f_api", "fn_handleReq"),
        ("f_api", "fn_parseBody"),
        ("f_utils", "fn_format"),
    ];
    for (f, t) in &contains {
        insert_edge(&conn, f, t, "Contains");
    }

    // Structural edges
    insert_edge(&conn, "fn_verify", "fn_hash", "Call");
    insert_edge(&conn, "fn_verify", "fn_connect", "Call");
    insert_edge(&conn, "fn_handleReq", "fn_parseBody", "Call");
    insert_edge(&conn, "fn_handleReq", "fn_connect", "Call");
    insert_edge(&conn, "fn_connect", "fn_query", "Call");

    drop(conn); // Close so ge-analyze can open read-only

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    // Verify structure
    assert_eq!(report.version, "1.0.0");
    assert_eq!(report.summary.total_nodes, 12);
    assert_eq!(report.summary.total_functions, 7);
    assert!(report.health_score.unwrap() <= 100);

    // Verify metrics block is populated
    assert_eq!(report.metrics.cycles.count, 0);
    assert_eq!(report.metrics.cycles.total, 12);
    assert_eq!(report.metrics.dead_code.total, 7);
    assert!(report.metrics.depth.max_call_depth <= 10);
    assert!(!report.metrics.cycles.description.is_empty());

    // No percentiles without norms
    assert!(report.percentiles.is_none());

    // No cycles in this graph
    assert_eq!(report.summary.cycles_found, 0);

    // fn_connect should have fan_in=2 (called by verify and handleReq)
    let connect_ann = &report.node_annotations["fn_connect"];
    assert_eq!(connect_ann.fan_in, 2);
    assert_eq!(connect_ann.blast_radius, 2); // verify and handleReq transitively depend on it

    // fn_format should be flagged as dead (no callers, not a handler)
    let format_ann = &report.node_annotations["fn_format"];
    assert!(
        format_ann.is_dead,
        "fn_format should be flagged as dead code"
    );

    // fn_verify and fn_handleReq are handlers — should NOT be dead
    let verify_ann = &report.node_annotations["fn_verify"];
    assert!(!verify_ann.is_dead, "Handler function should not be dead");
    let handle_ann = &report.node_annotations["fn_handleReq"];
    assert!(!handle_ann.is_dead, "Handler function should not be dead");

    // Findings should exist
    assert!(
        !report.findings.is_empty(),
        "Should have at least dead code finding"
    );

    // Verify deterministic output
    let json1 = serde_json::to_string(&report).unwrap();
    let report2 = graphengine_analysis::health::run_analysis(&db_path).unwrap();
    let json2 = serde_json::to_string(&report2).unwrap();

    // Exclude timestamp and duration fields for determinism check
    let strip_volatile = |s: &str| -> String {
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        let mut m = v.as_object().unwrap().clone();
        m.remove("generated_at");
        m.remove("analysis_duration_ms");
        serde_json::to_string(&m).unwrap()
    };

    assert_eq!(
        strip_volatile(&json1),
        strip_volatile(&json2),
        "Analysis output must be deterministic"
    );
}

#[test]
fn ecosystem_detection_from_project_node() {
    let conn = Connection::open_in_memory().unwrap();
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
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::main", "{}");
    insert_edge(&conn, "proj", "f1", "Contains");
    insert_edge(&conn, "f1", "fn1", "Contains");

    let eco = graphengine_analysis::health::graph::detect_ecosystem(&conn);
    assert_eq!(eco, Ecosystem::TypeScript);
}

#[test]
fn ecosystem_detection_fallback_to_unknown() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    insert_node(&conn, "fn1", "Function", "app::run", "{}");
    let eco = graphengine_analysis::health::graph::detect_ecosystem(&conn);
    assert_eq!(eco, Ecosystem::Unknown);
}

#[test]
fn ecosystem_detection_file_majority_beats_project_label() {
    // Regression for R22 (NPSP case):
    // Project node claims `language='javascript'` (parser default),
    // but 70% of File nodes are Apex. The File-majority must win so
    // downstream Apex-specific rules (TDTM, @AuraEnabled, etc.) are
    // applied — otherwise we classify a Salesforce codebase as JS and
    // every Apex entry point misreports as `framework_annotation_unresolved`.
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language": "javascript"}"#,
    );
    // 7 Apex files...
    for i in 0..7 {
        let id = format!("apex_{i}");
        insert_node(
            &conn,
            &id,
            "File",
            &format!("classes::Svc{i}"),
            r#"{"language":"apex","path_repo_rel":"classes/Svc.cls","role":"source"}"#,
        );
        insert_edge(&conn, "proj", &id, "Contains");
    }
    // 3 JS files.
    for i in 0..3 {
        let id = format!("js_{i}");
        insert_node(
            &conn,
            &id,
            "File",
            &format!("static::bundle{i}"),
            r#"{"language":"javascript","path_repo_rel":"src/a.js","role":"source"}"#,
        );
        insert_edge(&conn, "proj", &id, "Contains");
    }

    let eco = graphengine_analysis::health::graph::detect_ecosystem(&conn);
    assert_eq!(
        eco,
        Ecosystem::Apex,
        "7/10 Apex files must override a Project label of 'javascript'"
    );
}

#[test]
fn ecosystem_detection_project_wins_without_file_majority() {
    // With a 50/50 split (below the 60% threshold) no language has a
    // File-majority, so the Project node's explicit label is trusted.
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language": "typescript"}"#,
    );
    for i in 0..2 {
        let id = format!("py_{i}");
        insert_node(
            &conn,
            &id,
            "File",
            &format!("pkg::m{i}"),
            r#"{"language":"python","role":"source"}"#,
        );
        insert_edge(&conn, "proj", &id, "Contains");
    }
    for i in 0..2 {
        let id = format!("ts_{i}");
        insert_node(
            &conn,
            &id,
            "File",
            &format!("src::m{i}"),
            r#"{"language":"typescript","role":"source"}"#,
        );
        insert_edge(&conn, "proj", &id, "Contains");
    }

    let eco = graphengine_analysis::health::graph::detect_ecosystem(&conn);
    assert_eq!(eco, Ecosystem::TypeScript);
}

#[test]
fn rust_profile_does_not_exempt_jsx_components() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "f1",
        "File",
        "src::ui",
        r#"{"path_repo_rel":"src/ui.rs","role":"source"}"#,
    );
    insert_node(
        &conn,
        "MyComponent",
        "Function",
        "src::ui::MyComponent",
        "{}",
    );
    insert_edge(&conn, "f1", "MyComponent", "Contains");

    let graph = graphengine_analysis::health::graph::AnalysisGraph::load(&conn).unwrap();

    let ts_dc = DeadCodeConfig::for_ecosystem(Ecosystem::TypeScript);
    let ts_dead = graphengine_analysis::health::dead_code::detect_dead_code(&graph, &ts_dc);
    let ts_all: Vec<&String> = ts_dead.all().collect();
    assert!(
        !ts_all.iter().any(|id| *id == "MyComponent"),
        "TypeScript profile should exempt PascalCase as JSX component"
    );

    let rust_dc = DeadCodeConfig::for_ecosystem(Ecosystem::Rust);
    let rust_dead = graphengine_analysis::health::dead_code::detect_dead_code(&graph, &rust_dc);
    let rust_all: Vec<&String> = rust_dead.all().collect();
    assert!(
        rust_all.iter().any(|id| *id == "MyComponent"),
        "Rust profile should NOT exempt PascalCase as JSX component"
    );
}

#[test]
fn explicit_config_overrides_auto_detection() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
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
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    insert_edge(&conn, "proj", "f1", "Contains");
    insert_edge(&conn, "f1", "fn1", "Contains");
    drop(conn);

    // Pass explicit Rust config — should override the TS detected from DB
    let rust_config = AnalysisConfig::for_ecosystem(Ecosystem::Rust);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(rust_config),
        None,
        None,
        None,
    )
    .unwrap();

    // Should succeed without errors
    assert!(report.analysis_errors.is_empty());
    assert!(report.health_score.unwrap() <= 100);
}

// ---------------------------------------------------------------------------
// Calibration fixture tests — validate real-world parsed databases
// ---------------------------------------------------------------------------

fn calibration_fixture_path(name: &str) -> Option<String> {
    let fixture = format!(
        "{}/test-fixtures/calibration_{}.sqlite",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    if !Path::new(&fixture).exists() {
        return None;
    }
    // Verify the DB has actual data (not just an empty schema from a failed parse)
    let conn = Connection::open(&fixture).ok()?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
        .unwrap_or(0);
    if count == 0 {
        eprintln!(
            "SKIP: calibration_{}.sqlite exists but has 0 nodes (parse likely failed)",
            name
        );
        return None;
    }
    Some(fixture)
}

#[test]
fn calibration_hono_health_score_in_range() {
    let db_path = match calibration_fixture_path("hono") {
        Some(p) => p,
        None => {
            eprintln!("SKIP: calibration_hono.sqlite not found (run calibrate.sh first)");
            return;
        }
    };
    let config = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        report.summary.total_nodes > 100,
        "hono should have substantial nodes, got {}",
        report.summary.total_nodes
    );
    assert!(
        report.summary.total_functions > 50,
        "hono should have many functions, got {}",
        report.summary.total_functions
    );
    let hs = report.health_score.unwrap();
    assert!(
        (40..=100).contains(&hs),
        "hono health score out of plausible range: {}",
        hs
    );
    assert!(
        report.findings.len() <= 200,
        "hono findings count unreasonably high: {}",
        report.findings.len()
    );
    // Verify metrics block
    assert!(report.metrics.cycles.total > 0);
    assert!(!report.metrics.coupling.description.is_empty());
}

#[test]
fn calibration_zod_health_score_in_range() {
    let db_path = match calibration_fixture_path("zod") {
        Some(p) => p,
        None => {
            eprintln!("SKIP: calibration_zod.sqlite not found (run calibrate.sh first)");
            return;
        }
    };
    let config = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        report.summary.total_nodes > 10,
        "zod should have nodes, got {}",
        report.summary.total_nodes
    );
    let hs = report.health_score.unwrap();
    assert!(
        (40..=100).contains(&hs),
        "zod health score out of plausible range: {}",
        hs
    );
}

#[test]
fn calibration_express_health_score_in_range() {
    let db_path = match calibration_fixture_path("express") {
        Some(p) => p,
        None => {
            eprintln!("SKIP: calibration_express.sqlite not found (run calibrate.sh first)");
            return;
        }
    };
    let config = AnalysisConfig::for_ecosystem(Ecosystem::JavaScript);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        report.summary.total_nodes > 10,
        "express should have nodes, got {}",
        report.summary.total_nodes
    );
    let hs = report.health_score.unwrap();
    assert!(
        (30..=100).contains(&hs),
        "express health score out of plausible range: {}",
        hs
    );
}

#[test]
fn calibration_fastapi_health_score_in_range() {
    let db_path = match calibration_fixture_path("fastapi") {
        Some(p) => p,
        None => {
            eprintln!("SKIP: calibration_fastapi.sqlite not found (run calibrate.sh first)");
            return;
        }
    };
    let config = AnalysisConfig::for_ecosystem(Ecosystem::Python);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        report.summary.total_nodes > 10,
        "fastapi should have nodes, got {}",
        report.summary.total_nodes
    );
    let hs = report.health_score.unwrap();
    assert!(
        (30..=100).contains(&hs),
        "fastapi health score out of plausible range: {}",
        hs
    );
}

#[test]
fn calibration_chi_health_score_in_range() {
    let db_path = match calibration_fixture_path("chi") {
        Some(p) => p,
        None => {
            eprintln!("SKIP: calibration_chi.sqlite not found (run calibrate.sh first)");
            return;
        }
    };
    let config = AnalysisConfig::for_ecosystem(Ecosystem::Go);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        report.summary.total_nodes > 5,
        "chi should have nodes, got {}",
        report.summary.total_nodes
    );
    let hs = report.health_score.unwrap();
    assert!(
        (30..=100).contains(&hs),
        "chi health score out of plausible range: {}",
        hs
    );
}

#[test]
fn calibration_graphengine_self_analysis() {
    let db_path = match calibration_fixture_path("graphengine") {
        Some(p) => p,
        None => {
            eprintln!("SKIP: calibration_graphengine.sqlite not found (run calibrate.sh first)");
            return;
        }
    };
    let config = AnalysisConfig::for_ecosystem(Ecosystem::Rust);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert!(
        report.summary.total_nodes > 100,
        "graphengine should have many nodes, got {}",
        report.summary.total_nodes
    );
    let hs = report.health_score.unwrap();
    assert!(
        (30..=100).contains(&hs),
        "graphengine health score out of plausible range: {}",
        hs
    );
}

// ---------------------------------------------------------------------------
// Go ecosystem tests
// ---------------------------------------------------------------------------

#[test]
fn go_ecosystem_detection_from_project_node() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn);

    insert_node(&conn, "proj", "Project", "project", r#"{"language": "go"}"#);
    insert_node(
        &conn,
        "f1",
        "File",
        "main",
        r#"{"path_repo_rel":"main.go","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "main::main", "{}");
    insert_edge(&conn, "proj", "f1", "Contains");
    insert_edge(&conn, "f1", "fn1", "Contains");

    let eco = graphengine_analysis::health::graph::detect_ecosystem(&conn);
    assert_eq!(eco, Ecosystem::Go);
}

#[test]
fn go_dead_code_config_disables_jsx_heuristics() {
    let dc = DeadCodeConfig::for_ecosystem(Ecosystem::Go);
    assert!(
        !dc.jsx_components,
        "Go should not enable JSX component exemption"
    );
    assert!(
        !dc.jsx_runtime,
        "Go should not enable JSX runtime exemption"
    );
    assert!(
        !dc.jsx_intrinsic,
        "Go should not enable JSX intrinsic exemption"
    );
    assert!(
        !dc.trait_impls,
        "Go should not enable Rust trait_impls exemption"
    );
    assert!(
        !dc.spring_annotations,
        "Go should not enable Java spring exemption"
    );
    assert!(
        !dc.unity_lifecycle,
        "Go should not enable C# unity exemption"
    );
    assert!(dc.barrel_files, "Universal heuristics should remain on");
    assert!(
        dc.framework_handlers,
        "Universal heuristics should remain on"
    );
}

#[test]
fn go_health_analysis_on_minimal_graph() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    insert_node(&conn, "proj", "Project", "project", r#"{"language": "go"}"#);
    insert_node(
        &conn,
        "f_main",
        "File",
        "main",
        r#"{"path_repo_rel":"main.go","role":"source"}"#,
    );
    insert_node(
        &conn,
        "f_handler",
        "File",
        "handler",
        r#"{"path_repo_rel":"handler/handler.go","role":"source"}"#,
    );
    insert_node(&conn, "fn_main", "Function", "main::main", "{}");
    insert_node(&conn, "fn_serve", "Function", "main::Serve", "{}");
    insert_node(
        &conn,
        "fn_handle",
        "Function",
        "handler::HandleRequest",
        "{}",
    );
    insert_node(&conn, "fn_unused", "Function", "handler::unused", "{}");

    insert_edge(&conn, "proj", "f_main", "Contains");
    insert_edge(&conn, "proj", "f_handler", "Contains");
    insert_edge(&conn, "f_main", "fn_main", "Contains");
    insert_edge(&conn, "f_main", "fn_serve", "Contains");
    insert_edge(&conn, "f_handler", "fn_handle", "Contains");
    insert_edge(&conn, "f_handler", "fn_unused", "Contains");
    insert_edge(&conn, "fn_main", "fn_serve", "Call");
    insert_edge(&conn, "fn_serve", "fn_handle", "Call");

    drop(conn);

    let config = AnalysisConfig::for_ecosystem(Ecosystem::Go);
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(report.summary.total_functions, 4);
    assert_eq!(report.summary.cycles_found, 0);
    assert!(report.health_score.unwrap() <= 100);

    let unused_ann = &report.node_annotations["fn_unused"];
    assert!(
        unused_ann.is_dead,
        "fn_unused has no callers and is not a handler — should be dead"
    );
}

// ---------------------------------------------------------------------------
// Norms / Percentiles tests
// ---------------------------------------------------------------------------

#[test]
fn analysis_with_norms_produces_percentiles() {
    let dir = tempfile::tempdir().unwrap();

    // Create a small analysis database
    let db_path = dir.path().join("test.sqlite");
    let db_str = db_path.to_string_lossy().to_string();
    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    insert_node(&conn, "proj", "Project", "project", "{}");
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::main", "{}");
    insert_node(&conn, "fn2", "Function", "src::app::helper", "{}");
    insert_edge(&conn, "proj", "f1", "Contains");
    insert_edge(&conn, "f1", "fn1", "Contains");
    insert_edge(&conn, "f1", "fn2", "Contains");
    insert_edge(&conn, "fn1", "fn2", "Call");
    drop(conn);

    // Create a population database with a few rows
    let norms_path = dir.path().join("population.sqlite");
    let norms_str = norms_path.to_string_lossy().to_string();
    let norms_conn = graphengine_analysis::health::norms::init_population_db(&norms_str).unwrap();
    for i in 0..10 {
        graphengine_analysis::health::norms::insert_population_row(
            &norms_conn,
            &format!("repo-{i}"),
            "2026-02-25T00:00:00Z",
            "typescript",
            1000 + i * 100,
            200 + i * 20,
            i as f64 * 0.005,
            Some(0.3 + i as f64 * 0.05),
            i as f64 * 0.03,
            0.1 + i as f64 * 0.08,
            5 + i,
            i as f64 * 0.002,
            "calibration",
            None,
            None,
        )
        .unwrap();
    }
    drop(norms_conn);

    // Run analysis with norms
    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_str,
        None,
        Some(&norms_str),
        None,
        None,
    )
    .unwrap();

    // Percentiles should be present
    assert!(
        report.percentiles.is_some(),
        "Percentiles should be present when norms provided"
    );
    let pct = report.percentiles.unwrap();
    assert_eq!(pct.population_size, 10);
    assert!(pct.composite_percentile <= 100);
    assert!(pct.per_metric.contains_key("cycle_ratio"));
    assert!(pct.per_metric.contains_key("avg_coupling"));
    assert!(pct.per_metric.contains_key("dead_ratio"));

    // Health score should be derived from composite percentile
    assert!(report.health_score.is_some());
}

#[test]
fn analysis_without_norms_omits_percentiles() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    insert_edge(&conn, "f1", "fn1", "Contains");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();
    assert!(
        report.percentiles.is_none(),
        "Percentiles should be absent without norms"
    );
    assert!(
        report.health_score.is_some(),
        "Fallback score should still be present"
    );
}

#[test]
fn metrics_block_always_present() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    insert_node(&conn, "fn2", "Function", "src::app::helper", "{}");
    insert_edge(&conn, "f1", "fn1", "Contains");
    insert_edge(&conn, "f1", "fn2", "Contains");
    insert_edge(&conn, "fn1", "fn2", "Call");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    // Metrics block is always present
    assert_eq!(report.metrics.cycles.count, 0);
    assert_eq!(report.metrics.dead_code.total, 2);
    assert!(!report.metrics.coupling.description.is_empty());
    assert!(report.metrics.tangle_index.ratio >= 0.0);

    // Serialization includes metrics
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(json.contains("\"metrics\""));
    assert!(json.contains("\"cycles\""));
    assert!(json.contains("\"hotspot_concentration\""));
}

// ---------------------------------------------------------------------------
// End-to-end tests for the orchestration-bug fix + metric-status contract.
//
// Historical context: `cyclic_graph_detected` above exercises cycle detection
// directly and calls `finalize_production_edges()` manually, which is exactly
// why the ordering bug that shipped in production ("cycle detection reads an
// empty edge set because finalize runs later") was never caught.
// `full_pipeline_on_realistic_graph` does call the real `run_analysis` entry
// point but only on an acyclic graph. These three tests close both gaps:
// they all go through `run_analysis` *and* exercise cycle + status paths.
// ---------------------------------------------------------------------------

/// Regression test for the cycle-detection ordering bug. A 3-node cycle
/// must be discovered by the full `run_analysis` pipeline, not just by
/// calling `detect_cycles` in isolation with a manually-prepared graph.
#[test]
fn full_pipeline_detects_simple_cycle() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    insert_node(&conn, "proj", "Project", "project", "{}");
    insert_node(
        &conn,
        "f_auth",
        "File",
        "src::auth",
        r#"{"path_repo_rel":"src/auth.ts","role":"source"}"#,
    );
    insert_node(
        &conn,
        "f_db",
        "File",
        "src::db",
        r#"{"path_repo_rel":"src/db.ts","role":"source"}"#,
    );
    insert_node(
        &conn,
        "f_config",
        "File",
        "src::config",
        r#"{"path_repo_rel":"src/config.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn_auth", "Function", "src::auth::verify", "{}");
    insert_node(&conn, "fn_db", "Function", "src::db::connect", "{}");
    insert_node(&conn, "fn_config", "Function", "src::config::load", "{}");

    insert_edge(&conn, "proj", "f_auth", "Contains");
    insert_edge(&conn, "proj", "f_db", "Contains");
    insert_edge(&conn, "proj", "f_config", "Contains");
    insert_edge(&conn, "f_auth", "fn_auth", "Contains");
    insert_edge(&conn, "f_db", "fn_db", "Contains");
    insert_edge(&conn, "f_config", "fn_config", "Contains");

    // The cycle: verify -> connect -> load -> verify
    insert_edge(&conn, "fn_auth", "fn_db", "Call");
    insert_edge(&conn, "fn_db", "fn_config", "Call");
    insert_edge(&conn, "fn_config", "fn_auth", "Call");
    drop(conn);

    // Lower the edge threshold so this tiny fixture is considered
    // "enough graph" for the cycles metric to be meaningful.
    let mut config = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
    config.thresholds.min_edges_for_cycle_metric = 2;
    config.thresholds.min_call_edges_for_depth_metric = 2;

    let report = graphengine_analysis::health::run_analysis_with_config(
        &db_path,
        Some(config),
        None,
        None,
        None,
    )
    .unwrap();

    // The core regression assertion: cycle detection, when run through
    // the full pipeline (not the unit helper), actually sees cycles.
    assert!(
        report.summary.cycles_found >= 1,
        "run_analysis must detect the injected 3-node cycle; got {} cycles",
        report.summary.cycles_found
    );
    assert_eq!(report.metrics.cycles.count, 3, "3 nodes in the cycle");
    assert!(
        report.metrics.tangle_index.ratio > 0.0,
        "tangle_index must be > 0 when any cycle exists"
    );

    // The status contract: with enough edges and standard ecosystem,
    // cycles status must be Ok so downstream UIs render the number.
    use graphengine_analysis::health::report::MetricStatus;
    assert_eq!(
        report.metrics.cycles.status,
        MetricStatus::Ok,
        "cycles status must be Ok for this fixture"
    );
    assert_eq!(
        report.metrics.tangle_index.status,
        MetricStatus::Ok,
        "tangle_index status must be Ok"
    );

    // Integrity stamps: every report from the new engine is tagged.
    use graphengine_analysis::health::report::{
        CAVEAT_CYCLES_ORDERFIX_APPLIED, CAVEAT_METRIC_STATUS_CONTRACT,
    };
    assert!(report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_CYCLES_ORDERFIX_APPLIED));
    assert!(report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_METRIC_STATUS_CONTRACT));
    assert!(
        !report.integrity_status.invariant_violations,
        "graph invariants must not be violated on a well-formed fixture"
    );
}

/// On graphs whose production-edge count is below
/// `min_edges_for_cycle_metric`, cycles/tangle/depth must be stamped
/// `InsufficientEdges` so UIs refuse to render "0 cycles" as meaningful.
#[test]
fn full_pipeline_marks_insufficient_edges() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    insert_node(&conn, "proj", "Project", "project", "{}");
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn_a", "Function", "src::app::a", "{}");
    insert_node(&conn, "fn_b", "Function", "src::app::b", "{}");
    insert_edge(&conn, "proj", "f1", "Contains");
    insert_edge(&conn, "f1", "fn_a", "Contains");
    insert_edge(&conn, "f1", "fn_b", "Contains");
    insert_edge(&conn, "fn_a", "fn_b", "Call");
    drop(conn);

    // Default config: min_edges_for_cycle_metric = 50.
    // Our fixture has 1 production call edge, well below the floor.
    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    use graphengine_analysis::health::report::MetricStatus;
    assert_eq!(
        report.metrics.cycles.status,
        MetricStatus::InsufficientEdges,
        "cycles status should flag insufficient edges on a 1-edge graph"
    );
    assert_eq!(
        report.metrics.tangle_index.status,
        MetricStatus::InsufficientEdges,
        "tangle_index inherits the cycles status"
    );
    assert_eq!(
        report.metrics.depth.status,
        MetricStatus::InsufficientEdges,
        "depth status should also flag insufficient call edges"
    );
    // Dead-code has no edge-count floor — it remains Ok because the
    // ecosystem is not declarative-heavy.
    assert_eq!(report.metrics.dead_code.status, MetricStatus::Ok);
}

/// In a declarative-heavy ecosystem (Python/Apex/Java/C#) with low
/// resolution quality, dead-code and framework-sensitive metrics must be
/// stamped `FrameworkInvisible` so UIs warn that routing/dispatch edges
/// are almost certainly missing.
#[test]
fn full_pipeline_marks_framework_invisible() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);

    // Ecosystem=Python triggers DeclarativeHeavy classification.
    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language":"python"}"#,
    );
    insert_node(
        &conn,
        "f1",
        "File",
        "app::views",
        r#"{"path_repo_rel":"app/views.py","role":"source"}"#,
    );
    insert_node(&conn, "fn_view", "Function", "app::views::index", "{}");
    insert_edge(&conn, "proj", "f1", "Contains");
    insert_edge(&conn, "f1", "fn_view", "Contains");
    drop(conn);

    // No Import edges + no lsp_available metadata → ResolutionTier::None.
    // For a declarative-heavy ecosystem, dead-code + framework-sensitive
    // metrics must be flagged as FrameworkInvisible.
    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    use graphengine_analysis::health::report::MetricStatus;
    assert_eq!(
        report.metrics.dead_code.status,
        MetricStatus::FrameworkInvisible,
        "declarative-heavy ecosystems with low resolution must flag dead_code as framework-invisible"
    );
    // Cycles/tangle/depth prefer the more-specific InsufficientEdges
    // diagnostic when both apply. This mirrors the production contract:
    // edge sparsity is a narrower failure mode than framework invisibility.
    assert_eq!(
        report.metrics.cycles.status,
        MetricStatus::InsufficientEdges,
        "cycles should prefer InsufficientEdges over FrameworkInvisible when both apply"
    );
}

/// Invariant check: every HealthReport emitted by this engine carries
/// both caveat stamps so downstream can distinguish it from legacy data.
#[test]
fn reports_are_stamped_with_schema_caveats() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    insert_edge(&conn, "f1", "fn1", "Contains");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    use graphengine_analysis::health::report::{
        CAVEAT_CYCLES_ORDERFIX_APPLIED, CAVEAT_METRIC_STATUS_CONTRACT,
    };
    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_CYCLES_ORDERFIX_APPLIED),
        "reports must be stamped with cycles_orderfix_applied"
    );
    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_METRIC_STATUS_CONTRACT),
        "reports must be stamped with metric_status_contract_v1"
    );
    assert_eq!(
        report.integrity_status.engine_version,
        graphengine_analysis::VERSION
    );
}

// ---------------------------------------------------------------------------
// TR-A.0 / R32: stale parse DB detection
//
// `ge-analyze` must emit `CAVEAT_STALE_PARSE_DB_V1` whenever the parse DB
// predates TR-A.0 (i.e. lacks the `parse_meta` table entirely, or carries a
// `schema_version` value lower than the engine's expected version). The tests
// below pin both directions of the decision so we catch a silent regression if
// `is_parse_db_stale` ever drifts (e.g. a future refactor that forgets to
// bump the expected version, or changes the key name).
// ---------------------------------------------------------------------------

/// A DB produced by a pre-TR-A.0 engine has no `parse_meta` table at all
/// (the table itself was introduced in TR-A.0). The caveat MUST fire so
/// consumers see "re-parse your repository" rather than silently trusting
/// a pessimistic no_callers count.
#[test]
fn legacy_db_without_parse_meta_emits_stale_caveat() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn); // does NOT create parse_meta
    insert_node(
        &conn,
        "f1",
        "File",
        "src::app",
        r#"{"path_repo_rel":"src/app.ts","role":"source"}"#,
    );
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    insert_edge(&conn, "f1", "fn1", "Contains");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    use graphengine_analysis::health::report::CAVEAT_STALE_PARSE_DB_V1;
    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_STALE_PARSE_DB_V1),
        "legacy DB (no parse_meta table) must emit stale_parse_db_v1 caveat; got: {:?}",
        report.integrity_status.schema_caveats
    );
}

/// A DB with `parse_meta` but a schema_version older than the engine's
/// expectation MUST still fire the caveat. Guards against a migration that
/// creates the table but forgets to populate it.
#[test]
fn older_schema_version_emits_stale_caveat() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);
    conn.execute_batch(
        "CREATE TABLE parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO parse_meta (key, value) VALUES ('schema_version', '1');",
    )
    .unwrap();
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    use graphengine_analysis::health::report::CAVEAT_STALE_PARSE_DB_V1;
    assert!(
        report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_STALE_PARSE_DB_V1),
        "older schema_version must emit stale_parse_db_v1 caveat; got: {:?}",
        report.integrity_status.schema_caveats
    );
}

/// A DB with `parse_meta.schema_version` at the engine's expected version
/// MUST NOT fire the caveat. This is the "happy path" assertion — fresh
/// parse-DB produced by the current engine.
#[test]
fn current_schema_version_does_not_emit_stale_caveat() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();

    let conn = Connection::open(&db_path).unwrap();
    create_schema(&conn);
    // Mirror what `SqliteRepository::migrate_schema` stamps on a fresh
    // writer open. The value literal MUST track
    // `ANALYSIS_EXPECTED_SCHEMA_VERSION` in `health/mod.rs`; the
    // `parse_meta_schema_version_alignment` test below is the guard.
    conn.execute_batch(
        "CREATE TABLE parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO parse_meta (key, value) VALUES ('schema_version', '3');",
    )
    .unwrap();
    insert_node(&conn, "fn1", "Function", "src::app::run", "{}");
    drop(conn);

    let report = graphengine_analysis::health::run_analysis(&db_path).unwrap();

    use graphengine_analysis::health::report::CAVEAT_STALE_PARSE_DB_V1;
    assert!(
        !report
            .integrity_status
            .schema_caveats
            .iter()
            .any(|c| c == CAVEAT_STALE_PARSE_DB_V1),
        "fresh DB at current schema_version must NOT emit stale_parse_db_v1; got: {:?}",
        report.integrity_status.schema_caveats
    );
}
