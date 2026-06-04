use graphengine_infra::services::template_service::TemplateService;
use rusqlite::Connection;
use std::collections::HashSet;
use tempfile::NamedTempFile;

fn create_diamond_db(path: &str) {
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

    // Diamond:
    //   A -> B -> D
    //   A -> C -> D
    // plus a Call edge we should filter out when rel=='Contains'
    for (id, kind, fqn, props) in [
        ("n_a", "File", "/repo/A.ts", r#"{"path_repo_rel":"A.ts"}"#),
        ("n_b", "File", "/repo/B.ts", r#"{"path_repo_rel":"B.ts"}"#),
        ("n_c", "File", "/repo/C.ts", r#"{"path_repo_rel":"C.ts"}"#),
        ("n_d", "File", "/repo/D.ts", r#"{"path_repo_rel":"D.ts"}"#),
    ] {
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![
                id,
                kind,
                fqn,
                r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/x.ts"}"#,
                r#"{"source":"TreeSitter","confidence":"High"}"#,
                props,
            ],
        )
        .expect("insert node");
    }

    for (from_id, to_id, kind) in [
        ("n_a", "n_b", "Contains"),
        ("n_a", "n_c", "Contains"),
        ("n_b", "n_d", "Contains"),
        ("n_c", "n_d", "Contains"),
        ("n_a", "n_d", "Call"),
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

fn node_id_set(v: &serde_json::Value) -> HashSet<String> {
    v.get("nodes")
        .and_then(|n| n.as_array())
        .unwrap()
        .iter()
        .filter_map(|n| n.get("id").and_then(|s| s.as_str()).map(|s| s.to_string()))
        .collect()
}

fn edge_triple_set(v: &serde_json::Value) -> HashSet<(String, String, String)> {
    v.get("edges")
        .and_then(|e| e.as_array())
        .unwrap()
        .iter()
        .filter_map(|e| {
            let s = e.get("source")?.as_str()?.to_string();
            let t = e.get("target")?.as_str()?.to_string();
            let k = e.get("type")?.as_str()?.to_string();
            Some((s, t, k))
        })
        .collect()
}

#[test]
fn traversal_out_depth_exact_sets() {
    let db = NamedTempFile::new().expect("tmp db");
    create_diamond_db(db.path().to_str().unwrap());
    let svc = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    // depth=0: seed only, no edges
    let t0 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_a" }]

[graph]
mode="traversal"
depth=0
direction="out"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v0: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t0.path()).unwrap()).unwrap();
    assert_eq!(node_id_set(&v0), HashSet::from(["n_a".to_string()]));
    assert!(edge_triple_set(&v0).is_empty());

    // depth=1: A,B,C plus A->B,A->C
    let t1 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_a" }]

[graph]
mode="traversal"
depth=1
direction="out"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v1: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t1.path()).unwrap()).unwrap();
    assert_eq!(
        node_id_set(&v1),
        HashSet::from(["n_a".to_string(), "n_b".to_string(), "n_c".to_string()])
    );
    assert_eq!(
        edge_triple_set(&v1),
        HashSet::from([
            ("n_a".to_string(), "n_b".to_string(), "Contains".to_string()),
            ("n_a".to_string(), "n_c".to_string(), "Contains".to_string()),
        ])
    );

    // depth=2: includes D and the two second-hop edges; Call edge must be excluded by edge_filter.
    let t2 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_a" }]

[graph]
mode="traversal"
depth=2
direction="out"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v2: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t2.path()).unwrap()).unwrap();
    assert_eq!(
        node_id_set(&v2),
        HashSet::from([
            "n_a".to_string(),
            "n_b".to_string(),
            "n_c".to_string(),
            "n_d".to_string()
        ])
    );
    assert_eq!(
        edge_triple_set(&v2),
        HashSet::from([
            ("n_a".to_string(), "n_b".to_string(), "Contains".to_string()),
            ("n_a".to_string(), "n_c".to_string(), "Contains".to_string()),
            ("n_b".to_string(), "n_d".to_string(), "Contains".to_string()),
            ("n_c".to_string(), "n_d".to_string(), "Contains".to_string()),
        ])
    );
}

