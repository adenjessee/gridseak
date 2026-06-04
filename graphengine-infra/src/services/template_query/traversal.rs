use rusqlite::Connection;

use super::filters::{ParsedEdgeFilter, ParsedNodeFilter};
use super::seeds::{ParsedSeeds, SeedRoot};

/// (visited_node_ids, visited_edges as (from, to, kind, weight))
pub type TraversalResult = (Vec<String>, Vec<(String, String, String, String)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    Both,
}

impl Direction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "out" => Some(Direction::Out),
            "in" => Some(Direction::In),
            "both" => Some(Direction::Both),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Out => "out",
            Direction::In => "in",
            Direction::Both => "both",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TraversalSpec {
    pub seeds: ParsedSeeds,
    pub max_depth: i32,
    pub direction: Direction,
    pub node_filter: Option<ParsedNodeFilter>,
    pub edge_filter: Option<ParsedEdgeFilter>,
    pub show_externals: bool,
}

pub fn resolve_seed_ids(conn: &Connection, seeds: &ParsedSeeds) -> anyhow::Result<Vec<String>> {
    let mut ids: Vec<String> = Vec::new();
    for root in &seeds.roots {
        match root {
            SeedRoot::ById(id) => {
                ids.push(id.clone());
            }
            SeedRoot::ByPathRepoRel(path_repo_rel) => {
                // Resolve to Folder/File nodes with matching repo-relative path.
                let mut stmt = conn.prepare(
                    "SELECT id FROM nodes \
                     WHERE json_extract(properties, '$.path_repo_rel') = ?1 \
                       AND kind IN ('Folder','File')",
                )?;
                let mut rows = stmt.query([path_repo_rel])?;
                while let Some(row) = rows.next()? {
                    let id: String = row.get(0)?;
                    ids.push(id);
                }
            }
            SeedRoot::ByFqnLike(pattern) => {
                // Resolve via SQL LIKE against the fqn column.
                // The caller supplies wildcards (e.g. "%handlePayment%").
                let mut stmt = conn.prepare("SELECT id FROM nodes WHERE fqn LIKE ?1")?;
                let mut rows = stmt.query([pattern])?;
                while let Some(row) = rows.next()? {
                    let id: String = row.get(0)?;
                    ids.push(id);
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Compute visited node IDs and visited edges (from_id,to_id,kind,provenance) using a recursive CTE.
///
/// Semantics A:
/// - edge_filter gates traversal expansion
/// - node_filter gates emission later (handled by caller)
pub fn traverse_sql(
    conn: &Connection,
    seed_ids: &[String],
    max_depth: i32,
    direction: Direction,
    edge_filter: Option<&ParsedEdgeFilter>,
) -> anyhow::Result<TraversalResult> {
    // Determine edge kinds allowed for traversal. If none provided, traverse all.
    let edge_kinds: Option<Vec<String>> = edge_filter.and_then(|f| f.rel_kinds.clone());

    // Build a dynamic IN list for seed ids and edge kinds.
    let seed_placeholders = seed_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let seed_in = seed_placeholders;

    let edge_kind_clause = if let Some(kinds) = &edge_kinds {
        let placeholders = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        format!(" AND e.kind IN ({placeholders})")
    } else {
        "".to_string()
    };

    let (step_select, step_join): (String, String) = match direction {
        Direction::Out => (
            "e.to_id AS node_id".to_string(),
            "e.from_id = w.node_id".to_string(),
        ),
        Direction::In => (
            "e.from_id AS node_id".to_string(),
            "e.to_id = w.node_id".to_string(),
        ),
        Direction::Both => (
            "CASE WHEN e.from_id = w.node_id THEN e.to_id ELSE e.from_id END AS node_id"
                .to_string(),
            "(e.from_id = w.node_id OR e.to_id = w.node_id)".to_string(),
        ),
    };

    let sql = format!(
        r#"
WITH RECURSIVE
seed(id) AS (
  SELECT id FROM nodes WHERE id IN ({seed_in})
),
walk(node_id, depth) AS (
  SELECT id, 0 FROM seed
  UNION ALL
  SELECT
    {step_select},
    w.depth + 1 AS depth
  FROM walk w
  JOIN edges e
    ON {step_join}
  WHERE w.depth < ?
    {edge_kind_clause}
),
visited_nodes AS (
  SELECT DISTINCT node_id AS id FROM walk
)
SELECT id FROM visited_nodes
"#
    );

    // Parameter layout (unnumbered '?' placeholders):
    // - seed ids...
    // - max_depth
    // - edge kinds... (optional)
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    params.extend(seed_ids.iter().cloned().map(rusqlite::types::Value::Text));
    params.push(rusqlite::types::Value::Integer(max_depth as i64));
    if let Some(kinds) = &edge_kinds {
        params.extend(kinds.iter().cloned().map(rusqlite::types::Value::Text));
    }

    let param_refs: Vec<&dyn rusqlite::ToSql> =
        params.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
    let mut visited_nodes: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(&param_refs[..])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            visited_nodes.push(id);
        }
    }
    visited_nodes.sort();
    visited_nodes.dedup();

    // Query visited edges separately (keeps this function simple and explicit).
    let edge_sql = format!(
        r#"
WITH RECURSIVE
seed(id) AS (
  SELECT id FROM nodes WHERE id IN ({seed_in})
),
walk(node_id, depth) AS (
  SELECT id, 0 FROM seed
  UNION ALL
  SELECT
    {step_select},
    w.depth + 1 AS depth
  FROM walk w
  JOIN edges e
    ON {step_join}
  WHERE w.depth < ?
    {edge_kind_clause}
),
visited_nodes AS (
  SELECT DISTINCT node_id AS id FROM walk
)
SELECT DISTINCT e.from_id, e.to_id, e.kind, e.provenance
FROM edges e
JOIN visited_nodes v1 ON v1.id = e.from_id
JOIN visited_nodes v2 ON v2.id = e.to_id
WHERE 1=1
  {edge_kind_clause}
ORDER BY e.kind, e.from_id, e.to_id
"#
    );

    let mut visited_edges: Vec<(String, String, String, String)> = Vec::new();
    {
        // For the visited-edges query, the edge kind placeholders occur twice (once in walk, once
        // in the final edge filter), so we must bind the list twice.
        let mut params_edges = params.clone();
        if let Some(kinds) = &edge_kinds {
            params_edges.extend(kinds.iter().cloned().map(rusqlite::types::Value::Text));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_edges
            .iter()
            .map(|p| p as &dyn rusqlite::ToSql)
            .collect();

        let mut stmt = conn.prepare(&edge_sql)?;
        let mut rows = stmt.query(&param_refs[..])?;
        while let Some(row) = rows.next()? {
            let from_id: String = row.get(0)?;
            let to_id: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let prov: String = row.get(3)?;
            visited_edges.push((from_id, to_id, kind, prov));
        }
    }

    Ok((visited_nodes, visited_edges))
}
