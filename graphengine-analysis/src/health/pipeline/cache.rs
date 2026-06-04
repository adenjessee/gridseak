//! Per-segment cache persistence (S2-β schema v4).

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};

pub struct SegmentCacheRow {
    pub segment_id: String,
    pub graph_fingerprint: String,
    pub payload_json: String,
    pub updated_at: String,
}

pub fn ensure_segment_cache_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS analysis_segment_cache (
            segment_id TEXT NOT NULL,
            graph_fingerprint TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (segment_id, graph_fingerprint)
        )",
        [],
    )?;
    Ok(())
}

pub fn write_segment_cache(conn: &Connection, row: SegmentCacheRow) -> Result<()> {
    ensure_segment_cache_table(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO analysis_segment_cache
         (segment_id, graph_fingerprint, payload_json, updated_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            row.segment_id,
            row.graph_fingerprint,
            row.payload_json,
            row.updated_at
        ],
    )?;
    Ok(())
}

pub fn read_segment_cache(
    conn: &Connection,
    segment_id: &str,
    graph_fingerprint: &str,
) -> Result<Option<String>> {
    ensure_segment_cache_table(conn)?;
    let mut stmt = conn.prepare(
        "SELECT payload_json FROM analysis_segment_cache
         WHERE segment_id = ?1 AND graph_fingerprint = ?2",
    )?;
    let mut rows = stmt.query(params![segment_id, graph_fingerprint])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn touch_analysis_status_background(conn: &Connection) -> Result<()> {
    use super::AnalysisStatusRecord;
    use crate::health::incremental_fast_path::ANALYSIS_STATUS_KEY;
    let now = Utc::now().to_rfc3339();
    let record = AnalysisStatusRecord {
        mode: "background".into(),
        complete: false,
        segments_done: vec!["GraphPrep".into()],
        segments_pending: vec!["HealthScore".into()],
        started_at: now.clone(),
        updated_at: now,
    };
    let json = serde_json::to_string(&record)?;
    graphengine_parsing::infrastructure::storage::parse_meta_store::upsert_parse_meta(
        conn,
        ANALYSIS_STATUS_KEY,
        &json,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::incremental_fast_path::ANALYSIS_STATUS_KEY;

    #[test]
    fn touch_background_status_writes_partial_record() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("g.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        touch_analysis_status_background(&conn).unwrap();
        let raw = graphengine_parsing::infrastructure::storage::parse_meta_store::read_parse_meta(
            &conn,
            ANALYSIS_STATUS_KEY,
        )
        .unwrap()
        .unwrap();
        assert!(raw.contains("\"complete\":false"));
        assert!(raw.contains("background"));
    }
}
