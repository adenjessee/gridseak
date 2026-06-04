//! Durable `parse_meta` key/value helpers (S1 scan stats, S2 analysis cache keys).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// `parse_meta` key written at end-of-scan by the orchestrator.
pub const INCREMENTAL_SCAN_STATS_KEY: &str = "incremental_scan_stats";

/// FP-topology fingerprint from the last full/segmented analysis run.
pub const STRUCTURE_FINGERPRINT_KEY: &str = "structure_fingerprint";

/// Stats from the most recent parse pass — consumed by S2 fast-path analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncrementalScanStats {
    pub cached: usize,
    pub reparsed: usize,
    pub removed: usize,
    /// True when `--no-incremental` bypassed cache reuse (planner still ran).
    pub plan_disabled: bool,
    /// Repo-relative paths re-extracted this scan (for delta caveats).
    #[serde(default)]
    pub changed_paths: Vec<String>,
    /// Repo-relative paths removed from disk since last scan.
    #[serde(default)]
    pub removed_paths: Vec<String>,
}

impl IncrementalScanStats {
    pub fn delta_file_count(&self) -> usize {
        self.reparsed + self.removed
    }

    pub fn is_zero_delta(&self) -> bool {
        !self.plan_disabled && self.reparsed == 0 && self.removed == 0
    }
}

pub fn upsert_parse_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO parse_meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .with_context(|| format!("upsert parse_meta key `{key}`"))?;
    Ok(())
}

pub fn read_parse_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='parse_meta'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(None);
    }
    conn.query_row(
        "SELECT value FROM parse_meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .with_context(|| format!("read parse_meta key `{key}`"))
}

pub fn write_incremental_scan_stats(conn: &Connection, stats: &IncrementalScanStats) -> Result<()> {
    let json = serde_json::to_string(stats).context("serialize incremental_scan_stats")?;
    upsert_parse_meta(conn, INCREMENTAL_SCAN_STATS_KEY, &json)
}

/// Clear scan stats at the start of a multi-language parse phase so each
/// language pass can [`merge_incremental_scan_stats`] into a cumulative view.
pub fn reset_incremental_scan_stats(conn: &Connection) -> Result<()> {
    write_incremental_scan_stats(
        conn,
        &IncrementalScanStats {
            cached: 0,
            reparsed: 0,
            removed: 0,
            plan_disabled: false,
            changed_paths: Vec::new(),
            removed_paths: Vec::new(),
        },
    )
}

/// Accumulate one language pass into the durable stats row. Polyglot scans
/// run one orchestrator invocation per language; analysis must see the
/// union of all passes, not only the last language's plan.
pub fn merge_incremental_scan_stats(conn: &Connection, pass: &IncrementalScanStats) -> Result<()> {
    let merged = match read_incremental_scan_stats(conn)? {
        Some(mut existing) => {
            existing.cached += pass.cached;
            existing.reparsed += pass.reparsed;
            existing.removed += pass.removed;
            existing.plan_disabled |= pass.plan_disabled;
            existing
                .changed_paths
                .extend(pass.changed_paths.iter().cloned());
            existing
                .removed_paths
                .extend(pass.removed_paths.iter().cloned());
            existing.changed_paths.sort();
            existing.changed_paths.dedup();
            existing.removed_paths.sort();
            existing.removed_paths.dedup();
            existing
        }
        None => pass.clone(),
    };
    write_incremental_scan_stats(conn, &merged)
}

pub fn read_incremental_scan_stats(conn: &Connection) -> Result<Option<IncrementalScanStats>> {
    let Some(raw) = read_parse_meta(conn, INCREMENTAL_SCAN_STATS_KEY)? else {
        return Ok(None);
    };
    let stats: IncrementalScanStats =
        serde_json::from_str(&raw).context("deserialize incremental_scan_stats")?;
    Ok(Some(stats))
}

/// Blake3 fingerprint of coarse graph shape + incremental delta paths.
/// Used by S2 to decide whether a cached analysis report is still valid
/// enough for a warm fast path.
pub fn compute_graph_fingerprint(
    conn: &Connection,
    stats: &IncrementalScanStats,
) -> Result<String> {
    let node_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
        .unwrap_or(0);
    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
        .unwrap_or(0);

    let mut changed = stats.changed_paths.clone();
    changed.sort();
    let mut removed = stats.removed_paths.clone();
    removed.sort();

    let canonical = format!(
        "nodes={node_count};edges={edge_count};reparsed={};removed={};changed={};removed_paths={}",
        stats.reparsed,
        stats.removed,
        changed.join(","),
        removed.join(","),
    );
    Ok(blake3::hash(canonical.as_bytes()).to_hex().to_string())
}

