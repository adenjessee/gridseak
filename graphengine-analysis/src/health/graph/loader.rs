//! SQLite -> in-memory node/edge loading.
//!
//! Lifted verbatim out of `health/graph.rs` by R1. This module owns
//! everything that reads from a parse DB and produces the raw
//! `BTreeMap<String, GraphNode> + Vec<GraphEdge>` that the rest of
//! the `graph` module assembles into an `AnalysisGraph`. Keeping the
//! `rusqlite::Connection` surface confined here means
//! `analysis_graph.rs`, `classification.rs`, and `language.rs` can
//! compile against `types` alone and stay testable without a SQLite
//! handle.
//!
//! Public surface re-exported through `super::mod`:
//! - [`read_metadata`] — best-effort lookup against the parser's `metadata` table.
//! - [`validate_schema`] — required-column check used by the
//!   `ge_analyze` binary's pre-flight.
//!
//! Internal surface used by [`super::analysis_graph::AnalysisGraph::load`]:
//! - [`load_nodes`], [`load_edges`], [`parse_edge_confidence`],
//!   [`has_column`].

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::types::{Confidence, GraphEdge, GraphNode, NodeKind, PersistedEdgeKind};

/// Read a metadata value from the `metadata` table (created by the parser).
/// Returns None if the table doesn't exist or the key is missing.
pub fn read_metadata(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM metadata WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get(0),
    )
    .ok()
}

/// Validate that the SQLite database has the expected schema.
pub fn validate_schema(conn: &Connection) -> Result<()> {
    let node_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(nodes)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    let required_node_cols = ["id", "kind", "fqn", "location"];
    for col in &required_node_cols {
        if !node_cols.iter().any(|c| c == col) {
            anyhow::bail!("nodes table missing required column: {col}");
        }
    }

    let edge_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(edges)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    let required_edge_cols = ["from_id", "to_id", "kind"];
    for col in &required_edge_cols {
        if !edge_cols.iter().any(|c| c == col) {
            anyhow::bail!("edges table missing required column: {col}");
        }
    }

    Ok(())
}

/// Parse the `confidence` field out of the stored `provenance` JSON blob.
/// Returns `Confidence::Unknown` whenever the JSON is empty, malformed,
/// or missing the `confidence` key.
///
/// Parsing shape mirrors the parsing crate's `Provenance` serde form:
/// `{ "source": "...", "confidence": "High" | "Medium" | "Low", ... }`.
pub(super) fn parse_edge_confidence(provenance_json: &str) -> Confidence {
    serde_json::from_str::<serde_json::Value>(provenance_json)
        .ok()
        .and_then(|v| {
            v.get("confidence")
                .and_then(|c| c.as_str())
                .map(Confidence::parse)
        })
        .unwrap_or(Confidence::Unknown)
}

/// True if the given table contains the named column. Used to keep the
/// loader read-compatible with older parse DBs (missing `properties`,
/// `trait_metadata`, or `provenance` columns).
pub(super) fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("PRAGMA table_info({})", table))
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .map(|rows| rows.filter_map(|r| r.ok()).any(|c| c == column))
        })
        .unwrap_or(false)
}

