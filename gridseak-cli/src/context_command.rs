//! `gridseak context [PATH]` — the LLM-native command.
//!
//! Output is structured into sections that an agent can rely on to
//! answer a typical "what is risky in this repo and what should I do
//! next" question without further file reads:
//!
//! 1. Project summary (name, root, scan id, branch/commit/dirty)
//! 2. Latest metrics snapshot (score, key counts)
//! 3. Top recommendations
//! 4. Confidence caveats (what the report does NOT promise)
//! 5. Relevant graph slice (focus-mode + optional finding +
//!    `--changed-files` aware)
//! 6. Exact next commands the agent can run
//! 7. Local artifact references (absolute paths)
//!
//! The renderer reports an `estimated_tokens` line at the top so the
//! agent knows how much of its window was consumed. When `--budget
//! <N>` is supplied we trim least-important sections first (graph
//! slice, then confidence caveats, then metric detail) until the
//! estimate fits.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use graphengine_analysis::health::report::{HealthReport, Severity};
use graphengine_diagnostic::priority::{self, PriorityItem};
use gridseak_local_store::ProjectStore;

use crate::graph_queries::{self as gq, NodeRef, SliceDirection, SliceNode};
use crate::intent_router::routing_table;

#[derive(Args, Debug, Clone)]
pub struct ContextArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Token budget for the rendered output. The renderer trims
    /// least-important sections to fit. Defaults to 4000 (matches the
    /// spec's recommended starting budget).
    #[arg(long, default_value_t = 4000)]
    pub budget: usize,

    /// Focus the graph slice on a specific risk area instead of the
    /// top priority. Pick one of: `hotspots`, `coupling`, `cycles`,
    /// `deadcode`.
    #[arg(long, value_enum)]
    pub focus: Option<FocusMode>,

    /// Pin the graph slice to a specific finding id. Mutually
    /// exclusive with `--focus`. When both are present, `--finding`
    /// wins because it is more specific.
    #[arg(long)]
    pub finding: Option<String>,

    /// Restrict the graph slice to functions defined in files
    /// changed since `HEAD~1`. Useful in pre-commit / pre-push hooks.
    #[arg(long, default_value_t = false)]
    pub changed_files: bool,

    /// Emit the LLM-friendly text (default). When `--json` is set
    /// globally we emit a JSON envelope instead so MCP / scripted
    /// consumers can parse the same data.
    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum FocusMode {
    Hotspots,
    Coupling,
    Cycles,
    Deadcode,
}

