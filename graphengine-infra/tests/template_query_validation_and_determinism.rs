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

    for (id, kind, fqn, props) in [
        ("n_a", "Folder", "/repo/a", r#"{"path_repo_rel":"a"}"#),
        (
            "n_b",
            "File",
            "/repo/a/b.ts",
            r#"{"path_repo_rel":"a/b.ts"}"#,
        ),
        ("n_c", "Function", "/repo/a/b.ts::c", r#"{}"#),
    ] {
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![
                id,
                kind,
                fqn,
                r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/a/b.ts"}"#,
                r#"{"source":"TreeSitter","confidence":"High"}"#,
                props,
            ],
        )
        .expect("insert node");
    }

    for (from_id, to_id, kind) in [("n_a", "n_b", "Contains"), ("n_b", "n_c", "Call")] {
        conn.execute(
            "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
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
fn determinism_same_db_same_template_byte_identical() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
mode = "filtered_dump"
node_filter = "node_type in ['Folder','File','Function']"
edge_filter = "rel in ['Contains','Call']"
depth = 10
direction = "both"
show_externals = true
"#,
    );

    let a = service.get_custom_graph(template.path()).expect("query");
    let b = service.get_custom_graph(template.path()).expect("query");
    assert_eq!(a, b, "output must be byte-stable for caching/diffing");
}

#[test]
fn explain_includes_resolved_seed_ids_in_traversal_mode() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_path_repo_rel = "a" }]

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
        .get_custom_graph_with_explain(template.path(), true)
        .expect("query");
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let seed_ids = v
        .get("metadata")
        .and_then(|m| m.get("explain"))
        .and_then(|e| e.get("seed"))
        .and_then(|s| s.get("resolved_seed_ids"))
        .and_then(|a| a.as_array())
        .expect("resolved_seed_ids array");
    let ids = seed_ids
        .iter()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["n_a".to_string()]);
}

#[test]
fn traversal_requires_seed_section_and_roots() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let missing_seed = write_template(
        r#"
[perspective]
name = "t"

[graph]
mode = "traversal"
depth = 1
direction = "out"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let err = service
        .get_custom_graph(missing_seed.path())
        .expect_err("should fail without seed");
    assert!(format!("{err:#}").contains("requires a [seed] section"));

    let missing_roots = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = []

[graph]
mode = "traversal"
depth = 1
direction = "out"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let err = service
        .get_custom_graph(missing_roots.path())
        .expect_err("should fail without roots");
    assert!(format!("{err:#}").contains("seed.roots must contain at least one entry"));
}

#[test]
fn traversal_rejects_invalid_direction_and_pattern_seed() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let bad_direction = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_id = "n_a" }]

[graph]
mode = "traversal"
depth = 1
direction = "sideways"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let err = service
        .get_custom_graph(bad_direction.path())
        .expect_err("should fail");
    assert!(format!("{err:#}").contains("Invalid graph.direction"));

    let pattern_seed = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
mode = "traversal"
depth = 1
direction = "out"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let err = service
        .get_custom_graph(pattern_seed.path())
        .expect_err("should fail");
    assert!(format!("{err:#}").contains("does not support seed.pattern"));
}

#[test]
fn traversal_depth_is_bounded_by_capabilities_max_depth() {
    let db = NamedTempFile::new().expect("tmp db");
    create_parsing_schema_db(db.path().to_str().unwrap());
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let too_deep = write_template(
        r#"
[perspective]
name = "t"

[seed]
roots = [{ by_id = "n_a" }]

[graph]
mode = "traversal"
depth = 999
direction = "out"
edge_filter = "rel == 'Contains'"
show_externals = false
"#,
    );
    let err = service
        .get_custom_graph(too_deep.path())
        .expect_err("should fail");
    assert!(format!("{err:#}").contains("exceeds max_depth"));
}