#[test]
fn traversal_in_depth_exact_sets() {
    let db = NamedTempFile::new().expect("tmp db");
    create_diamond_db(db.path().to_str().unwrap());
    let svc = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    // Seed at D, depth=1 in: reaches B,C (parents)
    let t1 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_d" }]

[graph]
mode="traversal"
depth=1
direction="in"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v1: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t1.path()).unwrap()).unwrap();
    assert_eq!(
        node_id_set(&v1),
        HashSet::from(["n_d".to_string(), "n_b".to_string(), "n_c".to_string()])
    );
    // Edges emitted are among visited nodes with kind Contains.
    assert_eq!(
        edge_triple_set(&v1),
        HashSet::from([
            ("n_b".to_string(), "n_d".to_string(), "Contains".to_string()),
            ("n_c".to_string(), "n_d".to_string(), "Contains".to_string()),
        ])
    );

    // depth=2 in: also reaches A, and includes A->B and A->C edges
    let t2 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_d" }]

[graph]
mode="traversal"
depth=2
direction="in"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v2: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t2.path()).unwrap()).unwrap();
    assert_eq!(
        node_id_set(&v2),
        HashSet::from([
            "n_a".to_string(),
            "n_b".to_string(),
            "n_c".to_string(),
            "n_d".to_string()
        ])
    );
    assert_eq!(
        edge_triple_set(&v2),
        HashSet::from([
            ("n_a".to_string(), "n_b".to_string(), "Contains".to_string()),
            ("n_a".to_string(), "n_c".to_string(), "Contains".to_string()),
            ("n_b".to_string(), "n_d".to_string(), "Contains".to_string()),
            ("n_c".to_string(), "n_d".to_string(), "Contains".to_string()),
        ])
    );
}

#[test]
fn traversal_both_depth_exact_sets() {
    let db = NamedTempFile::new().expect("tmp db");
    create_diamond_db(db.path().to_str().unwrap());
    let svc = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    // Seed at B, depth=1 both: reaches A and D (neighbor union)
    let t1 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_b" }]

[graph]
mode="traversal"
depth=1
direction="both"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v1: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t1.path()).unwrap()).unwrap();
    assert_eq!(
        node_id_set(&v1),
        HashSet::from(["n_a".to_string(), "n_b".to_string(), "n_d".to_string()])
    );
    assert_eq!(
        edge_triple_set(&v1),
        HashSet::from([
            ("n_a".to_string(), "n_b".to_string(), "Contains".to_string()),
            ("n_b".to_string(), "n_d".to_string(), "Contains".to_string()),
        ])
    );

    // depth=2 both: should also pull in C and include A->C and C->D.
    let t2 = write_template(
        r#"
[perspective]
name="t"

[seed]
roots=[{ by_id="n_b" }]

[graph]
mode="traversal"
depth=2
direction="both"
node_filter="node_type in ['File']"
edge_filter="rel == 'Contains'"
show_externals=false
"#,
    );
    let v2: serde_json::Value =
        serde_json::from_str(&svc.get_custom_graph(t2.path()).unwrap()).unwrap();
    assert_eq!(
        node_id_set(&v2),
        HashSet::from([
            "n_a".to_string(),
            "n_b".to_string(),
            "n_c".to_string(),
            "n_d".to_string()
        ])
    );
    assert_eq!(
        edge_triple_set(&v2),
        HashSet::from([
            ("n_a".to_string(), "n_b".to_string(), "Contains".to_string()),
            ("n_a".to_string(), "n_c".to_string(), "Contains".to_string()),
            ("n_b".to_string(), "n_d".to_string(), "Contains".to_string()),
            ("n_c".to_string(), "n_d".to_string(), "Contains".to_string()),
        ])
    );
}