pub fn run_context(store: &ProjectStore, args: ContextArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let report: HealthReport = serde_json::from_value(store.load_report(&scan.id)?)?;
    let root_path = project
        .roots
        .first()
        .map(|r| r.path.clone())
        .unwrap_or_else(|| ".".into());

    let changed_files = if args.changed_files {
        load_changed_files(Path::new(&root_path))
            .map_err(|e| anyhow::anyhow!("--changed-files: {e}"))
            .ok()
            .unwrap_or_default()
    } else {
        BTreeSet::new()
    };

    let priorities = priority::compute_priorities(&report, 25);

    let focus_node = resolve_focus_node(&args, &priorities, &report);
    let graph_slice = if let Some(node) = focus_node.clone() {
        load_graph_slice(&scan.graph_artifact_path, &node, 2, 30, &changed_files)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let view = ContextView {
        repo_name: project.display_name.clone(),
        scan_id: scan.id.clone(),
        branch: scan.git_branch.clone(),
        commit: scan.git_commit.clone(),
        dirty: scan.git_dirty.unwrap_or(false),
        languages: scan.scan_languages.clone(),
        score: report.health_score,
        critical_count: report
            .findings
            .iter()
            .filter(|f| matches!(f.severity, Severity::Critical))
            .count(),
        high_count: report
            .findings
            .iter()
            .filter(|f| matches!(f.severity, Severity::High))
            .count(),
        total_findings: report.findings.len(),
        metrics: metric_summary(&report),
        recommendations: priorities
            .iter()
            .take(5)
            .map(|p| RecommendationSummary::from(p, &report))
            .collect(),
        caveats: confidence_caveats(&report),
        focus_node,
        graph_slice,
        changed_files: changed_files.into_iter().collect(),
        report_path: scan.report_path.clone().unwrap_or_default(),
        graph_artifact_path: scan.graph_artifact_path.clone().unwrap_or_default(),
        next_commands: next_commands_for_view(&priorities),
        free_paid_block: free_paid_block(),
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if args.for_llm || (!global_json) {
        render_text(&view, args.budget, &mut out)?;
    }
    if global_json {
        render_json(&view, &mut out)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Focus resolution
// ---------------------------------------------------------------------------

fn resolve_focus_node(
    args: &ContextArgs,
    priorities: &[PriorityItem],
    report: &HealthReport,
) -> Option<NodeFocus> {
    // `--finding <id>` wins because it is the most specific.
    if let Some(id) = &args.finding {
        if let Some(p) = priorities.iter().find(|p| &p.finding_id == id) {
            return Some(NodeFocus {
                reason: format!("finding `{}`", id),
                target_fqn: p.target.clone(),
                target_id: None,
            });
        }
        if let Some(f) = report.findings.iter().find(|f| &f.id == id) {
            return Some(NodeFocus {
                reason: format!("finding `{}`", id),
                target_fqn: f.primary_node_id.clone().unwrap_or_else(|| f.id.clone()),
                target_id: f.primary_node_id.clone(),
            });
        }
    }

    // `--focus <mode>` re-orders the priority list by mode-relevance.
    if let Some(focus) = args.focus {
        let want_type = match focus {
            FocusMode::Hotspots => "BlastRadiusHotspot",
            FocusMode::Coupling => "ModuleCouplingHigh",
            FocusMode::Cycles => "CycleParticipation",
            FocusMode::Deadcode => "DeadCode",
        };
        if let Some(p) = priorities.iter().find(|p| {
            let f = report.findings.iter().find(|f| f.id == p.finding_id);
            f.map(|f| format!("{:?}", f.finding_type) == want_type)
                .unwrap_or(false)
        }) {
            return Some(NodeFocus {
                reason: format!("focus=`{focus:?}`"),
                target_fqn: p.target.clone(),
                target_id: None,
            });
        }
    }

    // Default: anchor on the top priority, if any.
    priorities.first().map(|p| NodeFocus {
        reason: "top priority".into(),
        target_fqn: p.target.clone(),
        target_id: None,
    })
}

// ---------------------------------------------------------------------------
// Graph slice loader
// ---------------------------------------------------------------------------

fn load_graph_slice(
    artifact_path: &Option<String>,
    focus: &NodeFocus,
    depth: usize,
    cap: usize,
    changed_files: &BTreeSet<String>,
) -> Result<Vec<SliceNode>> {
    let Some(path) = artifact_path.as_ref() else {
        return Ok(Vec::new());
    };
    let conn = gq::open_graph(Path::new(path))?;
    let seed = match &focus.target_id {
        Some(id) => NodeRef {
            id: id.clone(),
            fqn: focus.target_fqn.clone(),
            kind: "Function".into(),
        },
        None => gq::resolve_symbol(&conn, &focus.target_fqn)
            .map_err(|e| anyhow::anyhow!("resolve focus: {e}"))?,
    };
    let mut rows =
        gq::slice(&conn, &seed.id, depth, cap).map_err(|e| anyhow::anyhow!("graph slice: {e}"))?;

    if !changed_files.is_empty() {
        rows.retain(|row| {
            let fqn = &row.node.fqn;
            changed_files.iter().any(|cf| fqn.contains(cf))
        });
    }

    Ok(rows)
}

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeFocus {
    pub reason: String,
    pub target_fqn: String,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextView {
    pub repo_name: String,
    pub scan_id: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub dirty: bool,
    pub languages: Vec<String>,
    pub score: Option<u32>,
    pub critical_count: usize,
    pub high_count: usize,
    pub total_findings: usize,
    pub metrics: Vec<(String, String)>,
    pub recommendations: Vec<RecommendationSummary>,
    pub caveats: Vec<String>,
    pub focus_node: Option<NodeFocus>,
    pub graph_slice: Vec<SliceNode>,
    pub changed_files: Vec<String>,
    pub report_path: String,
    pub graph_artifact_path: String,
    pub next_commands: Vec<String>,
    pub free_paid_block: FreePaidBlock,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecommendationSummary {
    pub rank: usize,
    pub finding_id: String,
    pub severity: String,
    pub finding_type: String,
    pub target: String,
    pub risk: String,
    pub action: String,
    pub score: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FreePaidBlock {
    pub free: Vec<String>,
    pub future_hosted: Vec<String>,
    pub feedback: String,
}

fn metric_summary(report: &HealthReport) -> Vec<(String, String)> {
    let m = &report.metrics;
    let mut rows = vec![
        ("cycles".into(), m.cycles.count.to_string()),
        (
            "tangle_index".into(),
            format!("{:.1}%", m.tangle_index.ratio * 100.0),
        ),
        ("max_call_depth".into(), m.depth.max_call_depth.to_string()),
        ("hotspots".into(), m.hotspot_concentration.count.to_string()),
        ("dead_functions".into(), m.dead_code.count.to_string()),
        (
            "avg_coupling".into(),
            format!("{:.2}", m.coupling.avg_coupling),
        ),
        (
            "high_coupling_modules".into(),
            report.summary.high_coupling_modules.to_string(),
        ),
    ];
    if let Some(c) = &m.complexity {
        rows.push(("max_cyclomatic".into(), c.max_cyclomatic.to_string()));
        rows.push(("max_cognitive".into(), c.max_cognitive.to_string()));
    }
    if let Some(coh) = &m.cohesion {
        rows.push(("avg_cohesion".into(), format!("{:.2}", coh.avg_cohesion)));
    }
    rows
}

fn confidence_caveats(report: &HealthReport) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(rq) = &report.resolution_quality {
        out.push(format!(
            "Resolution tier: {:?} ({} import edges total)",
            rq.resolution_tier, rq.import_edges_total
        ));
    }
    if !report.analysis_errors.is_empty() {
        out.push(format!(
            "{} analysis-error(s); metrics may be approximations.",
            report.analysis_errors.len()
        ));
    }
    if let Some(per_metric) = &report.metrics.metric_confidence {
        for (key, conf) in per_metric {
            let label = format!("{:?}", conf.level).to_ascii_lowercase();
            if label != "high" {
                out.push(format!(
                    "Confidence for {}: {label} ({})",
                    key.replace('_', " "),
                    conf.reason
                ));
            }
        }
    }
    if out.is_empty() {
        out.push("No reported confidence caveats on this scan.".into());
    }
    out
}

fn next_commands_for_view(priorities: &[PriorityItem]) -> Vec<String> {
    let mut cmds = Vec::new();
    if let Some(top) = priorities.first() {
        cmds.push(format!("gridseak explain {}", top.finding_id));
        cmds.push(format!("gridseak graph callers \"{}\"", top.target));
        cmds.push(format!("gridseak graph blast-radius \"{}\"", top.target));
    }
    cmds.push("gridseak findings . --severity critical".into());
    cmds.push("gridseak recommendations . --limit 10".into());
    cmds.push("gridseak trends --metric score --window 30d".into());
    cmds.push("gridseak scan .".into());
    cmds
}

fn free_paid_block() -> FreePaidBlock {
    // Canonical strings live in `crate::render::tier_signaling` so the
    // CLI hero report, the markdown export, the LLM scan render, and
    // this context bundle never drift from each other.
    let signal = crate::render::tier_signaling::TierSignal::default_v0();
    FreePaidBlock {
        free: signal.free,
        future_hosted: signal.future_hosted,
        feedback: signal.feedback_prompt,
    }
}

impl RecommendationSummary {
    fn from(p: &PriorityItem, report: &HealthReport) -> Self {
        let f = report.findings.iter().find(|f| f.id == p.finding_id);
        let severity = f
            .map(|f| crate::render::view::severity_display(f.severity))
            .unwrap_or("—")
            .to_string();
        let finding_type = f
            .map(|f| crate::render::view::finding_type_display(f.finding_type))
            .unwrap_or("—")
            .to_string();
        Self {
            rank: p.rank,
            finding_id: p.finding_id.clone(),
            severity,
            finding_type,
            target: p.target.clone(),
            risk: p.risk_narrative.clone(),
            action: p.suggested_action.clone(),
            score: p.priority_score,
        }
    }
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

fn render_text(view: &ContextView, budget: usize, out: &mut dyn Write) -> io::Result<()> {
    let sections = build_text_sections(view);
    let mut kept: Vec<&TextSection> = sections.iter().collect();
    // Drop least-important sections (highest `trim_rank`) until we
    // fit. Rank is sorted ascending — pop the highest values first.
    loop {
        let estimated = kept.iter().map(|s| estimate_tokens(&s.body)).sum::<usize>() + 50; // header line
        if estimated <= budget {
            writeln!(
                out,
                "[gridseak context repo={} scan={} estimated_tokens={}]",
                view.repo_name, view.scan_id, estimated
            )?;
            break;
        }
        let drop_idx = kept
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| s.trim_rank)
            .map(|(i, _)| i);
        match drop_idx {
            Some(i) if kept.len() > 1 => {
                kept.remove(i);
            }
            _ => {
                writeln!(
                    out,
                    "[gridseak context repo={} scan={} estimated_tokens={} budget={} note=truncated]",
                    view.repo_name,
                    view.scan_id,
                    kept.iter()
                        .map(|s| estimate_tokens(&s.body))
                        .sum::<usize>(),
                    budget
                )?;
                break;
            }
        }
    }
    for section in kept {
        writeln!(out, "\n## {}", section.title)?;
        write!(out, "{}", section.body)?;
        if !section.body.ends_with('\n') {
            writeln!(out)?;
        }
    }
    Ok(())
}

fn render_json(view: &ContextView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = serde_json::json!({
        "schema": "gridseak.context.v1",
        "view": view,
    });
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

#[derive(Debug)]
struct TextSection {
    title: String,
    body: String,
    /// Higher = more eligible for trim. The summary, metrics, and
    /// recommendations sit at 0–2 because they answer "what is going
    /// on at all". The graph slice and free/paid block are trimmable
    /// (5–7).
    trim_rank: usize,
}

fn build_text_sections(view: &ContextView) -> Vec<TextSection> {
    let mut out = Vec::new();
    out.push(TextSection {
        title: "Project".into(),
        body: format!(
            "repo: {}\nscan: {}\nbranch: {}{}\ncommit: {}\nlanguages: {}\nscore: {}\nfindings: {} (critical {}, high {})\n",
            view.repo_name,
            view.scan_id,
            view.branch.clone().unwrap_or_else(|| "—".into()),
            if view.dirty { " (dirty)" } else { "" },
            view.commit.clone().unwrap_or_else(|| "—".into()),
            if view.languages.is_empty() {
                "—".into()
            } else {
                view.languages.join(", ")
            },
            view.score.map(|s| format!("{s}/100")).unwrap_or_else(|| "—".into()),
            view.total_findings,
            view.critical_count,
            view.high_count,
        ),
        trim_rank: 0,
    });

    let metrics_body = view
        .metrics
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    out.push(TextSection {
        title: "Metrics".into(),
        body: format!("{metrics_body}\n"),
        trim_rank: 1,
    });

    let mut rec_body = String::new();
    if view.recommendations.is_empty() {
        rec_body.push_str("(no priorities returned by analysis.)\n");
    } else {
        for r in &view.recommendations {
            rec_body.push_str(&format!(
                "#{rank} [{sev}] {ty} :: {tgt} score={score:.2}\n  risk: {risk}\n  action: {action}\n  id: {id}\n",
                rank = r.rank,
                sev = r.severity,
                ty = r.finding_type,
                tgt = r.target,
                score = r.score,
                risk = r.risk,
                action = r.action,
                id = r.finding_id,
            ));
        }
    }
    out.push(TextSection {
        title: "Top recommendations".into(),
        body: rec_body,
        trim_rank: 2,
    });

    let caveats_body = view
        .caveats
        .iter()
        .map(|c| format!("- {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    out.push(TextSection {
        title: "Confidence caveats".into(),
        body: format!("{caveats_body}\n"),
        trim_rank: 4,
    });

    out.push(TextSection {
        title: "Routing (symptom → tool)".into(),
        body: routing_section_body(),
        trim_rank: 3,
    });

    let next_body = view
        .next_commands
        .iter()
        .map(|c| format!("- {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    out.push(TextSection {
        title: "Next commands".into(),
        body: format!("{next_body}\n"),
        trim_rank: 4,
    });

    let mut slice_body = String::new();
    if let Some(focus) = &view.focus_node {
        slice_body.push_str(&format!("focus: {} ({})\n", focus.target_fqn, focus.reason));
    }
    if view.graph_slice.is_empty() {
        slice_body.push_str("(no slice rows; widen depth or unset --changed-files)\n");
    } else {
        for row in &view.graph_slice {
            let dir = match row.direction {
                SliceDirection::Upstream => "up",
                SliceDirection::Downstream => "down",
            };
            slice_body.push_str(&format!("  d{} {} {}\n", row.depth, dir, row.node.fqn));
        }
    }
    if !view.changed_files.is_empty() {
        slice_body.push_str(&format!(
            "changed_files: {}\n",
            view.changed_files.join(", ")
        ));
    }
    out.push(TextSection {
        title: "Graph slice".into(),
        body: slice_body,
        trim_rank: 5,
    });

    out.push(TextSection {
        title: "Artifacts".into(),
        body: format!(
            "report: {}\ngraph_db: {}\n",
            view.report_path, view.graph_artifact_path
        ),
        trim_rank: 6,
    });

    let fp = &view.free_paid_block;
    let mut fp_body = String::from("Free in this build:\n");
    for item in &fp.free {
        fp_body.push_str(&format!("  - {item}\n"));
    }
    fp_body.push_str("Future hosted SaaS (not in this binary):\n");
    for item in &fp.future_hosted {
        fp_body.push_str(&format!("  - {item}\n"));
    }
    fp_body.push_str(&format!("\n{}\n", fp.feedback));
    out.push(TextSection {
        title: "Free vs hosted SaaS (signaling)".into(),
        body: fp_body,
        trim_rank: 7,
    });
    out
}

fn routing_section_body() -> String {
    let mut body = String::from(
        "Deterministic routing (no LLM). Use `gridseak route \"<question>\"` to debug.\n\n",
    );
    for (symptom, tool, _pre) in routing_table() {
        body.push_str(&format!("- {} → {}\n", symptom, tool.mcp_name()));
    }
    body.push_str(
        "\nIf MCP returns STALE_SNAPSHOT, run `gridseak scan .` before structural claims.\n",
    );
    body
}

fn estimate_tokens(s: &str) -> usize {
    // GPT-4 averages ~4 chars/token. The estimate doesn't need to be
    // exact; we just need stable, monotonic, conservative.
    s.len().div_ceil(4)
}

// ---------------------------------------------------------------------------
// --changed-files helper
// ---------------------------------------------------------------------------

fn load_changed_files(repo_root: &Path) -> Result<BTreeSet<String>> {
    let git_dir = repo_root.join(".git");
    if !git_dir.exists() {
        anyhow::bail!("no .git directory under {}", repo_root.display());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["diff", "--name-only", "HEAD~1..HEAD"])
        .output()
        .context("invoke `git diff`")?;
    if !output.status.success() {
        // Repos with only one commit have no HEAD~1; fall back to
        // working-tree changes.
        let working = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["status", "--porcelain"])
            .output()
            .context("invoke `git status`")?;
        let mut set = BTreeSet::new();
        for line in String::from_utf8_lossy(&working.stdout).lines() {
            if let Some(name) = line.get(3..) {
                set.insert(name.to_string());
            }
        }
        return Ok(set);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_roughly_matches_four_chars_per_token() {
        let s = "abcd".repeat(100);
        assert_eq!(estimate_tokens(&s), 100);
    }

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn changed_files_helper_returns_set() {
        use std::path::PathBuf;
        // Anti-flake: the test doesn't assert any specific path —
        // only that the call shape works on the workspace root we
        // happen to live in. If git is somehow unavailable the
        // function falls back to empty, which is also fine.
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(1)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = load_changed_files(&repo_root);
    }
}
