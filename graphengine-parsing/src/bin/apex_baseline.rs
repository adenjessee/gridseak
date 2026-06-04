//! End-to-end Apex repository scanner for baseline recording.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --bin apex_baseline -- \
//!     --repo   /path/to/sfdx-repo          \
//!     --label  dreamhouse-lwc              \
//!     --output tests/fixtures/apex_baseline/dreamhouse-lwc.json
//! ```
//!
//! What it does:
//!
//! 1. Loads the Apex `LanguageConfig` and builds the real end-to-end
//!    pipeline via `ParseRepositoryUseCase::with_real_components` — the
//!    exact same factory path the desktop binary would invoke.
//! 2. Points the pipeline at the given SFDX-shaped repository root.
//! 3. Runs parse → resolve → build graph.
//! 4. Emits a deterministic JSON baseline capturing:
//!    - Total nodes / edges.
//!    - Node counts per `NodeKind`.
//!    - Edge counts per `EdgeKind`.
//!    - Provenance breakdown (LSP vs Heuristic vs TreeSitter vs Combined).
//!    - Confidence breakdown (High/Medium/Low).
//!    - Apex-specific aggregates: `apex_sharing` distribution,
//!      `entry_points` tag histogram, trigger count per SObject,
//!      managed-package consumer count per namespace.
//!    - `ResolutionStatsSummary` (LSP vs heuristic fallback counts).
//!
//! The baseline is committed under `tests/fixtures/apex_baseline/` so any accuracy
//! regression shows up in `git diff` with zero guesswork.
//!
//! This binary is intentionally a thin wrapper: all measurement logic
//! lives in the library so it is unit-testable on the corpus fixture.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::{Confidence, EdgeKind, NodeKind, ProvenanceSource};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::lsp::resolve_lsp_command;
use graphengine_parsing::syntax::language::apex::ENV_APEX_RESOLVER;
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "apex_baseline",
    about = "Scan an SFDX repository and emit a deterministic Apex baseline JSON"
)]
struct Cli {
    /// Root of the SFDX repo (containing `force-app/` or `src/`).
    #[arg(long)]
    repo: PathBuf,

    /// Human-readable label for the baseline (e.g. `dreamhouse-lwc`).
    #[arg(long)]
    label: String,

    /// Output JSON path. Parent directory must exist.
    #[arg(long)]
    output: PathBuf,

    /// Sprint H.4 — require LSP resolution.
    ///
    /// When set, this flag forces the Apex resolver dispatcher to
    /// `lsp` mode (via `GRAPHENGINE_APEX_RESOLVER=lsp`) AND refuses to
    /// start the scan if the apex-jorje jar or a usable Java runtime
    /// is missing. This is how the nightly CI workflow
    /// (`.github/workflows/apex-lsp.yml`) guarantees that an LSP
    /// recall threshold check cannot silently pass on a heuristic-only
    /// fallback.
    ///
    /// Without this flag the dispatcher keeps its normal "auto" mode:
    /// try LSP first, fall back to heuristic if LSP fails to come up.
    #[arg(long = "enable-lsp", default_value_t = false)]
    enable_lsp: bool,
}

#[derive(Serialize)]
struct Baseline {
    label: String,
    repo_path: String,
    elapsed_ms: u128,
    node_count: usize,
    edge_count: usize,
    nodes_by_kind: BTreeMap<String, usize>,
    edges_by_kind: BTreeMap<String, usize>,
    edges_by_provenance: BTreeMap<String, usize>,
    edges_by_confidence: BTreeMap<String, usize>,
    apex_sharing_distribution: BTreeMap<String, usize>,
    entry_point_histogram: BTreeMap<String, usize>,
    triggers_per_sobject: BTreeMap<String, usize>,
    managed_package_consumers: BTreeMap<String, usize>,
    resolution_stats: ResolutionStatsView,
    heuristic_fallback_rate: f64,
}

