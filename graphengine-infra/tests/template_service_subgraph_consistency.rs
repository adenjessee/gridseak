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

    // Nodes:
    // - Folder + File (in filter)
    // - Function (NOT in filter)
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
                r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/src/main.ts"}"#,
                r#"{"source":"TreeSitter","confidence":"High"}"#,
                r#"{}"#,
            ],
        )
        .expect("insert node");
    }

    // Edges:
    // - Contains within node set (should be returned)
    // - Call edge (should NOT be returned by rel == 'Contains')
    // - Contains to node outside node set (must be excluded when show_externals=false)
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
fn ge_template_query_emits_self_consistent_subgraph_when_externals_off() {
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

    let nodes = v.get("nodes").and_then(|n| n.as_array()).unwrap();
    let edges = v.get("edges").and_then(|e| e.as_array()).unwrap();

    // Nodes must satisfy node_filter.
    assert_eq!(nodes.len(), 2);
    let node_ids: std::collections::HashSet<String> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|s| s.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(node_ids.contains("n_folder"));
    assert!(node_ids.contains("n_file"));
    assert!(!node_ids.contains("n_fn"));

    // Edges must satisfy edge_filter and must not reference nodes outside nodes[].
    assert_eq!(edges.len(), 1);
    let e = &edges[0];
    assert_eq!(e.get("type").and_then(|s| s.as_str()), Some("Contains"));
    let src = e.get("source").and_then(|s| s.as_str()).unwrap();
    let dst = e.get("target").and_then(|s| s.as_str()).unwrap();
    assert!(node_ids.contains(src));
    assert!(node_ids.contains(dst));
    assert_eq!((src, dst), ("n_folder", "n_file"));
}
