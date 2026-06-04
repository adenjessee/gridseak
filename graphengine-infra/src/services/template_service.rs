// Template service that processes TOML files and returns JSON data
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::services::template_query::contract::{
    default_capabilities, ExternalsMetadata, QueryMode, TemplateQueryMetadata,
    TemplateQueryPayload, TEMPLATE_QUERY_CONTRACT_VERSION_V1,
};
use crate::services::template_query::filters::{
    parse_edge_filter_v1, parse_node_filter_v1, ParsedEdgeFilter, ParsedNodeFilter,
};
use crate::services::template_query::schema::detect_schema_version;
use crate::services::template_query::seeds::parse_seeds_v1;
use crate::services::template_query::traversal::{resolve_seed_ids, traverse_sql, Direction};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionData {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub code_text: String,
    pub parameters: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileData {
    pub id: String,
    pub path: String,
    pub name: String,
    pub code_content: String,
    pub file_type: String,
    pub line_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyData {
    pub files: Vec<FileHierarchy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHierarchy {
    pub file_path: String,
    pub impl_blocks: Vec<ImplBlock>,
    pub standalone_functions: Vec<FunctionData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplBlock {
    pub id: String,
    pub name: String,
    pub line: u32,
    pub functions: Vec<FunctionData>,
}

struct SqlWhere {
    sql: String,
    params: Vec<String>,
}

fn build_nodes_where_v1(
    has_properties: bool,
    parsed: Option<&ParsedNodeFilter>,
) -> anyhow::Result<SqlWhere> {
    let mut where_sql = String::from("WHERE 1=1");
    let mut params: Vec<String> = Vec::new();

    if let Some(f) = parsed {
        if let Some(kinds) = &f.kinds {
            let placeholders = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            where_sql.push_str(&format!(" AND kind IN ({placeholders})"));
            params.extend(kinds.iter().cloned());
        }

        let needs_props = f.role.is_some()
            || f.is_vendor.is_some()
            || f.is_build_output.is_some()
            || f.is_generated.is_some()
            || f.is_test.is_some()
            || f.path_repo_rel_prefix.is_some();
        if needs_props && !has_properties {
            return Err(anyhow::anyhow!(
                "node_filter requires nodes.properties JSON column, but DB schema has no 'properties' column"
            ));
        }

        if let Some(roles) = &f.role {
            let placeholders = roles.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            where_sql.push_str(&format!(
                " AND json_extract(properties, '$.role') IN ({placeholders})"
            ));
            params.extend(roles.iter().cloned());
        }

        for (key, v) in [
            ("is_vendor", f.is_vendor),
            ("is_build_output", f.is_build_output),
            ("is_generated", f.is_generated),
            ("is_test", f.is_test),
        ] {
            if let Some(expected) = v {
                let expected_num = if expected { 1 } else { 0 };
                where_sql.push_str(&format!(
                    " AND COALESCE(json_extract(properties, '$.{key}'), 0) = {expected_num}"
                ));
            }
        }

        if let Some(prefix) = &f.path_repo_rel_prefix {
            where_sql.push_str(" AND json_extract(properties, '$.path_repo_rel') LIKE ?");
            params.push(format!("{}%", prefix.trim_start_matches('/')));
        }
    }

    Ok(SqlWhere {
        sql: where_sql,
        params,
    })
}

fn build_edges_where_v1(parsed: Option<&ParsedEdgeFilter>) -> SqlWhere {
    let mut where_sql = String::from("WHERE 1=1");
    let mut params: Vec<String> = Vec::new();

    if let Some(f) = parsed {
        if let Some(kinds) = &f.rel_kinds {
            let placeholders = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            where_sql.push_str(&format!(" AND kind IN ({placeholders})"));
            params.extend(kinds.iter().cloned());
        }
    }

    SqlWhere {
        sql: where_sql,
        params,
    }
}

fn node_matches_filter(kind: &str, properties: &serde_json::Value, f: &ParsedNodeFilter) -> bool {
    if let Some(kinds) = &f.kinds {
        if !kinds.iter().any(|k| k == kind) {
            return false;
        }
    }
    if let Some(roles) = &f.role {
        let role = properties
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !roles.iter().any(|r| r == role) {
            return false;
        }
    }
    for (key, expected) in [
        ("is_vendor", f.is_vendor),
        ("is_build_output", f.is_build_output),
        ("is_generated", f.is_generated),
        ("is_test", f.is_test),
    ] {
        if let Some(exp) = expected {
            let actual = properties
                .get(key)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if actual != exp {
                return false;
            }
        }
    }
    if let Some(prefix) = &f.path_repo_rel_prefix {
        let p = properties
            .get("path_repo_rel")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !p.starts_with(prefix.trim_start_matches('/')) {
            return false;
        }
    }
    true
}

fn materialize_stub_nodes(
    conn: &Connection,
    has_properties: bool,
    nodes: &mut Vec<serde_json::Value>,
    selected_node_ids: &mut std::collections::HashSet<String>,
    edges: &[serde_json::Value],
) -> Result<(), anyhow::Error> {
    let mut missing_ids: Vec<String> = Vec::new();
    for e in edges {
        let src = e.get("source").and_then(|v| v.as_str());
        let dst = e.get("target").and_then(|v| v.as_str());
        for id in [src, dst].into_iter().flatten() {
            if !selected_node_ids.contains(id) {
                missing_ids.push(id.to_string());
            }
        }
    }
    missing_ids.sort();
    missing_ids.dedup();
    if missing_ids.is_empty() {
        return Ok(());
    }

    let placeholders = missing_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = if has_properties {
        format!(
            "SELECT id, kind, fqn, location, provenance, properties FROM nodes WHERE id IN ({placeholders}) ORDER BY kind, fqn, id"
        )
    } else {
        format!(
            "SELECT id, kind, fqn, location, provenance FROM nodes WHERE id IN ({placeholders}) ORDER BY kind, fqn, id"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = missing_ids
        .iter()
        .map(|p| p as &dyn rusqlite::ToSql)
        .collect();
    let mut rows = stmt.query(&param_refs[..])?;
    while let Some(row) = rows.next()? {
        let node_id: String = row.get(0)?;
        let node_kind: String = row.get(1)?;
        let node_fqn: String = row.get(2)?;
        let node_location: String = row.get(3)?;
        let node_provenance: String = row.get(4)?;
        let mut properties_value = if has_properties {
            let node_properties: String = row.get(5)?;
            serde_json::from_str::<serde_json::Value>(&node_properties)
                .unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        if let Some(obj) = properties_value.as_object_mut() {
            obj.insert("is_stub".to_string(), serde_json::Value::Bool(true));
            obj.insert(
                "stub_reason".to_string(),
                serde_json::Value::String("external_endpoint".to_string()),
            );
        }

        nodes.push(serde_json::json!({
            "id": node_id.clone(),
            "type": node_kind,
            "fqn": node_fqn,
            "location": serde_json::from_str::<serde_json::Value>(&node_location).unwrap_or(serde_json::Value::Null),
            "provenance": serde_json::from_str::<serde_json::Value>(&node_provenance).unwrap_or(serde_json::Value::Null),
            "properties": properties_value
        }));
        selected_node_ids.insert(node_id);
    }

    Ok(())
}

fn sort_nodes_edges(nodes: &mut [serde_json::Value], edges: &mut [serde_json::Value]) {
    nodes.sort_by(|a, b| {
        let ak = (
            a.get("type").and_then(|v| v.as_str()).unwrap_or(""),
            a.get("fqn").and_then(|v| v.as_str()).unwrap_or(""),
            a.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        );
        let bk = (
            b.get("type").and_then(|v| v.as_str()).unwrap_or(""),
            b.get("fqn").and_then(|v| v.as_str()).unwrap_or(""),
            b.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        );
        ak.cmp(&bk)
    });
    edges.sort_by(|a, b| {
        let ak = (
            a.get("type").and_then(|v| v.as_str()).unwrap_or(""),
            a.get("source").and_then(|v| v.as_str()).unwrap_or(""),
            a.get("target").and_then(|v| v.as_str()).unwrap_or(""),
        );
        let bk = (
            b.get("type").and_then(|v| v.as_str()).unwrap_or(""),
            b.get("source").and_then(|v| v.as_str()).unwrap_or(""),
            b.get("target").and_then(|v| v.as_str()).unwrap_or(""),
        );
        ak.cmp(&bk)
    });
}

#[allow(clippy::too_many_arguments)]
fn build_explain_block(
    query_mode: QueryMode,
    depth: i32,
    direction: &str,
    show_externals: bool,
    seed_spec: Option<&toml::Value>,
    resolved_seed_ids: Option<Vec<String>>,
    node_filter: Option<&str>,
    edge_filter: Option<&str>,
) -> serde_json::Value {
    let seed_json = seed_spec
        .and_then(|s| serde_json::to_value(s).ok())
        .unwrap_or(serde_json::Value::Null);

    serde_json::json!({
        "query_mode": query_mode,
        "graph": {
            "depth": depth,
            "direction": direction,
            "show_externals": show_externals,
            "node_filter_raw": node_filter,
            "edge_filter_raw": edge_filter,
        },
        "seed": {
            "raw": seed_json,
            "resolved_seed_ids": resolved_seed_ids,
            "resolved_seed_count": resolved_seed_ids.as_ref().map(|v| v.len()),
        }
    })
}

pub struct TemplateService {
    db_path: String,
}

impl TemplateService {
    pub fn new(db_path: &str) -> Result<Self, anyhow::Error> {
        Ok(Self {
            db_path: db_path.to_string(),
        })
    }

    pub fn get_custom_graph(&self, path: &std::path::Path) -> Result<String, anyhow::Error> {
        self.get_custom_graph_with_explain(path, false)
    }

    pub fn get_custom_graph_with_explain(
        &self,
        path: &std::path::Path,
        explain: bool,
    ) -> Result<String, anyhow::Error> {
        println!("[TEMPLATE_SERVICE] Processing template: {}", path.display());

        // Parse the TOML template file
        let template_content = std::fs::read_to_string(path)?;
        let template: toml::Value = toml::from_str(&template_content)?;

        // Check if this is a hierarchy display template
        if path.to_string_lossy().contains("hierarchy_display") {
            self.get_hierarchy_data()
        } else {
            // Process the template and query the database
            self.process_template(&template, explain)
        }
    }

    fn process_template(
        &self,
        template: &toml::Value,
        explain: bool,
    ) -> Result<String, anyhow::Error> {
        let conn = Connection::open(&self.db_path)?;

        let graph_spec = template
            .get("graph")
            .ok_or_else(|| anyhow::anyhow!("Missing [graph] section"))?;

        let mode_raw = graph_spec
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .unwrap_or("filtered_dump");
        let query_mode = match mode_raw {
            "filtered_dump" => QueryMode::FilteredDump,
            "traversal" => QueryMode::Traversal,
            other => {
                return Err(anyhow::anyhow!(
                    "Unsupported graph.mode '{other}'. Supported: filtered_dump | traversal"
                ))
            }
        };

        let depth = graph_spec
            .get("depth")
            .and_then(|v| v.as_integer())
            .unwrap_or(3) as i32;
        let direction_raw = graph_spec
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("out");
        let show_externals = graph_spec
            .get("show_externals")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let node_filter_raw = graph_spec.get("node_filter").and_then(|v| v.as_str());
        let edge_filter_raw = graph_spec.get("edge_filter").and_then(|v| v.as_str());

        println!(
            "[TEMPLATE_SERVICE] Processing with mode={:?}, depth={}, direction={}, show_externals={}",
            query_mode, depth, direction_raw, show_externals
        );

        let schema_version = detect_schema_version(&conn);
        let has_properties: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('nodes') WHERE name='properties'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        // Strict contract-v1 parsing/validation for filters (fail fast; no silent ignores).
        let parsed_node_filter: Option<ParsedNodeFilter> = match node_filter_raw {
            None => None,
            Some(raw) => Some(parse_node_filter_v1(raw)?),
        };
        let parsed_edge_filter: Option<ParsedEdgeFilter> = match edge_filter_raw {
            None => None,
            Some(raw) => Some(parse_edge_filter_v1(raw)?),
        };

        let caps = default_capabilities(true);
        let mut warnings: Vec<String> = Vec::new();
        if graph_spec.get("mode").is_none() {
            warnings.push("graph.mode omitted; defaulted to 'filtered_dump'".to_string());
        }
        if query_mode == QueryMode::FilteredDump && (depth != 3 || direction_raw != "out") {
            warnings.push("depth/direction are ignored in graph.mode='filtered_dump'".to_string());
        }

        let mut resolved_seed_ids_for_explain: Option<Vec<String>> = None;

        let mut nodes: Vec<serde_json::Value> = Vec::new();
        let mut edges: Vec<serde_json::Value> = Vec::new();
        let mut selected_node_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut recommended_view_roots: Option<serde_json::Value> = None;

        match query_mode {
            QueryMode::FilteredDump => {
                let nodes_where =
                    build_nodes_where_v1(has_properties, parsed_node_filter.as_ref())?;
                // Guardrails: fail fast before materializing huge payloads.
                let node_count: i64 = conn.query_row(
                    &format!("SELECT COUNT(*) FROM nodes {}", nodes_where.sql),
                    nodes_where
                        .params
                        .iter()
                        .map(|p| p as &dyn rusqlite::ToSql)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    |row| row.get(0),
                )?;
                if node_count as usize > caps.limits.max_nodes {
                    return Err(anyhow::anyhow!(
                        "Result exceeds max_nodes ({} > {}). Refine seeds/filters.",
                        node_count,
                        caps.limits.max_nodes
                    ));
                }

                let node_query = if has_properties {
                    format!(
                        "SELECT id, kind, fqn, location, provenance, properties FROM nodes {} ORDER BY kind, fqn, id",
                        nodes_where.sql
                    )
                } else {
                    format!(
                        "SELECT id, kind, fqn, location, provenance FROM nodes {} ORDER BY kind, fqn, id",
                        nodes_where.sql
                    )
                };

                let mut stmt = conn.prepare(&node_query)?;
                let node_param_refs: Vec<&dyn rusqlite::ToSql> = nodes_where
                    .params
                    .iter()
                    .map(|p| p as &dyn rusqlite::ToSql)
                    .collect();
                let mut rows = stmt.query(&node_param_refs[..])?;

                while let Some(row) = rows.next()? {
                    let node_id: String = row.get(0)?;
                    let node_kind: String = row.get(1)?;
                    let node_fqn: String = row.get(2)?;
                    let node_location: String = row.get(3)?;
                    let node_provenance: String = row.get(4)?;
                    let properties_value = if has_properties {
                        let node_properties: String = row.get(5)?;
                        serde_json::from_str::<serde_json::Value>(&node_properties)
                            .unwrap_or(serde_json::json!({}))
                    } else {
                        serde_json::json!({})
                    };

                    if recommended_view_roots.is_none() && node_kind == "Project" {
                        if let Some(v) = properties_value.get("recommended_view_roots") {
                            recommended_view_roots = Some(v.clone());
                        }
                    }

                    nodes.push(serde_json::json!({
                        "id": node_id.clone(),
                        "type": node_kind,
                        "fqn": node_fqn,
                        "location": serde_json::from_str::<serde_json::Value>(&node_location).unwrap_or(serde_json::Value::Null),
                        "provenance": serde_json::from_str::<serde_json::Value>(&node_provenance).unwrap_or(serde_json::Value::Null),
                        "properties": properties_value
                    }));
                    selected_node_ids.insert(node_id);
                }

                let edges_where = build_edges_where_v1(parsed_edge_filter.as_ref());
                // Guardrails: edge count (uses externals mode semantics).
                let edge_count_query = if show_externals {
                    format!("SELECT COUNT(*) FROM edges {}", edges_where.sql)
                } else {
                    format!(
                        "WITH selected_nodes AS (SELECT id FROM nodes {}) \
                         SELECT COUNT(*) \
                         FROM edges e {} \
                         AND e.from_id IN (SELECT id FROM selected_nodes) \
                         AND e.to_id IN (SELECT id FROM selected_nodes)",
                        nodes_where.sql, edges_where.sql
                    )
                };
                let combined_edge_params: Vec<String> = if show_externals {
                    edges_where.params.clone()
                } else {
                    let mut combined = nodes_where.params.clone();
                    combined.extend(edges_where.params.clone());
                    combined
                };
                let edge_count: i64 = conn.query_row(
                    &edge_count_query,
                    combined_edge_params
                        .iter()
                        .map(|p| p as &dyn rusqlite::ToSql)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    |row| row.get(0),
                )?;
                if edge_count as usize > caps.limits.max_edges {
                    return Err(anyhow::anyhow!(
                        "Result exceeds max_edges ({} > {}). Refine seeds/filters.",
                        edge_count,
                        caps.limits.max_edges
                    ));
                }

                let edge_query = if show_externals {
                    format!(
                        "SELECT from_id, to_id, kind, provenance FROM edges {} ORDER BY kind, from_id, to_id",
                        edges_where.sql
                    )
                } else {
                    format!(
                        "WITH selected_nodes AS (SELECT id FROM nodes {}) \
                         SELECT e.from_id, e.to_id, e.kind, e.provenance \
                         FROM edges e {} \
                         AND e.from_id IN (SELECT id FROM selected_nodes) \
                         AND e.to_id IN (SELECT id FROM selected_nodes) \
                         ORDER BY e.kind, e.from_id, e.to_id",
                        nodes_where.sql, edges_where.sql
                    )
                };

                let mut stmt = conn.prepare(&edge_query)?;
                let combined_edge_params: Vec<String> = if show_externals {
                    edges_where.params.clone()
                } else {
                    let mut combined = nodes_where.params.clone();
                    combined.extend(edges_where.params.clone());
                    combined
                };
                let edge_param_refs: Vec<&dyn rusqlite::ToSql> = combined_edge_params
                    .iter()
                    .map(|p| p as &dyn rusqlite::ToSql)
                    .collect();
                let mut rows = stmt.query(&edge_param_refs[..])?;

                while let Some(row) = rows.next()? {
                    let edge_from: String = row.get(0)?;
                    let edge_to: String = row.get(1)?;
                    let edge_kind: String = row.get(2)?;
                    let edge_provenance: String = row.get(3)?;

                    if !show_externals {
                        let in_set = selected_node_ids.contains(&edge_from)
                            && selected_node_ids.contains(&edge_to);
                        if !in_set {
                            continue;
                        }
                    }

                    edges.push(serde_json::json!({
                        "source": edge_from,
                        "target": edge_to,
                        "type": edge_kind,
                        "provenance": serde_json::from_str::<serde_json::Value>(&edge_provenance).unwrap_or(serde_json::Value::Null)
                    }));
                }
            }

            QueryMode::Traversal => {
                // Validate depth/direction and require seeds.
                if depth < 0 {
                    return Err(anyhow::anyhow!("graph.depth must be >= 0"));
                }
                if depth > caps.limits.max_depth {
                    return Err(anyhow::anyhow!(
                        "graph.depth {depth} exceeds max_depth {}",
                        caps.limits.max_depth
                    ));
                }
                let direction = Direction::parse(direction_raw).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Invalid graph.direction '{direction_raw}'. Supported: out | in | both"
                    )
                })?;

                let seed_spec = template
                    .get("seed")
                    .ok_or_else(|| anyhow::anyhow!("Traversal mode requires a [seed] section"))?;
                if seed_spec.get("pattern").is_some() {
                    return Err(anyhow::anyhow!(
                        "Traversal mode does not support seed.pattern. Use seed.roots=[{{ by_path_repo_rel = \"...\" }}, ...] or by_id."
                    ));
                }
                let seeds = parse_seeds_v1(seed_spec)?;

                // Ensure node_filter does not require properties in a DB that doesn't have them.
                let _ = build_nodes_where_v1(has_properties, parsed_node_filter.as_ref())?;

                let seed_ids = resolve_seed_ids(&conn, &seeds)?;
                if seed_ids.is_empty() {
                    return Err(anyhow::anyhow!("Traversal seeds resolved to 0 nodes"));
                }
                resolved_seed_ids_for_explain = Some(seed_ids.clone());

                if parsed_edge_filter.is_none() {
                    warnings.push("Traversal with no edge_filter may be very broad".to_string());
                }

                let (visited_ids, visited_edges) = traverse_sql(
                    &conn,
                    &seed_ids,
                    depth,
                    direction,
                    parsed_edge_filter.as_ref(),
                )?;
                // Guardrails: visited counts are an upper bound on emitted.
                if visited_ids.len() > caps.limits.max_nodes {
                    return Err(anyhow::anyhow!(
                        "Result exceeds max_nodes ({} > {}). Refine seeds/filters.",
                        visited_ids.len(),
                        caps.limits.max_nodes
                    ));
                }
                if visited_edges.len() > caps.limits.max_edges {
                    return Err(anyhow::anyhow!(
                        "Result exceeds max_edges ({} > {}). Refine seeds/filters.",
                        visited_edges.len(),
                        caps.limits.max_edges
                    ));
                }

                // Fetch visited nodes; node_filter gates emission (Semantics A).
                let placeholders = visited_ids
                    .iter()
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(",");
                let node_query = if has_properties {
                    format!(
                        "SELECT id, kind, fqn, location, provenance, properties FROM nodes WHERE id IN ({placeholders}) ORDER BY kind, fqn, id"
                    )
                } else {
                    format!(
                        "SELECT id, kind, fqn, location, provenance FROM nodes WHERE id IN ({placeholders}) ORDER BY kind, fqn, id"
                    )
                };

                let mut stmt = conn.prepare(&node_query)?;
                let node_param_refs: Vec<&dyn rusqlite::ToSql> = visited_ids
                    .iter()
                    .map(|p| p as &dyn rusqlite::ToSql)
                    .collect();
                let mut rows = stmt.query(&node_param_refs[..])?;
                while let Some(row) = rows.next()? {
                    let node_id: String = row.get(0)?;
                    let node_kind: String = row.get(1)?;
                    let node_fqn: String = row.get(2)?;
                    let node_location: String = row.get(3)?;
                    let node_provenance: String = row.get(4)?;
                    let properties_value = if has_properties {
                        let node_properties: String = row.get(5)?;
                        serde_json::from_str::<serde_json::Value>(&node_properties)
                            .unwrap_or(serde_json::json!({}))
                    } else {
                        serde_json::json!({})
                    };

                    if recommended_view_roots.is_none() && node_kind == "Project" {
                        if let Some(v) = properties_value.get("recommended_view_roots") {
                            recommended_view_roots = Some(v.clone());
                        }
                    }

                    let emit = if let Some(f) = parsed_node_filter.as_ref() {
                        node_matches_filter(&node_kind, &properties_value, f)
                    } else {
                        true
                    };
                    if !emit {
                        continue;
                    }

                    nodes.push(serde_json::json!({
                        "id": node_id.clone(),
                        "type": node_kind,
                        "fqn": node_fqn,
                        "location": serde_json::from_str::<serde_json::Value>(&node_location).unwrap_or(serde_json::Value::Null),
                        "provenance": serde_json::from_str::<serde_json::Value>(&node_provenance).unwrap_or(serde_json::Value::Null),
                        "properties": properties_value
                    }));
                    selected_node_ids.insert(node_id);
                }

                for (from_id, to_id, kind, prov) in visited_edges {
                    edges.push(serde_json::json!({
                        "source": from_id,
                        "target": to_id,
                        "type": kind,
                        "provenance": serde_json::from_str::<serde_json::Value>(&prov).unwrap_or(serde_json::Value::Null)
                    }));
                }

                if !show_externals {
                    edges.retain(|e| {
                        let src = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
                        let dst = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
                        selected_node_ids.contains(src) && selected_node_ids.contains(dst)
                    });
                }
            }
        }

        if show_externals {
            materialize_stub_nodes(
                &conn,
                has_properties,
                &mut nodes,
                &mut selected_node_ids,
                &edges,
            )?;
        }

        sort_nodes_edges(&mut nodes, &mut edges);

        let externals_mode = if show_externals {
            "stubs".to_string()
        } else {
            "closed_subgraph".to_string()
        };
        let mut metadata = TemplateQueryMetadata {
            contract_version: TEMPLATE_QUERY_CONTRACT_VERSION_V1.to_string(),
            query_mode,
            schema_version,
            externals: ExternalsMetadata {
                show_externals,
                mode: externals_mode,
            },
            capabilities: caps,
            recommended_view_roots: recommended_view_roots
                .unwrap_or(serde_json::Value::Array(vec![])),
            warnings,
            explain: None,
        };

        if explain {
            metadata.explain = Some(build_explain_block(
                query_mode,
                depth,
                direction_raw,
                show_externals,
                template.get("seed"),
                resolved_seed_ids_for_explain,
                node_filter_raw,
                edge_filter_raw,
            ));
        }

        let payload = TemplateQueryPayload {
            nodes,
            edges,
            metadata,
        };
        Ok(serde_json::to_string(&payload)?)
    }

    fn get_hierarchy_data(&self) -> Result<String, anyhow::Error> {
        let conn = Connection::open(&self.db_path)?;

        // Query to get files with their impl blocks and functions
        // First get all impl blocks, then get their functions separately to avoid duplicates
        let impl_query = r#"
            SELECT 
                impl.id as impl_id,
                impl.properties as impl_properties
            FROM node impl
            WHERE impl.node_type = 'Impl'
            AND json_extract(impl.properties, '$.provenance') != 'external'
            ORDER BY json_extract(impl.properties, '$.file_path'), json_extract(impl.properties, '$.line')
        "#;

        let func_query = r#"
            SELECT 
                impl.id as impl_id,
                func.id as func_id,
                func.properties as func_properties
            FROM node impl
            JOIN edge e2 ON impl.id = e2.src AND e2.rel_id = 1  -- Contains relationship
            JOIN node func ON e2.dst = func.id AND func.node_type = 'Function'
            WHERE impl.node_type = 'Impl'
            AND json_extract(impl.properties, '$.provenance') != 'external'
            ORDER BY json_extract(impl.properties, '$.file_path'), json_extract(impl.properties, '$.line'), json_extract(func.properties, '$.line')
        "#;

        let mut file_map: HashMap<String, FileHierarchy> = HashMap::new();

        // First pass: Get all impl blocks (no duplicates)
        let mut stmt = conn.prepare(impl_query)?;
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let impl_id: i64 = row.get(0)?;
            let impl_properties: String = row.get(1)?;

            // Parse impl properties to get file path
            let impl_props: serde_json::Value = serde_json::from_str(&impl_properties)?;
            let file_path = impl_props["file_path"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();

            // Get or create file hierarchy
            let file_hierarchy =
                file_map
                    .entry(file_path.clone())
                    .or_insert_with(|| FileHierarchy {
                        file_path: file_path.clone(),
                        impl_blocks: Vec::new(),
                        standalone_functions: Vec::new(),
                    });

            // Process impl block
            let impl_name = impl_props["name"].as_str().unwrap_or("unknown").to_string();
            let impl_line = impl_props["line"].as_u64().unwrap_or(0) as u32;

            // Create impl block (guaranteed unique since we're processing each impl block only once)
            let new_impl_block = ImplBlock {
                id: impl_id.to_string(),
                name: impl_name,
                line: impl_line,
                functions: Vec::new(),
            };

            file_hierarchy.impl_blocks.push(new_impl_block);
        }

        // Second pass: Get all functions and associate them with their impl blocks
        let mut stmt = conn.prepare(func_query)?;
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let impl_id: i64 = row.get(0)?;
            let func_id: i64 = row.get(1)?;
            let func_properties: String = row.get(2)?;

            // Parse function properties
            let func_props: serde_json::Value = serde_json::from_str(&func_properties)?;
            let func_name = func_props["name"].as_str().unwrap_or("unknown").to_string();
            let func_line = func_props["line"].as_u64().unwrap_or(0) as u32;
            let file_path = func_props["file_path"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();

            // Find the impl block and add the function to it
            if let Some(file_hierarchy) = file_map.get_mut(&file_path) {
                for impl_block in &mut file_hierarchy.impl_blocks {
                    if impl_block.id == impl_id.to_string() {
                        let function_data = FunctionData {
                            id: func_id.to_string(),
                            name: func_name,
                            file_path: file_path.clone(),
                            line: func_line,
                            column: 0,
                            code_text: String::new(),
                            parameters: None,
                            start_line: func_line as i32,
                            end_line: func_line as i32,
                        };

                        impl_block.functions.push(function_data);
                        break;
                    }
                }
            }
        }

        // Convert to hierarchy data
        let hierarchy_data = HierarchyData {
            files: file_map.into_values().collect(),
        };

        // Serialize to JSON
        let json_response = serde_json::to_string(&hierarchy_data)?;
        Ok(json_response)
    }

    pub fn get_all_functions(&self) -> Result<Vec<FunctionData>, anyhow::Error> {
        Ok(vec![])
    }

    pub fn get_all_files(&self) -> Result<Vec<FileData>, anyhow::Error> {
        Ok(vec![])
    }

    pub fn get_graph_by_repo_id(&self, _repo_id: i64) -> Result<Vec<FunctionData>, anyhow::Error> {
        Ok(vec![])
    }
}