pub(super) fn load_nodes(conn: &Connection) -> Result<BTreeMap<String, GraphNode>> {
    let has_properties = has_column(conn, "nodes", "properties");
    let has_trait_metadata = has_column(conn, "nodes", "trait_metadata");

    let query = match (has_properties, has_trait_metadata) {
        (true, true) => "SELECT id, kind, fqn, location, properties, trait_metadata FROM nodes",
        (true, false) => {
            "SELECT id, kind, fqn, location, properties, NULL AS trait_metadata FROM nodes"
        }
        (false, true) => {
            "SELECT id, kind, fqn, location, '{}' AS properties, trait_metadata FROM nodes"
        }
        (false, false) => {
            "SELECT id, kind, fqn, location, '{}' AS properties, NULL AS trait_metadata FROM nodes"
        }
    };

    let mut stmt = conn
        .prepare(query)
        .context("Failed to prepare node query")?;

    let mut nodes = BTreeMap::new();

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let kind_str: String = row.get(1)?;
            let fqn: String = row.get(2)?;
            let location_json: String = row.get(3)?;
            let properties_json: String = row.get(4)?;
            let trait_metadata_json: Option<String> = row.get(5)?;
            Ok((
                id,
                kind_str,
                fqn,
                location_json,
                properties_json,
                trait_metadata_json,
            ))
        })
        .context("Failed to query nodes")?;

    for row in rows {
        let (id, kind_str, fqn, location_json, properties_json, trait_metadata_json) =
            row.context("Failed to read node row")?;

        let kind = match NodeKind::parse(&kind_str) {
            Some(k) => k,
            None => continue,
        };

        let location: serde_json::Value = serde_json::from_str(&location_json).unwrap_or_default();
        let properties: serde_json::Value =
            serde_json::from_str(&properties_json).unwrap_or_default();

        let name = GraphNode::extract_name(&fqn);

        let trait_meta: Option<serde_json::Value> = trait_metadata_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        let is_trait_impl = trait_meta
            .as_ref()
            .map(|tm| tm.get("trait_name").and_then(|v| v.as_str()).is_some())
            .unwrap_or(false);

        let trait_name = trait_meta
            .as_ref()
            .and_then(|tm| tm.get("trait_name"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let node = GraphNode {
            id: id.clone(),
            kind,
            fqn,
            name,
            file_path: location
                .get("file")
                .and_then(|v| v.as_str())
                .map(String::from),
            start_line: location
                .get("start_line")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            end_line: location
                .get("end_line")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            path_repo_rel: properties
                .get("path_repo_rel")
                .and_then(|v| v.as_str())
                .map(String::from),
            role: properties
                .get("role")
                .and_then(|v| v.as_str())
                .map(String::from),
            is_test: properties
                .get("is_test")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_vendor: properties
                .get("is_vendor")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_build_output: properties
                .get("is_build_output")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_generated: properties
                .get("is_generated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            cyclomatic_complexity: properties
                .get("cyclomatic_complexity")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            cognitive_complexity: properties
                .get("cognitive_complexity")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            visibility: properties
                .get("visibility")
                .and_then(|v| v.as_str())
                .map(String::from),
            import_sources: properties
                .get("import_sources")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            is_trait_impl,
            trait_name,
            is_attribute_invoked: properties
                .get("is_attribute_invoked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_callback_target: properties
                .get("is_callback_target")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            entry_point_tags: properties
                .get("entry_points")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            language: properties
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase()),
            frameworks: properties
                .get("frameworks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            is_synthetic: properties
                .get("synthetic")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        };

        nodes.insert(id, node);
    }

    Ok(nodes)
}

pub(super) fn load_edges(
    conn: &Connection,
    nodes: &BTreeMap<String, GraphNode>,
    unknown_kind_count: &mut usize,
) -> Result<Vec<GraphEdge>> {
    // Old parse.dbs may lack the `provenance` column; detect to keep
    // the analysis crate read-compatible. When absent, every edge
    // loads with `Confidence::Unknown`, which the dual-metric pipeline
    // renders as "fidelity gap unmeasurable for this scan".
    let has_provenance = has_column(conn, "edges", "provenance");

    let mut stmt = conn
        .prepare(if has_provenance {
            "SELECT from_id, to_id, kind, provenance FROM edges"
        } else {
            "SELECT from_id, to_id, kind, '{}' AS provenance FROM edges"
        })
        .context("Failed to prepare edge query")?;

    let mut edges = Vec::new();

    let rows = stmt
        .query_map([], |row| {
            let from_id: String = row.get(0)?;
            let to_id: String = row.get(1)?;
            let kind_str: String = row.get(2)?;
            let provenance_json: String = row.get(3)?;
            Ok((from_id, to_id, kind_str, provenance_json))
        })
        .context("Failed to query edges")?;

    for row in rows {
        let (from_id, to_id, kind_str, provenance_json) = row.context("Failed to read edge row")?;

        let kind = match PersistedEdgeKind::from_wire(&kind_str) {
            PersistedEdgeKind::Known(k) => k,
            PersistedEdgeKind::Unknown(_) => {
                *unknown_kind_count += 1;
                continue;
            }
        };

        if !nodes.contains_key(&from_id) || !nodes.contains_key(&to_id) {
            continue;
        }

        let confidence = parse_edge_confidence(&provenance_json);

        edges.push(GraphEdge {
            from_id,
            to_id,
            kind,
            confidence,
        });
    }

    Ok(edges)
}
