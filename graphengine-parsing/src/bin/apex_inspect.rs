//! Focused single-class Apex graph inspection.
//!
//! Purpose: support Sprint G.2 manual predictions by dumping the
//! full node + edge picture for a specific class/trigger FQN
//! substring. The operator picks a non-trivial class, predicts its
//! outgoing call / type / extends / implements / import edges from
//! reading the source, and then compares that prediction to what
//! the pipeline actually produced.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --bin apex_inspect -- \
//!     --repo   /path/to/sfdx-repo \
//!     --match  ContactTriggerHandler \
//!     --output /tmp/ContactTriggerHandler.json
//! ```
//!
//! Match semantics: any node whose `fqn` contains `--match` as a
//! case-sensitive substring is reported. Together with every edge
//! where that node is either endpoint. Deliberately a thin wrapper
//! so the inspection stays reproducible and unit-testable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use clap::Parser;
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::{Confidence, EdgeKind, NodeKind};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "apex_inspect",
    about = "Dump nodes + edges touching a specific Apex class FQN substring"
)]
struct Cli {
    /// SFDX repo root.
    #[arg(long)]
    repo: PathBuf,

    /// Substring match against `node.fqn`. Case-sensitive. May be passed
    /// more than once to inspect several classes in a single scan, which
    /// is the expected mode for large corpora (NPSP etc.) where re-parsing
    /// per class is wasteful.
    #[arg(long = "match")]
    fqn_match: Vec<String>,

    /// Output directory. Parent directory must exist. One JSON file per
    /// `--match` value is written, named `<sanitised match>.json`.
    #[arg(long = "output-dir")]
    output_dir: PathBuf,
}

#[derive(Serialize)]
struct NodeView {
    id: String,
    kind: String,
    fqn: String,
    file: String,
    start_line: u32,
    end_line: u32,
    properties: serde_json::Value,
}

#[derive(Serialize)]
struct EdgeView {
    kind: String,
    from_fqn: String,
    from_kind: String,
    to_fqn: String,
    to_kind: String,
    provenance: String,
    confidence: String,
}

#[derive(Serialize)]
struct InspectOutput {
    repo_path: String,
    fqn_match: String,
    matched_nodes: Vec<NodeView>,
    outgoing: Vec<EdgeView>,
    incoming: Vec<EdgeView>,
    edge_totals_by_kind_outgoing: BTreeMap<String, usize>,
    edge_totals_by_kind_incoming: BTreeMap<String, usize>,
}

fn sanitise(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if !cli.repo.exists() {
        anyhow::bail!("repo path does not exist: {}", cli.repo.display());
    }
    if cli.fqn_match.is_empty() {
        anyhow::bail!("at least one --match value is required");
    }
    if !cli.output_dir.exists() {
        anyhow::bail!(
            "output directory does not exist: {}",
            cli.output_dir.display()
        );
    }

    let scratch = std::env::temp_dir().join(format!(
        "apex_inspect_{}_{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let ws_url = url::Url::from_directory_path(&cli.repo)
        .map_err(|_| anyhow::anyhow!("repo path is not absolute: {}", cli.repo.display()))?;

    let use_case = ParseRepositoryUseCase::with_real_components(
        "apex".to_string(),
        Confidence::Low,
        scratch.to_str().unwrap(),
        Some(ws_url),
    )
    .await?;
    let resolved = use_case.parse(cli.repo.clone(), "apex".to_string()).await?;
    let graph = resolved.graph();

    let by_id: BTreeMap<&str, &graphengine_parsing::domain::Node> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    for fqn_match in &cli.fqn_match {
        let matched: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| n.fqn.contains(fqn_match))
            .collect();
        let matched_ids: BTreeSet<&str> = matched.iter().map(|n| n.id.as_str()).collect();

        let mut outgoing = Vec::new();
        let mut incoming = Vec::new();
        let mut edge_totals_outgoing: BTreeMap<String, usize> = BTreeMap::new();
        let mut edge_totals_incoming: BTreeMap<String, usize> = BTreeMap::new();

        for e in &graph.edges {
            let from_in = matched_ids.contains(e.from_id.as_str());
            let to_in = matched_ids.contains(e.to_id.as_str());
            if !from_in && !to_in {
                continue;
            }
            let (Some(from), Some(to)) =
                (by_id.get(e.from_id.as_str()), by_id.get(e.to_id.as_str()))
            else {
                continue;
            };
            let view = EdgeView {
                kind: edge_kind_label(&e.kind),
                from_fqn: from.fqn.clone(),
                from_kind: node_kind_label(&from.kind),
                to_fqn: to.fqn.clone(),
                to_kind: node_kind_label(&to.kind),
                provenance: format!("{:?}", e.provenance.source).to_ascii_lowercase(),
                confidence: format!("{:?}", e.provenance.confidence).to_ascii_lowercase(),
            };
            if from_in {
                *edge_totals_outgoing
                    .entry(edge_kind_label(&e.kind))
                    .or_default() += 1;
                outgoing.push(view);
            } else {
                *edge_totals_incoming
                    .entry(edge_kind_label(&e.kind))
                    .or_default() += 1;
                incoming.push(view);
            }
        }

        let matched_nodes: Vec<NodeView> = matched
            .iter()
            .map(|n| NodeView {
                id: n.id.clone(),
                kind: node_kind_label(&n.kind),
                fqn: n.fqn.clone(),
                file: n.location.file.clone(),
                start_line: n.location.start_line,
                end_line: n.location.end_line,
                properties: serde_json::to_value(&n.properties).unwrap_or(serde_json::Value::Null),
            })
            .collect();

        let output = InspectOutput {
            repo_path: cli.repo.display().to_string(),
            fqn_match: fqn_match.clone(),
            matched_nodes,
            outgoing,
            incoming,
            edge_totals_by_kind_outgoing: edge_totals_outgoing,
            edge_totals_by_kind_incoming: edge_totals_incoming,
        };

        let file_path = cli.output_dir.join(format!("{}.json", sanitise(fqn_match)));
        let json = serde_json::to_string_pretty(&output)?;
        std::fs::write(&file_path, json)?;
        eprintln!(
            "[apex_inspect] match='{}' -> {} ({} nodes, {} outgoing / {} incoming edges)",
            fqn_match,
            file_path.display(),
            output.matched_nodes.len(),
            output.outgoing.len(),
            output.incoming.len()
        );
    }

    let _ = std::fs::remove_file(&scratch);
    Ok(())
}

fn node_kind_label(k: &NodeKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}

fn edge_kind_label(k: &EdgeKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}
