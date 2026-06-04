//! GE-SEED-001: Tests for `by_fqn_like` seed resolution in the v1 contract.
//!
//! Validates that TOML templates can use `seed.roots = [{ by_fqn_like = "%pattern%" }]`
//! to resolve traversal seeds via SQL LIKE against the `fqn` column.

use graphengine_infra::services::template_service::TemplateService;
use rusqlite::Connection;
use tempfile::NamedTempFile;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Creates a test DB with a small but realistic graph:
///
/// ```text
///  Folder:src  --Contains-->  File:main.ts  --Contains-->  Fn:main
///                              File:main.ts  --Contains-->  Fn:handlePayment
///                              File:main.ts  --Contains-->  Fn:processOrder
///                              Fn:processOrder  --Call-->  Fn:handlePayment
///                              Fn:main          --Call-->  Fn:processOrder
/// ```
fn create_test_db(path: &str) {
    let conn = Connection::open(path).expect("open db");
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys=ON;
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
            from_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            to_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        "#,
    )
    .expect("init schema");

    let nodes = [
        (
            "n_folder",
            "Folder",
            "/repo/src",
            r#"{"path_repo_rel":"src"}"#,
        ),
        (
            "n_file",
            "File",
            "/repo/src/main.ts",
            r#"{"path_repo_rel":"src/main.ts"}"#,
        ),
        ("n_fn_main", "Function", "/repo/src/main.ts::main", r#"{}"#),
        (
            "n_fn_handle",
            "Function",
            "/repo/src/main.ts::handlePayment",
            r#"{}"#,
        ),
        (
            "n_fn_process",
            "Function",
            "/repo/src/main.ts::processOrder",
            r#"{}"#,
        ),
    ];

    let location =
        r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/src/main.ts"}"#;
    let provenance = r#"{"source":"TreeSitter","confidence":"High"}"#;

    for (id, kind, fqn, props) in nodes {
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![id, kind, fqn, location, provenance, props],
        )
        .expect("insert node");
    }

    let edges = [
        ("n_folder", "n_file", "Contains"),
        ("n_file", "n_fn_main", "Contains"),
        ("n_file", "n_fn_handle", "Contains"),
        ("n_file", "n_fn_process", "Contains"),
        ("n_fn_process", "n_fn_handle", "Call"),
        ("n_fn_main", "n_fn_process", "Call"),
    ];

    for (from_id, to_id, kind) in edges {
        conn.execute(
            "INSERT INTO edges (from_id, to_id, kind, provenance)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![from_id, to_id, kind, r#"{"source":"TreeSitter"}"#],
        )
        .expect("insert edge");
    }
}

fn write_template(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("tmp template");
    std::fs::write(file.path(), contents).expect("write template");
    file
}

fn parse_payload(json: &str) -> serde_json::Value {
    serde_json::from_str(json).expect("valid JSON payload")
}

fn node_ids(payload: &serde_json::Value) -> Vec<String> {
    payload
        .get("nodes")
        .and_then(|n| n.as_array())
        .unwrap()
        .iter()
        .map(|n| n.get("id").and_then(|s| s.as_str()).unwrap().to_string())
        .collect()
}

