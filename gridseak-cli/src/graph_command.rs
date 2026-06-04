//! Driver for `gridseak graph *` subcommands.
//!
//! Why this lives in its own driver module: every graph subcommand
//! has the same outer shape:
//!
//! 1. Resolve project / latest scan / graph artifact path.
//! 2. Open the SQLite read-only.
//! 3. Resolve any user-supplied symbol → node id (with helpful
//!    ambiguity errors).
//! 4. Run the actual query from `crate::graph_queries`.
//! 5. Wrap the result in a `GraphView` envelope so the renderer
//!    knows what title/labels to use.
//! 6. Hand the view to the unified `render::graph::render` dispatch.
//!
//! Keeping the resolve/open/render bookkeeping here means
//! `graph_queries` stays pure SQL and the renderer stays pure
//! formatting; either can be swapped in isolation (e.g. when MCP
//! re-uses the query module in Stage 8).

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use gridseak_local_store::ProjectStore;

use crate::graph_queries::{self as gq, GraphQueryError, NodeRef};
use crate::render::graph::{self as graph_render, GraphBody, GraphFormat, GraphView};
use crate::scan_command::ScanOutputFormat;

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_HOTSPOT_THRESHOLD: i64 = 10;
const DEFAULT_RADIUS_DEPTH: usize = 3;
const DEFAULT_PATH_DEPTH: usize = 10;
const DEFAULT_CYCLE_DEPTH: usize = 8;
const DEFAULT_SLICE_DEPTH: usize = 2;
const DEFAULT_TRAVERSAL_CAP: usize = 500;

#[derive(Args, Debug, Clone)]
pub struct GraphArgs {
    #[command(subcommand)]
    pub command: GraphSubcommand,

    #[arg(long, default_value = ".", global = true)]
    pub project: String,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table, global = true)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false, global = true)]
    pub for_llm: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum GraphSubcommand {
    /// Functions with the most inbound Call edges (callers).
    TopFanIn(LimitArgs),
    /// Functions with the most outbound Call edges (callees).
    TopFanOut(LimitArgs),
    /// High fan-in functions above a configurable threshold.
    Hotspots(HotspotArgs),
    /// Functions with zero inbound Call edges. Graph-only — see caveat.
    DeadCode(LimitArgs),
    /// Direct callers of `<symbol>`.
    Callers(SymbolArgs),
    /// Direct callees of `<symbol>`.
    Callees(SymbolArgs),
    /// All downstream Call-graph neighbours within `--depth N` hops.
    BlastRadius(RadiusArgs),
    /// Transitive upstream callers of every callable symbol in a file.
    FileBlastRadius(FileBlastRadiusArgs),
    /// Shortest Call-edge path between two symbols.
    Path(PathArgs),
    /// Top coupled module pairs by Call-edge count.
    ModuleCoupling(LimitArgs),
    /// Simple Call-graph cycles (depth-bounded).
    Cycles(CyclesArgs),
    /// Upstream + downstream slice around a symbol.
    Slice(SliceArgs),
}

