//! Read the scan sqlite: symbol lookup + callers with provenance class.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::graph_queries::{open_graph, resolve_symbol_detailed, GraphQueryError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitnessClass {
    Compiler,
    TreeSitter,
    None,
}

impl WitnessClass {
    pub fn as_str(self) -> &'static str {
        match self {
            WitnessClass::Compiler => "compiler",
            WitnessClass::TreeSitter => "treesitter",
            WitnessClass::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolHit {
    pub id: String,
    pub fqn: String,
    pub file: String,
    pub start_line: u32,
}

#[derive(Debug, Clone)]
pub struct CallerHit {
    #[allow(dead_code)]
    pub fqn: String,
    pub class: WitnessClass,
}

pub fn find_symbols(
    artifact: &Path,
    file: &str,
    names: &[String],
    deleted_ranges: &[(u32, u32)],
    whole_file: bool,
) -> Result<Vec<SymbolHit>> {
    let conn = open_graph(artifact).context("open gate artifact")?;
    let mut out = Vec::new();

    for name in names {
        match resolve_symbol_detailed(&conn, name) {
            Ok(r) => {
                let loc = node_location(&conn, &r.node.id)?;
                out.push(SymbolHit {
                    id: r.node.id,
                    fqn: r.node.fqn,
                    file: loc.0,
                    start_line: loc.1,
                });
            }
            Err(GraphQueryError::UnknownSymbol(_))
            | Err(GraphQueryError::AmbiguousSymbol { .. })
            | Err(GraphQueryError::OverlyGenericSymbol { .. })
            | Err(GraphQueryError::SeedNotAFunction { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }

    if !file.is_empty() {
        for hit in nodes_in_file(&conn, file)? {
            let in_range = whole_file
                || deleted_ranges
                    .iter()
                    .any(|(a, b)| hit.start_line >= *a && hit.start_line <= *b)
                || names.iter().any(|n| short_eq(&hit.fqn, n));
            if in_range && !out.iter().any(|e| e.id == hit.id) {
                out.push(hit);
            }
        }
    }

    Ok(out)
}

pub fn callers_with_tier(artifact: &Path, target_id: &str) -> Result<Vec<CallerHit>> {
    let conn = open_graph(artifact)?;
    callers_with_tier_conn(&conn, target_id)
}

pub fn callers_with_tier_conn(conn: &Connection, target_id: &str) -> Result<Vec<CallerHit>> {
    let mut stmt = conn.prepare(
        "SELECT n.fqn, e.provenance FROM edges e
         JOIN nodes n ON n.id = e.from_id
         WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'
         ORDER BY n.fqn ASC",
    )?;
    let rows = stmt
        .query_map([target_id], |row| {
            let fqn: String = row.get(0)?;
            let prov: String = row.get(1)?;
            Ok(CallerHit {
                fqn,
                class: class_from_provenance(&prov),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_same_short_name(artifact: &Path, short: &str) -> Result<Vec<SymbolHit>> {
    let conn = open_graph(artifact)?;
    let mut stmt = conn.prepare(
        "SELECT id, fqn, location FROM nodes
         WHERE kind IN ('Function', 'Method', 'Constructor')",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (id, fqn, loc) in rows {
        if short_eq(&fqn, short) {
            let (file, start_line) = parse_location(&loc);
            out.push(SymbolHit {
                id,
                fqn,
                file,
                start_line,
            });
        }
    }
    Ok(out)
}

fn nodes_in_file(conn: &Connection, file: &str) -> Result<Vec<SymbolHit>> {
    let mut stmt = conn.prepare(
        "SELECT id, fqn, location FROM nodes
         WHERE kind IN ('Function', 'Method', 'Constructor')",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let norm = file.replace('\\', "/");
    let mut out = Vec::new();
    for (id, fqn, loc) in rows {
        let (node_file, start_line) = parse_location(&loc);
        let nf = node_file.replace('\\', "/");
        if nf.ends_with(&norm) || norm.ends_with(&nf) || nf == norm {
            out.push(SymbolHit {
                id,
                fqn,
                file: node_file,
                start_line,
            });
        }
    }
    Ok(out)
}

fn node_location(conn: &Connection, id: &str) -> Result<(String, u32)> {
    let loc: String = conn.query_row("SELECT location FROM nodes WHERE id = ?1", [id], |row| {
        row.get(0)
    })?;
    Ok(parse_location(&loc))
}

fn parse_location(loc: &str) -> (String, u32) {
    let v: serde_json::Value = serde_json::from_str(loc).unwrap_or(serde_json::json!({}));
    let file = v
        .get("file")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let start = v.get("start_line").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    (file, start)
}

pub fn class_from_provenance(json: &str) -> WitnessClass {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or(serde_json::json!({}));
    let source = v
        .get("source")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match source.as_str() {
        "compiler" | "lsp" | "runtime" => WitnessClass::Compiler,
        "treesitter" | "heuristic" => WitnessClass::TreeSitter,
        _ => WitnessClass::None,
    }
}

fn short_eq(fqn: &str, needle: &str) -> bool {
    let a = fqn
        .rsplit([':', '.', '/'])
        .find(|s| !s.is_empty())
        .unwrap_or(fqn);
    let b = needle
        .rsplit([':', '.', '/'])
        .find(|s| !s.is_empty())
        .unwrap_or(needle);
    a.eq_ignore_ascii_case(b)
}