fn edge_tuples(payload: &serde_json::Value) -> Vec<(String, String, String)> {
    payload
        .get("edges")
        .and_then(|e| e.as_array())
        .unwrap()
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(|s| s.as_str())
                    .unwrap()
                    .to_string(),
                e.get("target")
                    .and_then(|s| s.as_str())
                    .unwrap()
                    .to_string(),
                e.get("type").and_then(|s| s.as_str()).unwrap().to_string(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn by_fqn_like_resolves_single_match() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    // Seed on handlePayment — should resolve to n_fn_handle, then walk inbound Call edges.
    let template = write_template(
        r#"
[perspective]
name = "who_calls_handlePayment"

[seed]
roots = [{ by_fqn_like = "%handlePayment%" }]

[graph]
mode = "traversal"
depth = 1
direction = "in"
edge_filter = "rel in ['Call']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service.get_custom_graph(template.path()).expect("query");
    let payload = parse_payload(&json);

    let ids = node_ids(&payload);
    // handlePayment (seed) + processOrder (its caller)
    assert!(
        ids.contains(&"n_fn_handle".to_string()),
        "seed node present"
    );
    assert!(ids.contains(&"n_fn_process".to_string()), "caller present");

    let edges = edge_tuples(&payload);
    assert!(
        edges.contains(&(
            "n_fn_process".to_string(),
            "n_fn_handle".to_string(),
            "Call".to_string()
        )),
        "call edge present"
    );
}

#[test]
fn by_fqn_like_resolves_multiple_matches() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    // Pattern "main" matches both the File (fqn contains main.ts) and the Function (fqn contains ::main).
    // With direction=out and Call edges only, the Function seed walks outbound calls.
    let template = write_template(
        r#"
[perspective]
name = "all_main"

[seed]
roots = [{ by_fqn_like = "%main%" }]

[graph]
mode = "traversal"
depth = 1
direction = "out"
edge_filter = "rel in ['Call']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service.get_custom_graph(template.path()).expect("query");
    let payload = parse_payload(&json);

    let ids = node_ids(&payload);
    // n_fn_main is a seed (fqn contains "main"), calls processOrder
    assert!(
        ids.contains(&"n_fn_main".to_string()),
        "seed fn:main present"
    );
    assert!(
        ids.contains(&"n_fn_process".to_string()),
        "callee processOrder present"
    );
}

#[test]
fn by_fqn_like_no_matches_returns_error() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    let template = write_template(
        r#"
[perspective]
name = "no_match"

[seed]
roots = [{ by_fqn_like = "%nonexistentSymbol%" }]

[graph]
mode = "traversal"
depth = 2
direction = "both"
edge_filter = "rel in ['Call']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let err = service
        .get_custom_graph(template.path())
        .expect_err("should error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("resolved to 0 nodes"),
        "error message indicates zero seed resolution: {msg}"
    );
}

#[test]
fn by_fqn_like_mixed_with_by_id() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    // Mix by_id and by_fqn_like in the same roots array.
    let template = write_template(
        r#"
[perspective]
name = "mixed_seeds"

[seed]
roots = [
    { by_id = "n_fn_main" },
    { by_fqn_like = "%handlePayment%" }
]

[graph]
mode = "traversal"
depth = 1
direction = "out"
edge_filter = "rel in ['Call']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service.get_custom_graph(template.path()).expect("query");
    let payload = parse_payload(&json);

    let ids = node_ids(&payload);
    // Both seeds resolved: n_fn_main (by_id) and n_fn_handle (by_fqn_like).
    // Outbound calls from n_fn_main: processOrder.
    // Outbound calls from n_fn_handle: none (handlePayment is a leaf).
    assert!(ids.contains(&"n_fn_main".to_string()), "by_id seed present");
    assert!(
        ids.contains(&"n_fn_handle".to_string()),
        "by_fqn_like seed present"
    );
    assert!(
        ids.contains(&"n_fn_process".to_string()),
        "callee of main present"
    );
}

#[test]
fn by_fqn_like_bidirectional_traversal() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    // Seed on processOrder, walk both directions via Call edges.
    // Inbound: main calls processOrder. Outbound: processOrder calls handlePayment.
    let template = write_template(
        r#"
[perspective]
name = "context_processOrder"

[seed]
roots = [{ by_fqn_like = "%processOrder%" }]

[graph]
mode = "traversal"
depth = 1
direction = "both"
edge_filter = "rel in ['Call']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service.get_custom_graph(template.path()).expect("query");
    let payload = parse_payload(&json);

    let ids = node_ids(&payload);
    assert!(ids.contains(&"n_fn_process".to_string()), "seed present");
    assert!(
        ids.contains(&"n_fn_main".to_string()),
        "inbound caller present"
    );
    assert!(
        ids.contains(&"n_fn_handle".to_string()),
        "outbound callee present"
    );
    assert_eq!(ids.len(), 3, "exactly 3 nodes in context");

    let edges = edge_tuples(&payload);
    assert_eq!(edges.len(), 2, "exactly 2 call edges");
}

#[test]
fn existing_by_id_seeds_still_work() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    // Regression: by_id must still work after adding by_fqn_like.
    let template = write_template(
        r#"
[perspective]
name = "by_id_regression"

[seed]
roots = [{ by_id = "n_fn_handle" }]

[graph]
mode = "traversal"
depth = 1
direction = "in"
edge_filter = "rel in ['Call']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service.get_custom_graph(template.path()).expect("query");
    let payload = parse_payload(&json);

    let ids = node_ids(&payload);
    assert!(ids.contains(&"n_fn_handle".to_string()));
    assert!(ids.contains(&"n_fn_process".to_string()));
}

#[test]
fn existing_by_path_repo_rel_seeds_still_work() {
    let db = NamedTempFile::new().expect("tmp db");
    create_test_db(db.path().to_str().unwrap());

    // Regression: by_path_repo_rel must still work after adding by_fqn_like.
    let template = write_template(
        r#"
[perspective]
name = "by_path_regression"

[seed]
roots = [{ by_path_repo_rel = "src" }]

[graph]
mode = "traversal"
depth = 1
direction = "out"
edge_filter = "rel in ['Contains']"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service.get_custom_graph(template.path()).expect("query");
    let payload = parse_payload(&json);

    let ids = node_ids(&payload);
    assert!(ids.contains(&"n_folder".to_string()), "seed folder present");
    assert!(
        ids.contains(&"n_file".to_string()),
        "contained file present"
    );
}
