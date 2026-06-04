use rusqlite::Connection;

/// Best-effort schema version detection for the parsing DB.
///
/// Contract intent:
/// - include something stable and machine-readable so clients can gate behavior
/// - avoid silent schema mismatches
pub fn detect_schema_version(conn: &Connection) -> String {
    // We prefer a lightweight structural detection because older/legacy DBs exist in the repo.
    let has_nodes = table_exists(conn, "nodes");
    let has_edges = table_exists(conn, "edges");
    if !(has_nodes && has_edges) {
        return "unknown_schema".to_string();
    }

    let nodes_cols = table_columns(conn, "nodes");
    let edges_cols = table_columns(conn, "edges");

    let nodes_ok = ["id", "kind", "fqn", "location", "provenance"]
        .iter()
        .all(|c| nodes_cols.iter().any(|x| x == c));
    let edges_ok = ["from_id", "to_id", "kind", "provenance"]
        .iter()
        .all(|c| edges_cols.iter().any(|x| x == c));

    if nodes_ok && edges_ok {
        // "parsing_db_v1" is intentionally stable; it does not encode build numbers.
        return "parsing_db_v1".to_string();
    }

    "unknown_schema".to_string()
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(row)) = rows.next() {
                if let Ok(name) = row.get::<_, String>(1) {
                    cols.push(name);
                }
            }
        }
    }
    cols
}
