use graphengine_infra::services::template_service::TemplateService;
use rusqlite::Connection;
use tempfile::NamedTempFile;

fn create_parsing_schema_db(path: &str) {
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
        ("n_fn", "Function", "/repo/src/main.ts::main", r#"{}"#),
    ];

    for (id, kind, fqn, props) in nodes {
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![
                id,
                kind,
                fqn,
                r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/src/main.ts"}"#,
                r#"{"source":"TreeSitter","confidence":"High"}"#,
                props,
            ],
        )
        .expect("insert node");
    }

    for (from_id, to_id, kind) in [
        ("n_folder", "n_file", "Contains"),
        ("n_folder", "n_fn", "Contains"),
        ("n_file", "n_fn", "Call"),
    ] {
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

#[test]
fn traversal_depth_and_externals_affect_emission_as_specified() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    // depth=1 from src folder, emit only Folder/File, externals off -> drops Contains to Function.
    let template_closed = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_path_repo_rel = "src" }]

[graph]
mode = "traversal"
depth = 1
direction = "out"
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let json = service
        .get_custom_graph(template_closed.path())
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let nodes = v.get("nodes").and_then(|n| n.as_array()).unwrap();
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(edges.len(), 1);

    // Same query, externals on -> stub the Function and keep both Contains edges.
    let template_stubs = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_path_repo_rel = "src" }]

[graph]
mode = "traversal"
depth = 1
direction = "out"
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
show_externals = true
"#,
    );
    let json = service
        .get_custom_graph(template_stubs.path())
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let nodes = v.get("nodes").and_then(|n| n.as_array()).unwrap();
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(nodes.len(), 3);
    let fn_node = nodes
        .iter()
        .find(|n| n.get("id").and_then(|s| s.as_str()) == Some("n_fn"))
        .expect("n_fn stub present");
    assert_eq!(
        fn_node
            .get("properties")
            .and_then(|p| p.get("is_stub"))
            .and_then(|b| b.as_bool()),
        Some(true)
    );

    // depth=0 -> seed only, no edges.
    let template_depth0 = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_path_repo_rel = "src" }]

[graph]
mode = "traversal"
depth = 0
direction = "out"
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let json = service
        .get_custom_graph_with_explain(template_depth0.path(), true)
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let nodes = v.get("nodes").and_then(|n| n.as_array()).unwrap();
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();
    let seed_ids = v
        .get("metadata")
        .and_then(|m| m.get("explain"))
        .and_then(|e| e.get("seed"))
        .and_then(|s| s.get("resolved_seed_ids"))
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(seed_ids, vec!["n_folder".to_string()]);
    assert_eq!(nodes.len(), 1);
    assert_eq!(edges.len(), 0);
}

#[test]
fn traversal_direction_in_reaches_parents() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_id = "n_file" }]

[graph]
mode = "traversal"
depth = 1
direction = "in"
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );

    let json = service
        .get_custom_graph(template.path())
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let nodes = v.get("nodes").and_then(|n| n.as_array()).unwrap();
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].get("type").and_then(|s| s.as_str()),
        Some("Contains")
    );
}