#[derive(Serialize)]
struct ResolutionStatsView {
    lsp_edges: usize,
    heuristic_edges: usize,
    lsp_failures: usize,
    heuristic_call_fallbacks: usize,
    heuristic_import_fallbacks: usize,
    heuristic_type_fallbacks: usize,
    /// Call sites the heuristic resolver refused to resolve because
    /// the candidate fanout exceeded its cap. A non-zero value is a
    /// direct measure of how much call-graph signal would be recovered
    /// by switching to LSP. Useful for the Sprint D ResolutionDegraded
    /// finding.
    heuristic_call_ambiguous_drops: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if !cli.repo.exists() {
        anyhow::bail!("repo path does not exist: {}", cli.repo.display());
    }
    if let Some(parent) = cli.output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            anyhow::bail!("output directory does not exist: {}", parent.display());
        }
    }

    // Sprint H.4 — when --enable-lsp is set, force LSP-only resolution
    // and preflight-check that the apex-jorje stack (java + jar) is
    // actually usable. Without this the nightly CI recall-check can
    // pass on a silent heuristic fallback, defeating the whole point.
    if cli.enable_lsp {
        // SAFETY: single-threaded binary startup, before any resolver
        // dispatcher reads the env var.
        unsafe {
            std::env::set_var(ENV_APEX_RESOLVER, "lsp");
        }

        let config = load_config("apex")
            .map_err(|e| anyhow::anyhow!("--enable-lsp requires a valid apex config: {e}"))?;
        match resolve_lsp_command(&config) {
            Ok(cmd) => {
                eprintln!(
                    "[apex_baseline] --enable-lsp preflight OK: java+jar resolved -> {:?}",
                    cmd.command
                );
            }
            Err(e) => {
                anyhow::bail!(
                    "--enable-lsp specified but apex-jorje LSP stack is not usable: {e}\n\
                     Set GRAPHENGINE_APEX_JORJE_JAR and/or GRAPHENGINE_JAVA_HOME, or \
                     install Java 17 and run scripts/download_apex_jorje.sh."
                );
            }
        }
    }

    // Use a scratch SQLite file — we don't need to persist the graph.
    // Stamped with PID + timestamp so concurrent runs don't clobber
    // each other.
    let scratch = std::env::temp_dir().join(format!(
        "apex_baseline_{}_{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let db_path = scratch;
    let ws_url = url::Url::from_directory_path(&cli.repo)
        .map_err(|_| anyhow::anyhow!("repo path is not absolute: {}", cli.repo.display()))?;

    let use_case = ParseRepositoryUseCase::with_real_components(
        "apex".to_string(),
        Confidence::Low,
        db_path.to_str().unwrap(),
        Some(ws_url),
    )
    .await?;

    let started = Instant::now();
    let resolved = use_case.parse(cli.repo.clone(), "apex".to_string()).await?;
    let elapsed_ms = started.elapsed().as_millis();

    let graph = resolved.graph();
    let stats = resolved.stats();

    let mut nodes_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges_by_provenance: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges_by_confidence: BTreeMap<String, usize> = BTreeMap::new();
    let mut apex_sharing: BTreeMap<String, usize> = BTreeMap::new();
    let mut entry_points_hist: BTreeMap<String, usize> = BTreeMap::new();
    let mut triggers_per_sobject: BTreeMap<String, usize> = BTreeMap::new();
    let mut managed_package_consumers: BTreeMap<String, usize> = BTreeMap::new();

    for n in &graph.nodes {
        *nodes_by_kind.entry(node_kind_label(&n.kind)).or_default() += 1;

        if let Some(sharing) = n.properties.get("apex_sharing").and_then(|v| v.as_str()) {
            *apex_sharing.entry(sharing.to_string()).or_default() += 1;
        }

        if let Some(tags) = n.properties.get("entry_points").and_then(|v| v.as_array()) {
            for tag in tags {
                if let Some(s) = tag.as_str() {
                    *entry_points_hist.entry(s.to_string()).or_default() += 1;
                }
            }
        }

        // Trigger struct → SObject binding.
        let is_trigger_struct = matches!(n.kind, NodeKind::Struct)
            && n.properties
                .get("subtype")
                .and_then(|v| v.as_str())
                .map(|s| s == "trigger")
                .unwrap_or(false);
        if is_trigger_struct {
            if let Some(sobj) = n.properties.get("sobject").and_then(|v| v.as_str()) {
                *triggers_per_sobject.entry(sobj.to_string()).or_default() += 1;
            }
        }

        // Managed-package external Module virtual nodes carry a stable
        // FQN prefix per the managed_packages synthesizer.
        if matches!(n.kind, NodeKind::Module)
            && n.fqn.starts_with(
                graphengine_parsing::syntax::language::apex::VIRTUAL_MANAGED_MODULE_FQN_PREFIX,
            )
        {
            let ns = n
                .fqn
                .rsplit("::")
                .next()
                .unwrap_or(n.fqn.as_str())
                .to_string();
            // Count consumers by scanning incoming Import edges.
            let consumers = graph
                .edges
                .iter()
                .filter(|e| e.to_id == n.id && matches!(e.kind, EdgeKind::Import))
                .count();
            managed_package_consumers.insert(ns, consumers);
        }
    }

    for e in &graph.edges {
        *edges_by_kind.entry(edge_kind_label(&e.kind)).or_default() += 1;
        *edges_by_provenance
            .entry(provenance_label(&e.provenance.source))
            .or_default() += 1;
        *edges_by_confidence
            .entry(confidence_label(&e.provenance.confidence))
            .or_default() += 1;
    }

    let total_edges = graph.edges.len() as f64;
    let heuristic_fallback_rate = if total_edges > 0.0 {
        let heuristic = *edges_by_provenance.get("heuristic").unwrap_or(&0) as f64;
        heuristic / total_edges
    } else {
        0.0
    };

    let baseline = Baseline {
        label: cli.label.clone(),
        repo_path: cli.repo.display().to_string(),
        elapsed_ms,
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
        nodes_by_kind,
        edges_by_kind,
        edges_by_provenance,
        edges_by_confidence,
        apex_sharing_distribution: apex_sharing,
        entry_point_histogram: entry_points_hist,
        triggers_per_sobject,
        managed_package_consumers,
        resolution_stats: ResolutionStatsView {
            lsp_edges: stats.lsp_edges,
            heuristic_edges: stats.heuristic_edges,
            lsp_failures: stats.lsp_failures.len(),
            heuristic_call_fallbacks: stats.heuristic_call_fallbacks,
            heuristic_import_fallbacks: stats.heuristic_import_fallbacks,
            heuristic_type_fallbacks: stats.heuristic_type_fallbacks,
            heuristic_call_ambiguous_drops: stats.heuristic_call_ambiguous_drops,
        },
        heuristic_fallback_rate,
    };

    let json = serde_json::to_string_pretty(&baseline)?;
    std::fs::write(&cli.output, json)?;

    eprintln!(
        "[apex_baseline] {} -> {} ({} nodes, {} edges, {:.2}% heuristic, {}ms)",
        cli.label,
        cli.output.display(),
        baseline.node_count,
        baseline.edge_count,
        baseline.heuristic_fallback_rate * 100.0,
        baseline.elapsed_ms
    );

    // Sprint H.4 — post-scan assertion. If --enable-lsp was set but the
    // scan produced zero LSP-sourced edges, the LSP stack started but
    // immediately died, or every request fell through to the heuristic.
    // Either way, this is NOT a valid LSP measurement and the nightly
    // recall-threshold check would silently pass on heuristic data.
    // Fail hard so the CI job records red.
    if cli.enable_lsp && baseline.resolution_stats.lsp_edges == 0 {
        anyhow::bail!(
            "--enable-lsp was set but scan produced 0 LSP-sourced edges \
             (heuristic_edges={}). The apex-jorje stack may have crashed \
             after initialization, or every request timed out. Inspect the \
             resolver logs above for failures.",
            baseline.resolution_stats.heuristic_edges,
        );
    }

    Ok(())
}

fn node_kind_label(k: &NodeKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}

fn edge_kind_label(k: &EdgeKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}

fn provenance_label(p: &ProvenanceSource) -> String {
    match p {
        ProvenanceSource::Lsp => "lsp",
        ProvenanceSource::Heuristic => "heuristic",
        ProvenanceSource::TreeSitter => "treesitter",
    }
    .to_string()
}

fn confidence_label(c: &Confidence) -> String {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
    .to_string()
}
