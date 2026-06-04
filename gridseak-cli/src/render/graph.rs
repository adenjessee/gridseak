//! Output rendering for `gridseak graph *` commands.
//!
//! The graph commands all share one shape: "here is a labelled list
//! of nodes (with optional secondary numbers like fan-count, depth,
//! edge-count) and here is the artifact/scan that produced them".
//! We collapse that into a `GraphView` envelope so every command's
//! output is structurally identical across formats — only the title
//! and column labels change.

use std::io::{self, Write};

use serde::Serialize;

use crate::graph_queries::{
    CoupledModuleRow, CycleRow, FanRow, FileBlastRadiusResult, NodeRef, SliceNode,
};

#[derive(Debug, Clone, Serialize)]
pub struct GraphView {
    pub title: String,
    pub scan_id: String,
    pub artifact_path: String,
    pub repo_name: String,
    pub body: GraphBody,
}

#[allow(dead_code)] // Empty variant kept for future use by `path` / `slice` when seed not in graph
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphBody {
    Fan {
        rows: Vec<FanRow>,
        count_label: String,
    },
    Nodes {
        rows: Vec<NodeRef>,
    },
    Slice {
        rows: Vec<SliceNode>,
    },
    FileBlastRadius {
        result: FileBlastRadiusResult,
    },
    ModuleCoupling {
        rows: Vec<CoupledModuleRow>,
    },
    Cycles {
        rows: Vec<CycleRow>,
    },
    Path {
        rows: Vec<NodeRef>,
        from: NodeRef,
        to: NodeRef,
    },
    Empty {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum GraphFormat {
    Table,
    Markdown,
    Json,
    ForLlm,
}

pub fn render(format: GraphFormat, view: &GraphView, out: &mut dyn Write) -> io::Result<()> {
    match format {
        GraphFormat::Table => render_table(view, out),
        GraphFormat::Markdown => render_markdown(view, out),
        GraphFormat::Json => render_json(view, out),
        GraphFormat::ForLlm => render_llm(view, out),
    }
}

fn render_table(view: &GraphView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "{}", view.title)?;
    writeln!(out, "repo:     {}", view.repo_name)?;
    writeln!(out, "scan:     {}", view.scan_id)?;
    writeln!(out, "artifact: {}", view.artifact_path)?;
    writeln!(out)?;
    match &view.body {
        GraphBody::Fan { rows, count_label } => {
            if rows.is_empty() {
                writeln!(out, "(no rows.)")?;
                return Ok(());
            }
            let mut max_count = count_label.len();
            let mut max_fqn = "Function".len();
            for r in rows {
                max_count = max_count.max(r.count.to_string().len());
                max_fqn = max_fqn.max(r.node.fqn.len());
            }
            writeln!(
                out,
                "{:>w$}  {:<f$}  id",
                count_label,
                "Function",
                w = max_count,
                f = max_fqn
            )?;
            for r in rows {
                writeln!(
                    out,
                    "{:>w$}  {:<f$}  {}",
                    r.count,
                    r.node.fqn,
                    r.node.id,
                    w = max_count,
                    f = max_fqn
                )?;
            }
        }
        GraphBody::Nodes { rows } => {
            if rows.is_empty() {
                writeln!(out, "(no rows.)")?;
                return Ok(());
            }
            for n in rows {
                writeln!(out, "  {} ({}) :: {}", n.fqn, n.kind, n.id)?;
            }
        }
        GraphBody::Slice { rows } => {
            if rows.is_empty() {
                writeln!(out, "(no rows.)")?;
                return Ok(());
            }
            for r in rows {
                let arrow = match r.direction {
                    crate::graph_queries::SliceDirection::Upstream => "← caller",
                    crate::graph_queries::SliceDirection::Downstream => "→ callee",
                };
                let tier = r.edge_evidence_tier.as_deref().unwrap_or("tier_?");
                writeln!(
                    out,
                    "  d{depth} {arrow} [{tier}] {fqn} :: {id}",
                    depth = r.depth,
                    arrow = arrow,
                    tier = tier,
                    fqn = r.node.fqn,
                    id = r.node.id,
                )?;
            }
        }
        GraphBody::FileBlastRadius { result } => {
            writeln!(out, "file: {}", result.file_path)?;
            writeln!(out, "seeds ({}):", result.seeds.len())?;
            for s in &result.seeds {
                writeln!(out, "  - {} :: {}", s.fqn, s.id)?;
            }
            writeln!(out)?;
            if result.cap_hit {
                writeln!(out, "(cap hit — partial upstream set)")?;
            }
            if result.rows.is_empty() {
                writeln!(out, "(no upstream callers within depth/cap.)")?;
                return Ok(());
            }
            for r in &result.rows {
                let tier = r.edge_evidence_tier.as_deref().unwrap_or("tier_?");
                let via = r
                    .via_seeds
                    .iter()
                    .map(|s| s.fqn.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(
                    out,
                    "  d{depth} ← [{tier}] {fqn} via [{via}] :: {id}",
                    depth = r.depth,
                    tier = tier,
                    fqn = r.node.fqn,
                    via = via,
                    id = r.node.id,
                )?;
            }
        }
        GraphBody::ModuleCoupling { rows } => {
            if rows.is_empty() {
                writeln!(out, "(no cross-module Call edges in this scan.)")?;
                return Ok(());
            }
            let mut max_edges = "edges".len();
            for r in rows {
                max_edges = max_edges.max(r.edge_count.to_string().len());
            }
            writeln!(out, "{:>w$}  from → to", "edges", w = max_edges)?;
            for r in rows {
                writeln!(
                    out,
                    "{:>w$}  {} → {}",
                    r.edge_count,
                    r.from.fqn,
                    r.to.fqn,
                    w = max_edges
                )?;
            }
        }
        GraphBody::Cycles { rows } => {
            if rows.is_empty() {
                writeln!(out, "(no cycles detected in this scan.)")?;
                return Ok(());
            }
            for (i, c) in rows.iter().enumerate() {
                writeln!(out, "#{} length={}", i + 1, c.length)?;
                for m in &c.members {
                    writeln!(out, "  - {} :: {}", m.fqn, m.id)?;
                }
                writeln!(out)?;
            }
        }
        GraphBody::Path { rows, from, to } => {
            writeln!(out, "from: {}", from.fqn)?;
            writeln!(out, "to:   {}", to.fqn)?;
            writeln!(out)?;
            if rows.is_empty() {
                writeln!(out, "(no Call-edge path within the search depth.)")?;
                return Ok(());
            }
            for (i, n) in rows.iter().enumerate() {
                writeln!(out, "  {:>2}. {}", i + 1, n.fqn)?;
            }
        }
        GraphBody::Empty { reason } => {
            writeln!(out, "(empty: {reason})")?;
        }
    }
    Ok(())
}

fn render_markdown(view: &GraphView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "# {}", view.title)?;
    writeln!(out)?;
    writeln!(out, "- **Repo:** {}", view.repo_name)?;
    writeln!(out, "- **Scan:** `{}`", view.scan_id)?;
    writeln!(out, "- **Artifact:** `{}`", view.artifact_path)?;
    writeln!(out)?;
    match &view.body {
        GraphBody::Fan { rows, count_label } => {
            if rows.is_empty() {
                writeln!(out, "_No rows._")?;
                return Ok(());
            }
            writeln!(out, "| {} | Function | Id |", count_label)?;
            writeln!(out, "|---:|----------|----|")?;
            for r in rows {
                writeln!(out, "| {} | `{}` | `{}` |", r.count, r.node.fqn, r.node.id)?;
            }
        }
        GraphBody::Nodes { rows } => {
            if rows.is_empty() {
                writeln!(out, "_No rows._")?;
                return Ok(());
            }
            writeln!(out, "| Function | Kind | Id |")?;
            writeln!(out, "|----------|------|----|")?;
            for n in rows {
                writeln!(out, "| `{}` | {} | `{}` |", n.fqn, n.kind, n.id)?;
            }
        }
        GraphBody::Slice { rows } => {
            if rows.is_empty() {
                writeln!(out, "_No rows._")?;
                return Ok(());
            }
            writeln!(out, "| Depth | Direction | Evidence | Function | Id |")?;
            writeln!(out, "|------:|-----------|----------|----------|----|")?;
            for r in rows {
                let dir = match r.direction {
                    crate::graph_queries::SliceDirection::Upstream => "upstream",
                    crate::graph_queries::SliceDirection::Downstream => "downstream",
                };
                let tier = r.edge_evidence_tier.as_deref().unwrap_or("tier_?");
                writeln!(
                    out,
                    "| {} | {} | {} | `{}` | `{}` |",
                    r.depth, dir, tier, r.node.fqn, r.node.id
                )?;
            }
        }
        GraphBody::FileBlastRadius { result } => {
            writeln!(out, "- **File:** `{}`", result.file_path)?;
            writeln!(out, "- **Seeds:** {}", result.seeds.len())?;
            for s in &result.seeds {
                writeln!(out, "  - `{}`", s.fqn)?;
            }
            writeln!(out)?;
            if result.cap_hit {
                writeln!(out, "_Cap hit — partial upstream set._")?;
            }
            if result.rows.is_empty() {
                writeln!(out, "_No upstream callers._")?;
                return Ok(());
            }
            writeln!(out, "| Depth | Evidence | Function | Via seeds | Id |")?;
            writeln!(out, "|------:|----------|----------|-----------|----|")?;
            for r in &result.rows {
                let tier = r.edge_evidence_tier.as_deref().unwrap_or("tier_?");
                let via = r
                    .via_seeds
                    .iter()
                    .map(|s| format!("`{}`", s.fqn))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(
                    out,
                    "| {} | {} | `{}` | {} | `{}` |",
                    r.depth, tier, r.node.fqn, via, r.node.id
                )?;
            }
        }
        GraphBody::ModuleCoupling { rows } => {
            if rows.is_empty() {
                writeln!(out, "_No cross-module Call edges._")?;
                return Ok(());
            }
            writeln!(out, "| Edges | From | To |")?;
            writeln!(out, "|------:|------|----|")?;
            for r in rows {
                writeln!(
                    out,
                    "| {} | `{}` | `{}` |",
                    r.edge_count, r.from.fqn, r.to.fqn
                )?;
            }
        }
        GraphBody::Cycles { rows } => {
            if rows.is_empty() {
                writeln!(out, "_No cycles detected._")?;
                return Ok(());
            }
            for (i, c) in rows.iter().enumerate() {
                writeln!(out, "### Cycle #{} (length {})", i + 1, c.length)?;
                for m in &c.members {
                    writeln!(out, "- `{}`", m.fqn)?;
                }
                writeln!(out)?;
            }
        }
        GraphBody::Path { rows, from, to } => {
            writeln!(out, "- **From:** `{}`", from.fqn)?;
            writeln!(out, "- **To:** `{}`", to.fqn)?;
            writeln!(out)?;
            if rows.is_empty() {
                writeln!(out, "_No Call-edge path._")?;
                return Ok(());
            }
            for (i, n) in rows.iter().enumerate() {
                writeln!(out, "{}. `{}`", i + 1, n.fqn)?;
            }
        }
        GraphBody::Empty { reason } => {
            writeln!(out, "_Empty: {reason}_")?;
        }
    }
    Ok(())
}

fn render_json(view: &GraphView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.graph.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn render_llm(view: &GraphView, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "[gridseak graph repo={} scan={}]",
        view.repo_name, view.scan_id
    )?;
    writeln!(out, "{}", view.title)?;
    match &view.body {
        GraphBody::Fan { rows, count_label } => {
            for r in rows {
                writeln!(
                    out,
                    "{}={}  {}  id={}",
                    count_label, r.count, r.node.fqn, r.node.id
                )?;
            }
        }
        GraphBody::Nodes { rows } => {
            for n in rows {
                writeln!(out, "{}  kind={}  id={}", n.fqn, n.kind, n.id)?;
            }
        }
        GraphBody::Slice { rows } => {
            for r in rows {
                let dir = match r.direction {
                    crate::graph_queries::SliceDirection::Upstream => "up",
                    crate::graph_queries::SliceDirection::Downstream => "down",
                };
                let tier = r.edge_evidence_tier.as_deref().unwrap_or("tier_?");
                writeln!(
                    out,
                    "d{}={}  tier={}  {}  id={}",
                    r.depth, dir, tier, r.node.fqn, r.node.id
                )?;
            }
        }
        GraphBody::FileBlastRadius { result } => {
            writeln!(out, "file={}", result.file_path)?;
            for s in &result.seeds {
                writeln!(out, "seed={} id={}", s.fqn, s.id)?;
            }
            for r in &result.rows {
                let tier = r.edge_evidence_tier.as_deref().unwrap_or("tier_?");
                let via = r
                    .via_seeds
                    .iter()
                    .map(|s| s.fqn.as_str())
                    .collect::<Vec<_>>()
                    .join("|");
                writeln!(
                    out,
                    "d{} up tier={} via={} {} id={}",
                    r.depth, tier, via, r.node.fqn, r.node.id
                )?;
            }
        }
        GraphBody::ModuleCoupling { rows } => {
            for r in rows {
                writeln!(out, "edges={} {} -> {}", r.edge_count, r.from.fqn, r.to.fqn)?;
            }
        }
        GraphBody::Cycles { rows } => {
            for (i, c) in rows.iter().enumerate() {
                writeln!(out, "cycle#{} len={}", i + 1, c.length)?;
                for m in &c.members {
                    writeln!(out, "  -> {}", m.fqn)?;
                }
            }
        }
        GraphBody::Path { rows, from, to } => {
            writeln!(out, "from={} to={}", from.fqn, to.fqn)?;
            for n in rows {
                writeln!(out, "  - {}", n.fqn)?;
            }
        }
        GraphBody::Empty { reason } => {
            writeln!(out, "empty: {reason}")?;
        }
    }
    Ok(())
}
