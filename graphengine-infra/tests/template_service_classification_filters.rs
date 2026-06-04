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

    // Insert two file nodes: one vendor, one source.
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
        rusqlite::params![
            "n_vendor",
            "File",
            "/repo/node_modules/pkg/index.ts",
            r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/node_modules/pkg/index.ts"}"#,
            r#"{"source":"TreeSitter","confidence":"High"}"#,
            r#"{"role":"vendor","is_vendor":true,"is_build_output":false,"is_generated":false,"is_test":false,"path_repo_rel":"node_modules/pkg/index.ts","path_abs":"/repo/node_modules/pkg/index.ts"}"#,
        ],
    )
    .expect("insert vendor node");

    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
        rusqlite::params![
            "n_src",
            "File",
            "/repo/src/main.ts",
            r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/src/main.ts"}"#,
            r#"{"source":"TreeSitter","confidence":"High"}"#,
            r#"{"role":"source","is_vendor":false,"is_build_output":false,"is_generated":false,"is_test":false,"path_repo_rel":"src/main.ts","path_abs":"/repo/src/main.ts"}"#,
        ],
    )
    .expect("insert source node");
}

fn write_template(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("tmp template");
    std::fs::write(file.path(), contents).expect("write template");
    file
}

#[test]
fn filters_by_role_using_properties_json() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
node_filter = "role == 'vendor'"
depth = 0
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
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes[0].get("id").and_then(|s| s.as_str()),
        Some("n_vendor")
    );
}

#[test]
fn filters_by_path_repo_rel_prefix() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
node_filter = "path_repo_rel starts_with 'src/'"
depth = 0
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
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].get("id").and_then(|s| s.as_str()), Some("n_src"));
}
