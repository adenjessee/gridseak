//! L1 merge path must be byte-identical across two runs (S2-γ determinism gate).

use rusqlite::Connection;
use std::fs;

fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
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
            from_id TEXT NOT NULL REFERENCES nodes(id),
            to_id TEXT NOT NULL REFERENCES nodes(id),
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        ",
    )
    .unwrap();
}

fn insert_node(conn: &Connection, id: &str, kind: &str, fqn: &str, file: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            format!(r#"{{"file": "{file}", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}}"#),
            r#"{"source": "Lsp", "confidence": "High"}"#,
            r#"{"cyclomatic_complexity": 2, "cognitive_complexity": 1}"#,
        ],
    )
    .unwrap();
}

fn insert_edge(conn: &Connection, from_id: &str, to_id: &str, kind: &str) {
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            from_id,
            to_id,
            format!(r#"{{"kind":"{kind}"}}"#),
            r#"{"source": "Lsp", "confidence": "High"}"#,
        ],
    )
    .unwrap();
}

fn build_db(path: &std::path::Path) {
    let _ = fs::remove_file(path);
    let conn = Connection::open(path).unwrap();
    create_schema(&conn);

    insert_node(&conn, "proj", "Project", "project", "src/lib.rs");
    insert_node(&conn, "mod", "Folder", "src::lib", "src/lib.rs");
    insert_edge(&conn, "proj", "mod", "Contains");

    insert_node(&conn, "file", "File", "src::lib::file", "src/lib.rs");
    insert_edge(&conn, "mod", "file", "Contains");

    for idx in 0..4 {
        let id = format!("fn_{idx}");
        insert_node(
            &conn,
            &id,
            "Function",
            &format!("src::lib::fn_{idx}"),
            "src/lib.rs",
        );
        insert_edge(&conn, "file", &id, "Contains");
    }
    insert_edge(&conn, "fn_0", "fn_1", "Call");
    insert_edge(&conn, "fn_1", "fn_2", "Call");

    graphengine_parsing::infrastructure::storage::parse_meta_store::merge_incremental_scan_stats(
        &conn,
        &graphengine_parsing::infrastructure::storage::parse_meta_store::IncrementalScanStats {
            cached: 0,
            reparsed: 1,
            removed: 0,
            plan_disabled: false,
            changed_paths: vec!["src/lib.rs".into()],
            removed_paths: Vec::new(),
        },
    )
    .unwrap();

    let fp = graphengine_parsing::infrastructure::storage::parse_meta_store::compute_structure_fingerprint(&conn).unwrap();
    graphengine_parsing::infrastructure::storage::parse_meta_store::write_structure_fingerprint(
        &conn, &fp,
    )
    .unwrap();

    let full =
        graphengine_analysis::health::run_analysis(path.to_str().unwrap()).expect("seed full");
    graphengine_analysis::health::pipeline::cache::write_segment_cache(
        &conn,
        graphengine_analysis::health::pipeline::cache::SegmentCacheRow {
            segment_id:
                graphengine_analysis::health::pipeline::segments::AnalysisSegment::HealthScore
                    .as_str()
                    .to_string(),
            graph_fingerprint: fp.clone(),
            payload_json: serde_json::to_string(
                &graphengine_analysis::health::pipeline::merge::HealthScoreSegmentPayload {
                    report: full,
                },
            )
            .unwrap(),
            updated_at: "now".into(),
        },
    )
    .unwrap();
}

fn normalise(v: &mut serde_json::Value) {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("generated_at");
        obj.remove("analysis_duration_ms");
        if let Some(integrity) = obj
            .get_mut("integrity_status")
            .and_then(|x| x.as_object_mut())
        {
            integrity.remove("engine_version");
            integrity.remove("engine_commit");
        }
        if let Some(provenance) = obj
            .get_mut("analysis_provenance")
            .and_then(|x| x.as_object_mut())
        {
            provenance.remove("delta_fingerprint");
        }
    }
}

#[test]
fn determinism_l1_merge_path() {
    let db_path = std::env::temp_dir().join(format!(
        "l1_merge_determinism_{}.sqlite",
        std::process::id()
    ));
    build_db(&db_path);
    let db_str = db_path.to_str().unwrap();

    let conn = Connection::open(db_str).unwrap();
    let fp = graphengine_parsing::infrastructure::storage::parse_meta_store::compute_structure_fingerprint(&conn).unwrap();
    let delta = graphengine_analysis::health::pipeline::scope::AnalysisDelta {
        changed_paths: vec!["src/lib.rs".into()],
        removed_paths: Vec::new(),
    };

    let r1 = graphengine_analysis::health::pipeline::l1_merge::try_l1_fast_merge(
        &conn, db_str, &fp, &delta, None,
    )
    .expect("merge 1")
    .expect("L1 cache hit");
    let r2 = graphengine_analysis::health::pipeline::l1_merge::try_l1_fast_merge(
        &conn, db_str, &fp, &delta, None,
    )
    .expect("merge 2")
    .expect("L1 cache hit");

    let mut j1 = serde_json::to_value(&r1).unwrap();
    let mut j2 = serde_json::to_value(&r2).unwrap();
    normalise(&mut j1);
    normalise(&mut j2);

    assert_eq!(
        serde_json::to_string_pretty(&j1).unwrap(),
        serde_json::to_string_pretty(&j2).unwrap(),
        "two L1 merges must be byte-identical after normalisation"
    );

    let _ = fs::remove_file(&db_path);
}
