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

    for (id, kind, fqn) in [
        ("n_folder", "Folder", "/repo/src"),
        ("n_file", "File", "/repo/src/main.ts"),
        ("n_fn", "Function", "/repo/src/main.ts::main"),
    ] {
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![
                id,
                kind,
                fqn,
                r#"{\"start_line\":1,\"start_char\":0,\"end_line\":1,\"end_char\":0,\"file\":\"/repo/src/main.ts\"}"#,
                r#"{\"source\":\"TreeSitter\",\"confidence\":\"High\"}"#,
                r#"{"path_repo_rel":"src/main.ts"}"#,
            ],
        )
        .expect("insert node");
    }

    for (from_id, to_id, kind) in [
        ("n_folder", "n_file", "Contains"),
        ("n_file", "n_fn", "Call"),
        ("n_folder", "n_fn", "Contains"),
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
fn contract_v1_metadata_is_present_and_mode_is_explicit() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
depth = 10
direction = "out"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service
        .get_custom_graph(template.path())
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");

    let md = v
        .get("metadata")
        .and_then(|m| m.as_object())
        .expect("metadata");
    assert_eq!(
        md.get("contract_version").and_then(|s| s.as_str()),
        Some("template_query_v1")
    );
    assert_eq!(
        md.get("query_mode").and_then(|s| s.as_str()),
        Some("filtered_dump")
    );
    assert!(md.get("externals").is_some());
    assert!(md.get("capabilities").is_some());
}

#[test]
fn externals_on_materializes_stub_nodes_for_missing_endpoints() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
node_filter = "node_type in ['Folder','File']"
edge_filter = "rel == 'Contains'"
depth = 1
direction = "out"
show_externals = true
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service
        .get_custom_graph(template.path())
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let nodes = v.get("nodes").and_then(|n| n.as_array()).unwrap();
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();

    // Should include the missing function node as a stub because it is an endpoint of Contains.
    assert_eq!(edges.len(), 2);
    assert_eq!(nodes.len(), 3);
    let fn_node = nodes
        .iter()
        .find(|n| n.get("id").and_then(|s| s.as_str()) == Some("n_fn"))
        .expect("n_fn node present");
    assert_eq!(
        fn_node
            .get("properties")
            .and_then(|p| p.get("is_stub"))
            .and_then(|b| b.as_bool()),
        Some(true)
    );
}

#[test]
fn edge_filter_is_applied_even_when_endpoints_are_in_node_set() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());

    // Include Function in node_filter so the Call edge endpoints are both selected.
    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
node_filter = "node_type in ['Folder','File','Function']"
edge_filter = "rel == 'Contains'"
depth = 0
direction = "out"
show_externals = false
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let json = service
        .get_custom_graph(template.path())
        .expect("query should succeed");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();

    assert_eq!(edges.len(), 2);
    assert!(edges
        .iter()
        .all(|e| e.get("type").and_then(|s| s.as_str()) == Some("Contains")));
}

#[test]
fn unsupported_filter_clauses_fail_fast() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
node_filter = "node_type == 'File' or node_type == 'Folder'"
depth = 0
direction = "out"
show_externals = true
"#,
    );

    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");
    let err = service
        .get_custom_graph(template.path())
        .expect_err("should error");
    let msg = format!("{err:#}");
    assert!(msg.contains("unsupported boolean logic") || msg.contains("unsupported clause"));
}