/// Sorted FQN tuples for structural edges (Call, Import, Contains, Framework, Declarative).
/// Stable across T2 node-id churn when call-graph topology is unchanged.
pub fn compute_structure_fingerprint(conn: &Connection) -> Result<String> {
    let node_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
        .unwrap_or(0);
    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
        .unwrap_or(0);

    let mut stmt = conn.prepare(
        "SELECT e.kind, n1.fqn, n2.fqn
         FROM edges e
         JOIN nodes n1 ON e.from_id = n1.id
         JOIN nodes n2 ON e.to_id = n2.id",
    )?;
    let mut rows = stmt.query([])?;
    let mut tuples: Vec<String> = Vec::new();
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        if !is_topology_edge_kind(&kind) {
            continue;
        }
        let from_fqn: String = row.get(1)?;
        let to_fqn: String = row.get(2)?;
        tuples.push(format!("{kind}|{from_fqn}|{to_fqn}"));
    }
    tuples.sort();
    tuples.dedup();

    let canonical = format!(
        "nodes={node_count};edges={edge_count};topology={}",
        tuples.join(";")
    );
    Ok(blake3::hash(canonical.as_bytes()).to_hex().to_string())
}

fn is_topology_edge_kind(kind_json: &str) -> bool {
    kind_json.contains("\"Call\"")
        || kind_json.contains("\"Import\"")
        || kind_json.contains("\"Contains\"")
        || kind_json.contains("\"Framework\"")
        || kind_json.contains("\"Declarative\"")
}

pub fn compute_delta_fingerprint(stats: &IncrementalScanStats) -> String {
    let mut changed = stats.changed_paths.clone();
    changed.sort();
    let mut removed = stats.removed_paths.clone();
    removed.sort();
    let canonical = format!(
        "reparsed={};removed={};changed={};removed_paths={}",
        stats.reparsed,
        stats.removed,
        changed.join(","),
        removed.join(","),
    );
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

pub fn read_structure_fingerprint(conn: &Connection) -> Result<Option<String>> {
    read_parse_meta(conn, STRUCTURE_FINGERPRINT_KEY)
}

pub fn write_structure_fingerprint(conn: &Connection, fingerprint: &str) -> Result<()> {
    upsert_parse_meta(conn, STRUCTURE_FINGERPRINT_KEY, fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_with_parse_meta() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        conn
    }

    #[test]
    fn merge_accumulates_polyglot_passes() {
        let conn = open_with_parse_meta();
        merge_incremental_scan_stats(
            &conn,
            &IncrementalScanStats {
                cached: 10,
                reparsed: 0,
                removed: 0,
                plan_disabled: false,
                changed_paths: Vec::new(),
                removed_paths: Vec::new(),
            },
        )
        .unwrap();
        merge_incremental_scan_stats(
            &conn,
            &IncrementalScanStats {
                cached: 5,
                reparsed: 1,
                removed: 0,
                plan_disabled: false,
                changed_paths: vec!["b.cls".into()],
                removed_paths: Vec::new(),
            },
        )
        .unwrap();
        let merged = read_incremental_scan_stats(&conn).unwrap().unwrap();
        assert_eq!(merged.cached, 15);
        assert_eq!(merged.reparsed, 1);
        assert!(!merged.is_zero_delta());
        assert_eq!(merged.changed_paths, vec!["b.cls"]);
    }

    #[test]
    fn structure_fingerprint_stable_when_only_node_ids_change() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
             location TEXT NOT NULL, provenance TEXT NOT NULL, properties TEXT NOT NULL DEFAULT '{}',
             trait_metadata TEXT);
             CREATE TABLE edges (from_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
             to_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE, kind TEXT NOT NULL,
             provenance TEXT NOT NULL, PRIMARY KEY (from_id, to_id, kind));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes VALUES ('id-a', 'Function', 'mod::foo', '{}', '{}', '{}', NULL),
             ('id-b', 'Function', 'mod::bar', '{}', '{}', '{}', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges VALUES ('id-a', 'id-b', '{\"kind\":\"Call\"}', '{}')",
            [],
        )
        .unwrap();
        let fp_a = compute_structure_fingerprint(&conn).unwrap();
        conn.execute("DELETE FROM edges", []).unwrap();
        conn.execute("DELETE FROM nodes", []).unwrap();
        conn.execute(
            "INSERT INTO nodes VALUES ('id-x', 'Function', 'mod::foo', '{}', '{}', '{}', NULL),
             ('id-y', 'Function', 'mod::bar', '{}', '{}', '{}', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges VALUES ('id-x', 'id-y', '{\"kind\":\"Call\"}', '{}')",
            [],
        )
        .unwrap();
        let fp_b = compute_structure_fingerprint(&conn).unwrap();
        assert_eq!(fp_a, fp_b, "FP-topology must ignore node id churn");
    }
}