#[derive(Args, Debug, Clone)]
pub struct LimitArgs {
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct HotspotArgs {
    #[arg(long, default_value_t = DEFAULT_HOTSPOT_THRESHOLD)]
    pub min_fan_in: i64,
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct SymbolArgs {
    pub symbol: String,
}

#[derive(Args, Debug, Clone)]
pub struct RadiusArgs {
    pub symbol: String,
    #[arg(long, default_value_t = DEFAULT_RADIUS_DEPTH)]
    pub depth: usize,
    #[arg(long, default_value_t = DEFAULT_TRAVERSAL_CAP)]
    pub cap: usize,
}

#[derive(Args, Debug, Clone)]
pub struct FileBlastRadiusArgs {
    pub file: String,
    #[arg(long, default_value_t = DEFAULT_RADIUS_DEPTH)]
    pub depth: usize,
    #[arg(long, default_value_t = 200)]
    pub cap: usize,
}

#[derive(Args, Debug, Clone)]
pub struct PathArgs {
    pub from: String,
    pub to: String,
    #[arg(long, default_value_t = DEFAULT_PATH_DEPTH)]
    pub depth: usize,
}

#[derive(Args, Debug, Clone)]
pub struct CyclesArgs {
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,
    #[arg(long, default_value_t = DEFAULT_CYCLE_DEPTH)]
    pub max_depth: usize,
}

#[derive(Args, Debug, Clone)]
pub struct SliceArgs {
    pub symbol: String,
    #[arg(long, default_value_t = DEFAULT_SLICE_DEPTH)]
    pub depth: usize,
    #[arg(long, default_value_t = DEFAULT_TRAVERSAL_CAP)]
    pub cap: usize,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum GraphFormatArg {
    Table,
    Markdown,
    Json,
    ForLlm,
}

pub fn run_graph(store: &ProjectStore, args: GraphArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let artifact_path = scan
        .graph_artifact_path
        .clone()
        .context("latest scan has no graph artifact recorded")?;
    let artifact = PathBuf::from(&artifact_path);
    let conn = gq::open_graph(&artifact)
        .with_context(|| format!("open graph artifact at {artifact_path}"))?;

    let format = resolve_format(args.format, args.for_llm, global_json);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let view = match &args.command {
        GraphSubcommand::TopFanIn(a) => {
            let rows = gq::top_fan_in(&conn, a.limit)?;
            GraphView {
                title: format!("Top fan-in (callers) — {}", project.display_name),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Fan {
                    rows,
                    count_label: "callers".into(),
                },
            }
        }
        GraphSubcommand::TopFanOut(a) => {
            let rows = gq::top_fan_out(&conn, a.limit)?;
            GraphView {
                title: format!("Top fan-out (callees) — {}", project.display_name),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Fan {
                    rows,
                    count_label: "callees".into(),
                },
            }
        }
        GraphSubcommand::Hotspots(a) => {
            let rows = gq::hotspots(&conn, a.min_fan_in, a.limit)?;
            GraphView {
                title: format!(
                    "Hotspots (≥{} callers) — {}",
                    a.min_fan_in, project.display_name
                ),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Fan {
                    rows,
                    count_label: "callers".into(),
                },
            }
        }
        GraphSubcommand::DeadCode(a) => {
            let rows = gq::dead_code(&conn, a.limit)?;
            GraphView {
                title: format!(
                    "Dead-code candidates (graph-only) — {}",
                    project.display_name
                ),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Fan {
                    rows,
                    count_label: "callers".into(),
                },
            }
        }
        GraphSubcommand::Callers(a) => {
            let target = resolve_or_die(&conn, &a.symbol)?;
            let rows = gq::callers(&conn, &target.id)?;
            GraphView {
                title: format!("Callers of {}", target.fqn),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Nodes { rows },
            }
        }
        GraphSubcommand::Callees(a) => {
            let target = resolve_or_die(&conn, &a.symbol)?;
            let rows = gq::callees(&conn, &target.id)?;
            GraphView {
                title: format!("Callees of {}", target.fqn),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Nodes { rows },
            }
        }
        GraphSubcommand::BlastRadius(a) => {
            // Reject path-shaped seeds with the same guidance the MCP
            // layer gives the agent; otherwise blast_radius silently
            // returns no rows and a human user thinks "huh, nothing
            // depends on it."
            if gq::seed_looks_like_path(&a.symbol) {
                return Err(anyhow::anyhow!(
                    "blast-radius expects a function/method symbol, not a file or path (got `{}`).\n\
                     Resolve a specific public symbol in that file first — try\n\
                     `gridseak graph callers \"<symbol>\"` or `gridseak recommendations`.",
                    a.symbol
                ));
            }
            let target = resolve_or_die(&conn, &a.symbol)?;
            let rows = gq::blast_radius(&conn, &target.id, a.depth, a.cap)?;
            GraphView {
                title: format!(
                    "Blast radius (≤{} hops, capped {}) — {}",
                    a.depth, a.cap, target.fqn
                ),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Slice { rows },
            }
        }
        GraphSubcommand::FileBlastRadius(a) => {
            let normalised = normalise_file_path_for_lookup(&project, &a.file);
            let result = gq::file_blast_radius(&conn, &normalised, a.depth, a.cap)?;
            if result.seeds.is_empty() {
                return Err(anyhow::anyhow!(
                    "no Function/Method nodes found in `{}` (normalised to `{}`). \
                     Use a repo-relative path from the project root, or re-run `gridseak scan` \
                     if the file is new since the last scan.",
                    a.file,
                    normalised
                ));
            }
            GraphView {
                title: format!(
                    "File blast radius (≤{} hops, cap {}) — {}",
                    a.depth, a.cap, normalised
                ),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::FileBlastRadius { result },
            }
        }
        GraphSubcommand::Path(a) => {
            let from = resolve_or_die(&conn, &a.from)?;
            let to = resolve_or_die(&conn, &a.to)?;
            let rows = gq::shortest_path(&conn, &from.id, &to.id, a.depth)?.unwrap_or_default();
            GraphView {
                title: format!("Call-path {} → {}", from.fqn, to.fqn),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Path { rows, from, to },
            }
        }
        GraphSubcommand::ModuleCoupling(a) => {
            let rows = gq::module_coupling(&conn, a.limit)?;
            GraphView {
                title: format!("Top module coupling — {}", project.display_name),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::ModuleCoupling { rows },
            }
        }
        GraphSubcommand::Cycles(a) => {
            let rows = gq::cycles(&conn, a.limit, a.max_depth)?;
            GraphView {
                title: format!(
                    "Call-graph cycles (≤{} depth) — {}",
                    a.max_depth, project.display_name
                ),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Cycles { rows },
            }
        }
        GraphSubcommand::Slice(a) => {
            let target = resolve_or_die(&conn, &a.symbol)?;
            let rows = gq::slice(&conn, &target.id, a.depth, a.cap)?;
            GraphView {
                title: format!("Slice ±{} hops — {}", a.depth, target.fqn),
                scan_id: scan.id.clone(),
                artifact_path: artifact_path.clone(),
                repo_name: project.display_name.clone(),
                body: GraphBody::Slice { rows },
            }
        }
    };

    graph_render::render(format, &view, &mut out)?;
    Ok(())
}

fn resolve_or_die(conn: &rusqlite::Connection, symbol: &str) -> Result<NodeRef> {
    gq::resolve_symbol(conn, symbol).map_err(|e| match e {
        GraphQueryError::AmbiguousSymbol {
            candidates, total, ..
        } => anyhow::anyhow!(
            "symbol `{symbol}` matched {total} candidates. Try a more specific FQN. \
             Top matches:\n{}",
            candidates
                .iter()
                .map(|c| format!("  - {c}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        other => anyhow::anyhow!(other),
    })
}

fn resolve_format(format: ScanOutputFormat, for_llm: bool, global_json: bool) -> GraphFormat {
    if for_llm {
        return GraphFormat::ForLlm;
    }
    if global_json {
        return GraphFormat::Json;
    }
    match format {
        ScanOutputFormat::Table => GraphFormat::Table,
        ScanOutputFormat::Markdown => GraphFormat::Markdown,
        ScanOutputFormat::Json => GraphFormat::Json,
    }
}

/// Strip a project-root prefix from absolute file paths (MCP parity).
fn normalise_file_path_for_lookup(
    project: &gridseak_local_store::ProjectDto,
    file: &str,
) -> String {
    use std::path::Path;
    let p = Path::new(file);
    if !p.is_absolute() {
        return file.to_string();
    }
    for root in &project.roots {
        let root_path = Path::new(&root.path);
        if let Ok(rel) = p.strip_prefix(root_path) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    file.to_string()
}
