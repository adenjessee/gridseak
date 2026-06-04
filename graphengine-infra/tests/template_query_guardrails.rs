use graphengine_infra::services::template_service::TemplateService;
use rusqlite::Connection;
use tempfile::NamedTempFile;

fn create_large_db(path: &str, node_count: usize) {
    let mut conn = Connection::open(path).expect("open db");
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys=OFF;
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
            from_id TEXT NOT NULL,
            to_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        "#,
    )
    .expect("init schema");

    let tx = conn.transaction().expect("tx");
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata)
                 VALUES (?1, 'File', ?2, ?3, ?4, '{}', NULL)",
            )
            .expect("prepare");
        for i in 0..node_count {
            let id = format!("n_{i}");
            let fqn = format!("/repo/{i}.ts");
            stmt.execute(rusqlite::params![
                id,
                fqn,
                r#"{"start_line":1,"start_char":0,"end_line":1,"end_char":0,"file":"/repo/x.ts"}"#,
                r#"{"source":"TreeSitter","confidence":"High"}"#,
            ])
            .expect("insert");
        }
    }
    tx.commit().expect("commit");
}

fn write_template(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("tmp template");
    std::fs::write(file.path(), contents).expect("write template");
    file
}

#[test]
fn filtered_dump_fails_fast_when_exceeding_max_nodes() {
    // Contract limit: max_nodes = 50_000
    let db = NamedTempFile::new().expect("tmp db");
    create_large_db(db.path().to_str().unwrap(), 50_001);
    let service = TemplateService::new(db.path().to_str().unwrap()).expect("service");

    let template = write_template(
        r#"
[perspective]
name = "t"

[seed]
pattern = "%"

[graph]
mode = "filtered_dump"
depth = 0
direction = "out"
show_externals = true
"#,
    );

    let err = service
        .get_custom_graph(template.path())
        .expect_err("should fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("max_nodes"), "unexpected error: {msg}");
}
