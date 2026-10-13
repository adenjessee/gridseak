//! `gridseak audit`: grade a coding agent on this repository against compiler truth.
//!
//! Plan: pick target functions from the scan's call graph, small to large, and
//! take each one's transitive static callers as the answer key. The graph must
//! be fully compiler-indexed or the audit refuses to run, because the key would
//! be a guess.
//! Run: give each task to a fresh headless agent session (Claude Code today),
//! once without GridSeak ("plain") and optionally once with its MCP tools.
//! Grade: set recall and precision against the key; a miss is "silent" when
//! the reply never says the list is incomplete.
//! Report: `report.html` (a shareable card), `report.md` and `results.json`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Args;
use gridseak_local_store::ProjectStore;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod scope;
use scope::{ts_rules, Coverage};

#[derive(Args, Debug, Clone)]
pub struct AuditArgs {
    /// Project to audit (path or name). Defaults to the current directory.
    #[arg(long, default_value = ".")]
    pub project: String,
    /// Number of tasks, spread across small, medium and large caller sets.
    #[arg(long, default_value_t = 6)]
    pub tasks: usize,
    /// Agent to audit. Only `claude` (Claude Code, headless) is supported today.
    #[arg(long, default_value = "claude")]
    pub agent: String,
    /// Model passed to the agent, e.g. `claude-opus-5-5`. Agent default if unset.
    #[arg(long)]
    pub model: Option<String>,
    /// Comma-separated arms: `plain` (agent alone) and/or `gridseak` (agent + GridSeak MCP).
    #[arg(long, default_value = "plain,gridseak")]
    pub arms: String,
    /// Spending cap per task run, in USD at API prices (passed to the agent).
    #[arg(long, default_value_t = 4.0)]
    pub max_usd_per_task: f64,
    /// Wall-clock limit per task run, in minutes.
    #[arg(long, default_value_t = 30)]
    pub timeout_min: u64,
    /// Let the agent write and run helper programs under `.gridseak/audit/scratch/`.
    /// Off by default: the agent gets read-only tools plus the language toolchain.
    #[arg(long, default_value_t = false)]
    pub allow_scratch: bool,
    /// Seed for target selection; the same seed and scan give the same tasks.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Write the task plan and answer keys, then stop without running an agent.
    #[arg(long, default_value_t = false)]
    pub plan_only: bool,
    /// Run each task this many times per arm; the card shows pass rates with 95% ranges.
    #[arg(long, default_value_t = 1)]
    pub reps: u32,
    /// Reuse the tasks (and answer keys) of an earlier audit's `tasks.json`
    /// instead of planning, so repeats of the same audit can be merged.
    #[arg(long)]
    pub plan: Option<PathBuf>,
    /// Re-render the report from an existing audit directory's `results.json`.
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// Merge finished audit directories (same tasks, e.g. one per model) into one
    /// comparison card written to `--out`.
    #[arg(long, num_args = 1..)]
    pub compare: Vec<PathBuf>,
    /// Output directory. Default: `<repo>/.gridseak/audit/<timestamp>/`.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Task family: `callers` (transitive-caller questions), `edits` (Go
    /// interface-change edits graded by the compiler; plain arm only) or `all`.
    #[arg(long, default_value = "callers", value_parser = ["callers", "edits", "all"])]
    pub family: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Caller {
    pub file: String,
    pub line: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub band: String,
    pub target: String,
    pub file: String,
    pub line: u32,
    pub question: String,
    pub max_depth: usize,
    pub answer: Vec<Caller>,
    /// `callers` (question) or `edit` (code change, see `audit_edit`).
    #[serde(default = "callers_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<crate::audit_edit::EditSpec>,
}

fn callers_kind() -> String {
    "callers".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Score {
    pub pass: bool,
    pub recall: f64,
    pub precision: f64,
    pub n_answer: usize,
    pub flagged_incomplete: bool,
    pub missed: Vec<String>,
    pub extra: Vec<String>,
    /// Answers in code the compiler does not build (e.g. Go `_examples/`); not scored.
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<crate::audit_edit::EditScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub task: String,
    pub arm: String,
    /// Model the run used; filled from the audit when older files lack it.
    #[serde(default)]
    pub model: String,
    #[serde(default = "one")]
    pub rep: u32,
    pub wall_s: f64,
    pub cost_usd: Option<f64>,
    pub turns: Option<u64>,
    pub tokens: Option<u64>,
    pub score: Score,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResults {
    pub repo: String,
    pub root: String,
    pub date: String,
    pub agent: String,
    pub model: String,
    pub mode: String,
    pub answer_key: String,
    pub call_edges: u64,
    pub tasks: Vec<Task>,
    pub runs: Vec<RunResult>,
}

fn one() -> u32 {
    1
}

const PASS_RECALL: f64 = 0.95;
const PASS_PRECISION: f64 = 0.9;

pub fn run(store: &ProjectStore, data_dir: &Path, args: AuditArgs) -> Result<()> {
    if let Some(dir) = &args.report {
        let raw = std::fs::read_to_string(dir.join("results.json"))
            .with_context(|| format!("read {}/results.json", dir.display()))?;
        let mut results: AuditResults = serde_json::from_str(&raw)?;
        regrade(dir, &mut results);
        write_reports(dir, &results)?;
        println!("Report written to {}", dir.join("report.html").display());
        return Ok(());
    }
    if !args.compare.is_empty() {
        let out = args.out.clone().context("--compare needs --out <dir>")?;
        let merged = merge_audits(&args.compare)?;
        std::fs::create_dir_all(&out)?;
        write_reports(&out, &merged)?;
        println!("Comparison card: {}", out.join("report.html").display());
        return Ok(());
    }
    if args.agent != "claude" {
        bail!(
            "only `--agent claude` is supported today (got `{}`)",
            args.agent
        );
    }
    let arms: Vec<String> = args
        .arms
        .split(',')
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    for a in &arms {
        if a != "plain" && a != "gridseak" {
            bail!("unknown arm `{a}`; use `plain` and/or `gridseak`");
        }
    }

    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans: run `gridseak scan .` first")?;
    let artifact = scan
        .graph_artifact_path
        .clone()
        .context("latest scan has no graph artifact")?;
    let root = project
        .roots
        .first()
        .map(|r| PathBuf::from(&r.path))
        .context("project has no root path")?;
    let conn = Connection::open(&artifact).with_context(|| format!("open {artifact}"))?;

    let graph = Graph::load(&conn, &root)?;
    let (want_callers, want_edits) = (args.family != "edits", args.family != "callers");
    if want_callers && (graph.call_edges == 0 || graph.compiler_edges < graph.call_edges) {
        bail!(
            "only {} of {} call edges in this scan come from the compiler index, so the answer key \
             would be partly guessed. Rescan with the compiler indexer available (`gridseak doctor \
             --provision <lang>`, then `gridseak scan .`) and try again.",
            graph.compiler_edges,
            graph.call_edges
        );
    }
    let reused = args.plan.is_some();
    let mut tasks = if let Some(plan) = &args.plan {
        let raw =
            std::fs::read_to_string(plan).with_context(|| format!("read {}", plan.display()))?;
        serde_json::from_str::<Vec<Task>>(&raw)
            .with_context(|| format!("parse {}", plan.display()))?
    } else if want_callers {
        graph.plan(args.tasks, args.seed, &project.display_name)
    } else {
        Vec::new()
    };
    if want_callers && tasks.is_empty() {
        bail!("no suitable targets: no function has a transitive caller set of 8–400 functions");
    }

    let out = args.out.clone().unwrap_or_else(|| {
        root.join(".gridseak/audit")
            .join(chrono::Local::now().format("%Y%m%d-%H%M%S").to_string())
    });
    std::fs::create_dir_all(&out)?;
    if want_edits && !reused {
        tasks.extend(crate::audit_edit::plan(&root, args.tasks, args.seed, &out)?);
    }
    std::fs::write(
        out.join("tasks.json"),
        serde_json::to_string_pretty(&tasks)?,
    )?;

    println!(
        "GridSeak audit · {} · {} call edges, all from the compiler",
        project.display_name, graph.call_edges
    );
    for t in &tasks {
        if t.edit.is_some() {
            println!("{}", crate::audit_edit::plan_line(t));
            continue;
        }
        println!(
            "  {:<6} {:<44} {:>4} callers, {} hops",
            t.band,
            t.target,
            t.answer.len(),
            t.max_depth
        );
    }
    if args.plan_only {
        println!("Plan written to {}", out.join("tasks.json").display());
        return Ok(());
    }

    let model = args.model.clone().unwrap_or_else(|| "agent default".into());
    let mut results = AuditResults {
        repo: project.display_name.clone(),
        root: root.display().to_string(),
        date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        agent: "Claude Code".into(),
        model: model.clone(),
        mode: if args.allow_scratch {
            "may write helper programs".into()
        } else {
            "read-only tools + language toolchain".into()
        },
        answer_key: format!(
            "GridSeak compiler-indexed call graph ({} edges, 100% from the compiler index){}",
            graph.call_edges,
            graph.coverage.key_note()
        ),
        call_edges: graph.call_edges,
        tasks: tasks.clone(),
        runs: Vec::new(),
    };
    crate::audit_edit::annotate(&mut results, want_callers);
    let mcp_cfg = out.join("mcp-gridseak.json");
    let exe = std::env::current_exe()?;
    std::fs::write(
        &mcp_cfg,
        serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": { "gridseak": { "command": exe, "args": ["--data-dir", data_dir, "mcp"] } }
        }))?,
    )?;

    for rep in 1..=args.reps.max(1) {
        for t in &tasks {
            for arm in &arms {
                if t.edit.is_some() && !crate::audit_edit::arm_applies(arm) {
                    println!(
                        "  r{rep} {arm:<8} {:<44} n/a: edit tasks run without GridSeak",
                        t.target
                    );
                    continue;
                }
                print!("  r{rep} {:<8} {:<44} ", arm, t.target);
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let mut r = if t.edit.is_some() {
                    crate::audit_edit::run_task(t, arm, &args, &root, &out)?
                } else {
                    run_claude(t, arm, rep, &args, &root, &mcp_cfg, &out)?
                };
                r.model = model.clone();
                r.rep = rep;
                println!(
                    "{}  {}  {:.1} min  {}",
                    verdict(&r.score),
                    crate::audit_edit::detail(&r.score).unwrap_or_else(|| format!(
                        "recall {:.2}  precision {:.2}",
                        r.score.recall, r.score.precision
                    )),
                    r.wall_s / 60.0,
                    r.cost_usd.map(|c| format!("${c:.2}")).unwrap_or_default()
                );
                results.runs.push(r);
                std::fs::write(
                    out.join("results.json"),
                    serde_json::to_string_pretty(&results)?,
                )?;
            }
        }
    }
    write_reports(&out, &results)?;
    println!("\nReport card: {}", out.join("report.html").display());
    Ok(())
}

fn verdict(s: &Score) -> &'static str {
    if s.edit.is_some() && s.error.is_none() {
        return match (s.pass, s.flagged_incomplete) {
            (true, _) => "PASS ",
            (false, true) => "FAIL (flagged)",
            (false, false) => "FAIL (silent) ",
        };
    }
    if s.error.is_some() {
        "ERROR"
    } else if s.pass {
        "PASS "
    } else if s.flagged_incomplete {
        "MISS (flagged)"
    } else {
        "MISS (silent) "
    }
}

// ---------------------------------------------------------------------------
// Graph and planning
// ---------------------------------------------------------------------------

struct Node {
    file: String,
    line: u32,
    name: String,
    callable: bool,
}

struct Graph {
    nodes: HashMap<String, Node>,
    /// callee -> direct static callers
    callers: HashMap<String, Vec<String>>,
    call_edges: u64,
    compiler_edges: u64,
    coverage: Coverage,
}

fn is_test_file(f: &str) -> bool {
    let base = f.rsplit('/').next().unwrap_or(f);
    base.ends_with("_test.go")
        || base.contains(".test.")
        || base.contains(".spec.")
        || base.starts_with("test_")
        || f.contains("/__tests__/")
}

impl Graph {
    fn load(conn: &Connection, root: &Path) -> Result<Self> {
        let (call_edges, compiler_edges): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(json_extract(provenance, '$.source') = 'Compiler'), 0)
             FROM edges WHERE json_extract(kind, '$.kind') = 'Call'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let root_s = format!("{}/", root.display());
        let mut nodes = HashMap::new();
        let mut files = HashSet::new();
        let mut stmt = conn.prepare(
            "SELECT id, kind, fqn, json_extract(location, '$.file'), json_extract(location, '$.start_line'),
                    COALESCE(json_extract(properties, '$.synthetic'), 0) FROM nodes",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5).unwrap_or(0) != 0,
            ))
        })?;
        for row in rows {
            let (id, kind, fqn, file, line, synthetic) = row?;
            let file = file.unwrap_or_default();
            let file = file.strip_prefix(&root_s).unwrap_or(&file).to_string();
            let name = fqn.rsplit("::").next().unwrap_or(&fqn).to_string();
            // `__file_scope__` stands for a TS/JS module's top-level code.
            let synthetic = synthetic || name == "__file_scope__" || name == "__file_module__";
            if kind == "File" {
                files.insert(file.clone());
            }
            nodes.insert(
                id,
                Node {
                    file,
                    line: line.unwrap_or(0) as u32,
                    name,
                    callable: (kind == "Function" || kind == "Method") && !synthetic,
                },
            );
        }
        let mut callers: HashMap<String, Vec<String>> = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT from_id, to_id FROM edges WHERE json_extract(kind, '$.kind') = 'Call'
             AND COALESCE(json_extract(provenance, '$.via'), 'call') = 'call'",
        )?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (from, to) = row?;
            if from != to {
                callers.entry(to).or_default().push(from);
            }
        }
        let coverage = Coverage::load(conn, &root_s, &files);
        Ok(Self {
            nodes,
            callers,
            call_edges: call_edges as u64,
            compiler_edges: compiler_edges as u64,
            coverage,
        })
    }

    /// Transitive callable callers of `seed` with BFS depth, stopping early past `cap`.
    fn closure(&self, seed: &str, cap: usize) -> (Vec<String>, usize) {
        let mut seen = HashSet::from([seed.to_string()]);
        let mut out = Vec::new();
        let mut max_depth = 0;
        let mut q = VecDeque::from([(seed.to_string(), 0usize)]);
        while let Some((id, d)) = q.pop_front() {
            for c in self.callers.get(&id).into_iter().flatten() {
                let Some(n) = self.nodes.get(c) else { continue };
                if !n.callable || !seen.insert(c.clone()) {
                    continue;
                }
                out.push(c.clone());
                max_depth = max_depth.max(d + 1);
                if out.len() > cap {
                    return (out, max_depth);
                }
                q.push_back((c.clone(), d + 1));
            }
        }
        (out, max_depth)
    }

    /// Whether any direct caller of the target or its callers sits in a
    /// partly parsed file, where a missing function could hide more callers.
    fn touches_partial(&self, seed: &str, closure: &[String]) -> bool {
        let partial = &self.coverage.partial;
        !partial.is_empty()
            && std::iter::once(seed)
                .chain(closure.iter().map(String::as_str))
                .flat_map(|id| self.callers.get(id).into_iter().flatten())
                .any(|c| self.nodes.get(c).is_some_and(|n| partial.contains(&n.file)))
    }

    fn plan(&self, n: usize, seed: u64, repo: &str) -> Vec<Task> {
        let bands = [
            ("small", 8usize, 30usize),
            ("medium", 31, 120),
            ("large", 121, 400),
        ];
        let cov = &self.coverage;
        let mut ids: Vec<&String> = self
            .nodes
            .iter()
            .filter(|(_, v)| v.callable && !is_test_file(&v.file) && !v.file.is_empty())
            .filter(|(_, v)| !cov.unindexed.contains(&v.file) && !cov.partial.contains(&v.file))
            .map(|(k, _)| k)
            .collect();
        ids.sort();
        let mut rng = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
            | 1;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for i in (1..ids.len()).rev() {
            ids.swap(i, (next() % (i as u64 + 1)) as usize);
        }
        let mut pools: Vec<Vec<(String, Vec<String>, usize)>> = vec![Vec::new(); bands.len()];
        for id in ids {
            let (cl, depth) = self.closure(id, 400);
            if depth < 2 || self.touches_partial(id, &cl) {
                continue;
            }
            if let Some(b) = bands
                .iter()
                .position(|(_, lo, hi)| cl.len() >= *lo && cl.len() <= *hi)
            {
                if pools[b].len() < 50 {
                    pools[b].push((id.clone(), cl, depth));
                }
            }
        }
        let mut chosen: Vec<(usize, String, Vec<String>, usize)> = Vec::new();
        let mut b = 0;
        let mut empty_rounds = 0;
        while chosen.len() < n && empty_rounds < bands.len() {
            let pick = pools[b].iter().position(|(id, cl, _)| {
                let set: HashSet<&String> = cl.iter().collect();
                chosen.iter().all(|(_, cid, ccl, _)| {
                    // Skip targets whose answers mostly repeat an earlier task's.
                    let overlap = ccl.iter().filter(|x| set.contains(x)).count();
                    cid != id && !set.contains(cid) && overlap * 2 <= ccl.len().min(cl.len())
                })
            });
            match pick {
                Some(i) => {
                    let (id, cl, d) = pools[b].remove(i);
                    chosen.push((b, id, cl, d));
                    empty_rounds = 0;
                }
                None => empty_rounds += 1,
            }
            b = (b + 1) % bands.len();
        }
        chosen.sort_by_key(|(b, _, cl, _)| (*b, cl.len()));
        chosen
            .into_iter()
            .enumerate()
            .map(|(i, (b, id, cl, depth))| {
                let t = &self.nodes[&id];
                let mut answer: Vec<Caller> = cl
                    .iter()
                    .map(|c| {
                        let n = &self.nodes[c];
                        Caller {
                            file: n.file.clone(),
                            line: n.line,
                            name: n.name.clone(),
                        }
                    })
                    .collect();
                answer.sort_by(|a, b| (a.file.as_str(), a.line).cmp(&(b.file.as_str(), b.line)));
                Task {
                    id: format!("t{}", i + 1),
                    band: bands[b].0.into(),
                    target: t.name.clone(),
                    file: t.file.clone(),
                    line: t.line,
                    question: question(repo, &t.name, &t.file, t.line)
                        + ts_rules(&t.file)
                        + &self.coverage.question_note(),
                    max_depth: depth,
                    answer,
                    kind: callers_kind(),
                    edit: None,
                }
            })
            .collect()
    }
}

fn question(repo: &str, name: &str, file: &str, line: u32) -> String {
    format!(
        "In this repository ({repo}), list every function and method, including those in test files, \
         that transitively calls `{name}` (declared at {file}:{line}) through chains of direct calls. \
         Follow only direct calls of named functions and methods: do not follow calls made through an \
         interface, abstract or trait method (dynamic dispatch), and do not follow a function being used \
         as a value (passed as an argument, assigned, or stored). A call inside an anonymous function, \
         closure or lambda counts as a call by the enclosing named function. Exclude `{name}` itself. \
         {scope}The list must be complete.",
        scope = scope_note(file)
    )
}

/// What "this repository" covers for the answer key, stated so agents and graders agree.
fn scope_note(target_file: &str) -> &'static str {
    if target_file.ends_with(".go") {
        "Consider only code the Go toolchain builds (`go build ./...` and `go test ./...`): skip \
         directories whose names start with `_` or `.`, and `testdata`. "
    } else {
        ""
    }
}

/// True when `file` sits in code the compiler never builds, so the answer key cannot cover it.
fn out_of_compiler_scope(target_file: &str, file: &str) -> bool {
    target_file.ends_with(".go")
        && file.split('/').any(|c| {
            c.starts_with('_')
                || (c.starts_with('.') && c.len() > 1 && c != "..")
                || c == "testdata"
        })
}

// ---------------------------------------------------------------------------
// Running the agent
// ---------------------------------------------------------------------------

const READ_ONLY: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "Bash(grep:*)",
    "Bash(rg:*)",
    "Bash(find:*)",
    "Bash(ls:*)",
    "Bash(cat:*)",
    "Bash(head:*)",
    "Bash(tail:*)",
    "Bash(wc:*)",
    "Bash(sed -n:*)",
    "Bash(go:*)",
    "Bash(gofmt:*)",
    "Bash(tsc:*)",
    "Bash(npx tsc:*)",
    "Bash(cargo:*)",
];
const SCRATCH: &[&str] = &[
    "Write",
    "Edit",
    "Bash(mkdir:*)",
    "Bash(python3:*)",
    "Bash(node:*)",
    "Bash(cd:*)",
    "Bash(sort:*)",
    "Bash(uniq:*)",
    "Bash(jq:*)",
];
const GS_TOOLS: &[&str] = &[
    "mcp__gridseak__gridseak_graph_blast_radius",
    "mcp__gridseak__gridseak_graph_callers",
];
const GS_HINT: &str = "GridSeak MCP tools are available for structural questions about this repository's call graph. \
They answer from a compiler-verified call graph: use them first, before searching files, then verify only what they cannot answer.";

fn run_claude(
    t: &Task,
    arm: &str,
    rep: u32,
    args: &AuditArgs,
    root: &Path,
    mcp_cfg: &Path,
    out: &Path,
) -> Result<RunResult> {
    let rule = if args.allow_scratch {
        "Do not modify the repository's existing files. You may create and run helper programs under ./.gridseak/audit/scratch/."
    } else {
        "Do not modify any files."
    };
    let prompt = format!(
        "You are working in the repository in the current directory. {rule}\n\n{}\n\nWhen you are done, end your reply \
         with a fenced ```json block in exactly this shape:\n{{\"callers\": [{{\"file\": \"repo/relative/path\", \"line\": 123, \"name\": \"FuncName or Type.Method\"}}]}}",
        t.question
    );
    let mut tools = vec!["Read", "Grep", "Glob", "Bash"];
    let mut allowed: Vec<&str> = READ_ONLY.to_vec();
    if args.allow_scratch {
        tools.extend(["Write", "Edit"]);
        allowed.extend(SCRATCH);
        std::fs::create_dir_all(root.join(".gridseak/audit/scratch"))?;
    }
    let mut cmd = Command::new("claude");
    cmd.current_dir(root)
        .arg("-p")
        .arg(&prompt)
        .args([
            "--output-format",
            "json",
            "--max-turns",
            "150",
            "--strict-mcp-config",
            "--no-session-persistence",
        ])
        .args(["--tools", &tools.join(",")])
        .args(["--max-budget-usd", &format!("{:.2}", args.max_usd_per_task)]);
    if let Some(m) = &args.model {
        cmd.args(["--model", m]);
    }
    if arm == "gridseak" {
        allowed.extend(GS_TOOLS);
        cmd.arg("--mcp-config")
            .arg(mcp_cfg)
            .args(["--append-system-prompt", GS_HINT]);
    }
    cmd.arg("--allowedTools").args(&allowed);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let mut child = cmd
        .spawn()
        .context("start `claude` (is Claude Code installed and logged in?)")?;
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let limit = Duration::from_secs(args.timeout_min * 60);
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = child.wait();
    let raw = reader.join().unwrap_or_default();
    let wall_s = start.elapsed().as_secs_f64();
    std::fs::write(out.join(format!("{}-{arm}-r{rep}.json", t.id)), &raw)?;

    let mut result = RunResult {
        task: t.id.clone(),
        arm: arm.into(),
        model: String::new(),
        rep,
        wall_s,
        cost_usd: None,
        turns: None,
        tokens: None,
        score: Score::default(),
    };
    let v: Value = match serde_json::from_str(raw.trim()) {
        Ok(v) => v,
        Err(_) => {
            result.score.error = Some(if timed_out {
                "timed out".into()
            } else {
                "agent produced no result".into()
            });
            return Ok(result);
        }
    };
    result.cost_usd = v["total_cost_usd"].as_f64();
    result.turns = v["num_turns"].as_u64();
    let u = &v["usage"];
    result.tokens = Some(
        [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .filter_map(|k| u[*k].as_u64())
        .sum(),
    );
    let text = v["result"].as_str().unwrap_or("");
    result.score = grade(t, text);
    Ok(result)
}

/// Re-grade every run from its saved raw agent output with the current grader.
fn regrade(dir: &Path, r: &mut AuditResults) {
    for run in &mut r.runs {
        let Some(t) = r.tasks.iter().find(|t| t.id == run.task) else {
            continue;
        };
        let candidates = [
            dir.join(format!("{}-{}-r{}.json", run.task, run.arm, run.rep)),
            dir.join(format!("{}-{}.json", run.task, run.arm)),
        ];
        let Some(raw) = candidates
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
        else {
            continue;
        };
        if let Ok(v) = serde_json::from_str::<Value>(raw.trim()) {
            if let Some(text) = v["result"].as_str() {
                run.score = grade(t, text);
            }
        }
    }
}

fn last_json_block(text: &str) -> Option<Value> {
    let mut found = None;
    let mut rest = text;
    while let Some(i) = rest.find("```json") {
        let body = &rest[i + 7..];
        let Some(j) = body.find("```") else { break };
        if let Ok(v) = serde_json::from_str::<Value>(body[..j].trim()) {
            found = Some(v);
        }
        rest = &body[j + 3..];
    }
    found
}

fn flags_incomplete(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "incomplete",
        "not complete",
        "partial list",
        "lower bound",
        "could not finish",
        "couldn't finish",
        "not exhaustive",
        "unable to complete",
    ]
    .iter()
    .any(|p| t.contains(p))
}

pub fn grade(t: &Task, text: &str) -> Score {
    let mut s = Score {
        flagged_incomplete: flags_incomplete(text),
        ..Default::default()
    };
    let Some(v) = last_json_block(text) else {
        s.error = Some("no parsable JSON answer".into());
        return s;
    };
    let truth: HashSet<(String, u32)> = t.answer.iter().map(|c| (c.file.clone(), c.line)).collect();
    let mut by_name: HashMap<(String, String), Vec<(String, u32)>> = HashMap::new();
    for c in &t.answer {
        let bare = c.name.rsplit('.').next().unwrap_or(&c.name).to_string();
        by_name
            .entry((c.file.clone(), bare))
            .or_default()
            .push((c.file.clone(), c.line));
    }
    let mut got: HashSet<(String, u32)> = HashSet::new();
    let mut unmatched: Vec<String> = Vec::new();
    for c in v["callers"].as_array().into_iter().flatten() {
        let file = c["file"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("./")
            .to_string();
        if out_of_compiler_scope(&t.file, &file) {
            s.out_of_scope.push(format!(
                "{file}:{} {}",
                c["line"].as_u64().unwrap_or(0),
                c["name"].as_str().unwrap_or("")
            ));
            continue;
        }
        let line = c["line"]
            .as_u64()
            .or_else(|| c["line"].as_str().and_then(|l| l.parse().ok()))
            .unwrap_or(0) as u32;
        let name = c["name"].as_str().unwrap_or("");
        let bare = name
            .rsplit('.')
            .next()
            .unwrap_or(name)
            .trim_matches(|ch: char| ch == '(' || ch == ')' || ch == '*' || ch == ' ')
            .to_string();
        let key = (file.clone(), line);
        if truth.contains(&key) {
            got.insert(key);
        } else if let Some(v) = by_name
            .get(&(file.clone(), bare.clone()))
            .filter(|v| v.len() == 1)
        {
            got.insert(v[0].clone());
        } else {
            unmatched.push(format!("{file}:{line} {name}"));
        }
    }
    let tp = got.len();
    s.n_answer = tp + unmatched.len();
    s.recall = if truth.is_empty() {
        1.0
    } else {
        tp as f64 / truth.len() as f64
    };
    s.precision = if s.n_answer == 0 {
        0.0
    } else {
        tp as f64 / s.n_answer as f64
    };
    s.pass = s.recall >= PASS_RECALL && s.precision >= PASS_PRECISION;
    let mut missed: Vec<String> = t
        .answer
        .iter()
        .filter(|c| !got.contains(&(c.file.clone(), c.line)))
        .map(|c| format!("{}:{} {}", c.file, c.line, c.name))
        .collect();
    missed.truncate(25);
    unmatched.truncate(25);
    s.missed = missed;
    s.extra = unmatched;
    s
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

struct ArmSummary {
    model: String,
    arm: String,
    runs: usize,
    pass: usize,
    silent: usize,
    flagged: usize,
    errors: usize,
    minutes: f64,
    cost: f64,
    tokens: u64,
}

impl ArmSummary {
    /// Wilson 95% interval for the pass rate, as whole percents.
    fn pass_range(&self) -> (f64, f64) {
        if self.runs == 0 {
            return (0.0, 0.0);
        }
        let n = self.runs as f64;
        let p = self.pass as f64 / n;
        let z = 1.96_f64;
        let denom = 1.0 + z * z / n;
        let centre = (p + z * z / (2.0 * n)) / denom;
        let half = z * ((p * (1.0 - p) / n) + z * z / (4.0 * n * n)).sqrt() / denom;
        (
            ((centre - half).max(0.0) * 100.0).round(),
            ((centre + half).min(1.0) * 100.0).round(),
        )
    }

    fn cost_per_correct(&self) -> Option<f64> {
        (self.pass > 0 && self.cost > 0.0).then(|| self.cost / self.pass as f64)
    }
}

fn summarize(r: &AuditResults) -> Vec<ArmSummary> {
    let mut by: BTreeMap<(String, u8, String), Vec<&RunResult>> = BTreeMap::new();
    for run in &r.runs {
        let model = if run.model.is_empty() {
            r.model.clone()
        } else {
            run.model.clone()
        };
        let order = if run.arm == "plain" { 0 } else { 1 };
        by.entry((model, order, run.arm.clone()))
            .or_default()
            .push(run);
    }
    by.into_iter()
        .map(|((model, _, arm), runs)| ArmSummary {
            runs: runs.len(),
            pass: runs.iter().filter(|x| x.score.pass).count(),
            silent: runs
                .iter()
                .filter(|x| !x.score.pass && x.score.error.is_none() && !x.score.flagged_incomplete)
                .count(),
            flagged: runs
                .iter()
                .filter(|x| !x.score.pass && x.score.error.is_none() && x.score.flagged_incomplete)
                .count(),
            errors: runs.iter().filter(|x| x.score.error.is_some()).count(),
            minutes: runs.iter().map(|x| x.wall_s).sum::<f64>() / 60.0,
            cost: runs.iter().filter_map(|x| x.cost_usd).sum(),
            tokens: runs.iter().filter_map(|x| x.tokens).sum(),
            model,
            arm,
        })
        .collect()
}

/// `claude-opus-5-5` -> `Opus 5.5`; anything else passes through.
fn model_name(m: &str) -> String {
    let core = m.strip_prefix("claude-").unwrap_or(m);
    let mut parts = core.split('-');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(fam), Some(a), Some(b), None) if a.chars().all(|c| c.is_ascii_digit()) => {
            let mut f = fam.to_string();
            if let Some(c) = f.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            format!("{f} {a}.{b}")
        }
        _ => m.to_string(),
    }
}

fn arm_label(arm: &str) -> &'static str {
    if arm == "gridseak" {
        "Agent + GridSeak"
    } else {
        "Agent alone"
    }
}

fn group_label(a: &ArmSummary, multi_model: bool) -> String {
    if multi_model {
        let m = model_name(&a.model);
        if a.arm == "gridseak" {
            format!("{m} + GridSeak")
        } else {
            format!("{m} alone")
        }
    } else {
        arm_label(&a.arm).to_string()
    }
}

fn run_label(r: &AuditResults, run: &RunResult, multi_model: bool) -> String {
    let model = if run.model.is_empty() {
        &r.model
    } else {
        &run.model
    };
    if multi_model {
        let m = model_name(model);
        if run.arm == "gridseak" {
            format!("{m} + GridSeak")
        } else {
            format!("{m} alone")
        }
    } else {
        arm_label(&run.arm).to_string()
    }
}

fn is_multi_model(r: &AuditResults) -> bool {
    let models: HashSet<&str> = r
        .runs
        .iter()
        .map(|x| {
            if x.model.is_empty() {
                r.model.as_str()
            } else {
                x.model.as_str()
            }
        })
        .collect();
    models.len() > 1
}

/// Merge audit directories that ran the same tasks (e.g. one per model).
pub fn merge_audits(dirs: &[PathBuf]) -> Result<AuditResults> {
    let mut merged: Option<AuditResults> = None;
    for dir in dirs {
        let raw = std::fs::read_to_string(dir.join("results.json"))
            .with_context(|| format!("read {}/results.json", dir.display()))?;
        let mut r: AuditResults = serde_json::from_str(&raw)?;
        for run in &mut r.runs {
            if run.model.is_empty() {
                run.model = r.model.clone();
            }
        }
        match &mut merged {
            None => merged = Some(r),
            Some(m) => {
                let key = |ts: &[Task]| -> Vec<(String, String, u32)> {
                    ts.iter()
                        .map(|t| (t.id.clone(), t.file.clone(), t.line))
                        .collect()
                };
                if key(&m.tasks) != key(&r.tasks) {
                    bail!(
                        "{} ran different tasks; compare audits made with the same scan, --tasks and --seed",
                        dir.display()
                    );
                }
                if !m.model.split(", ").any(|x| x == r.model) {
                    m.model = format!("{}, {}", m.model, r.model);
                }
                m.runs.extend(r.runs);
            }
        }
    }
    merged.context("no audit directories given")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn write_reports(dir: &Path, r: &AuditResults) -> Result<()> {
    std::fs::write(dir.join("results.json"), serde_json::to_string_pretty(r)?)?;
    std::fs::write(dir.join("report.md"), render_markdown(r))?;
    std::fs::write(dir.join("report.html"), render_html(r))?;
    Ok(())
}

fn headline(r: &AuditResults, groups: &[ArmSummary]) -> String {
    let multi = is_multi_model(r);
    let mut parts = Vec::new();
    let mut models: Vec<&str> = Vec::new();
    for g in groups {
        if !models.contains(&g.model.as_str()) {
            models.push(&g.model);
        }
    }
    for m in &models {
        let plain = groups.iter().find(|g| g.model == *m && g.arm == "plain");
        let gs = groups.iter().find(|g| g.model == *m && g.arm == "gridseak");
        let who = if multi {
            model_name(m)
        } else {
            "The agent".into()
        };
        parts.push(match (plain, gs) {
            (Some(p), Some(g)) => format!(
                "{who} alone was exact on {} of {}; with GridSeak, {} of {}.",
                p.pass, p.runs, g.pass, g.runs
            ),
            (Some(p), None) => format!("{who} was exact on {} of {}.", p.pass, p.runs),
            (None, Some(g)) => {
                format!("{who} with GridSeak was exact on {} of {}.", g.pass, g.runs)
            }
            _ => String::new(),
        });
    }
    // Cheapest way to a correct answer among the groups with the best pass rate.
    let best_rate = groups
        .iter()
        .map(|g| g.pass as f64 / g.runs.max(1) as f64)
        .fold(0.0, f64::max);
    if let Some(best) = groups
        .iter()
        .filter(|g| (g.pass as f64 / g.runs.max(1) as f64) >= best_rate - 1e-9)
        .filter_map(|g| g.cost_per_correct().map(|c| (g, c)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
    {
        if groups.len() > 1 {
            parts.push(format!(
                "Cheapest correct answers: {} at ${:.2} each.",
                group_label(best.0, multi),
                best.1
            ));
        }
    }
    parts.join(" ")
}

pub fn render_markdown(r: &AuditResults) -> String {
    let groups = summarize(r);
    let multi = is_multi_model(r);
    let mut s = format!(
        "# Agent report card: {}\n\n{} · {} · {} · {}\n\n{}\n\nAnswer key: {}.\n\n| Setup | Exact answers | Pass rate (95% range) | Silent misses | Flagged misses | Minutes | Cost | Cost per correct answer | Tokens |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|\n",
        r.repo,
        r.date,
        r.agent,
        r.model,
        r.mode,
        headline(r, &groups),
        r.answer_key
    );
    for a in &groups {
        let (lo, hi) = a.pass_range();
        s +=
            &format!(
            "| {} | {}/{} | {:.0}% ({lo:.0}–{hi:.0}%) | {} | {} | {:.1} | ${:.2} | {} | {:.2}M |\n",
            group_label(a, multi),
            a.pass,
            a.runs,
            100.0 * a.pass as f64 / a.runs.max(1) as f64,
            a.silent,
            a.flagged + a.errors,
            a.minutes,
            a.cost,
            a.cost_per_correct().map(|c| format!("${c:.2}")).unwrap_or_else(|| "n/a".into()),
            a.tokens as f64 / 1e6
        );
    }
    s += "\n| Task | Target | True callers | Hops | Setup | Rep | Result | Recall | Precision | Min | Cost |\n|---|---|---:|---:|---|---:|---|---:|---:|---:|---:|\n";
    for t in &r.tasks {
        for run in r.runs.iter().filter(|x| x.task == t.id) {
            let (size, hops) = crate::audit_edit::size_cells(t)
                .unwrap_or_else(|| (t.answer.len().to_string(), t.max_depth.to_string()));
            let rp = crate::audit_edit::detail(&run.score)
                .map(|d| format!("{d} | —"))
                .unwrap_or_else(|| format!("{:.2} | {:.2}", run.score.recall, run.score.precision));
            s += &format!(
                "| {} ({}) | `{}` ({}:{}) | {} | {} | {} | {} | {} | {} | {:.1} | {} |\n",
                t.id,
                t.kind,
                t.target,
                t.file,
                t.line,
                size,
                hops,
                run_label(r, run, multi),
                run.rep,
                verdict(&run.score).trim(),
                rp,
                run.wall_s / 60.0,
                run.cost_usd.map(|c| format!("${c:.2}")).unwrap_or_default()
            );
        }
    }
    if let Some(note) = crate::audit_edit::report_note(r) {
        s += &format!("\n{note} For edit tasks the Recall column holds the build, vet and signature checks.\n");
    }
    s += "\nA miss is **silent** when the agent presented its list without saying it was incomplete. Pass: recall ≥ 0.95 and precision ≥ 0.9. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices.\n\nReproduce: `gridseak audit --tasks N --seed S` (see `tasks.json`).\n";
    s
}

pub fn render_html(r: &AuditResults) -> String {
    let groups = summarize(r);
    let multi = is_multi_model(r);
    let mut cards = String::new();
    for a in &groups {
        let us = a.arm == "gridseak";
        let (lo, hi) = a.pass_range();
        cards += &format!(
            r#"<div class="arm{cls}"><div class="lbl">{label}</div><div class="big">{pass}<span>/{runs}</span></div><div class="sub">exact answers · 95% range {lo:.0}–{hi:.0}%</div>
<dl><div><dt>Silent misses</dt><dd class="{silent_cls}">{silent}</dd></div><div><dt>Flagged misses</dt><dd>{flagged}</dd></div>
<div><dt>Time</dt><dd>{min:.1} min</dd></div><div><dt>Cost</dt><dd>${cost:.2}</dd></div><div><dt>Per correct</dt><dd>{cpc}</dd></div><div><dt>Tokens</dt><dd>{tok:.2}M</dd></div></dl></div>"#,
            cls = if us { " us" } else { "" },
            label = esc(&group_label(a, multi)),
            pass = a.pass,
            runs = a.runs,
            silent = a.silent,
            silent_cls = if a.silent > 0 { "warn" } else { "" },
            flagged = a.flagged + a.errors,
            min = a.minutes,
            cost = a.cost,
            cpc = a
                .cost_per_correct()
                .map(|c| format!("${c:.2}"))
                .unwrap_or_else(|| "n/a".into()),
            tok = a.tokens as f64 / 1e6,
        );
    }
    let mut rows = String::new();
    for t in &r.tasks {
        for run in r.runs.iter().filter(|x| x.task == t.id) {
            let v = verdict(&run.score).trim();
            let cls = if run.score.pass {
                "ok"
            } else if v.contains("silent") {
                "bad"
            } else {
                "meh"
            };
            let (size, hops) = crate::audit_edit::size_cells(t)
                .unwrap_or_else(|| (t.answer.len().to_string(), t.max_depth.to_string()));
            let rp = crate::audit_edit::detail(&run.score)
                .unwrap_or_else(|| format!("{:.2} / {:.2}", run.score.recall, run.score.precision));
            rows += &format!(
                "<tr><td><code>{}</code><small>{} · {}:{}</small></td><td>{}</td><td>{}</td><td>{}</td><td class=\"{cls}\">{}</td><td>{}</td><td>{:.1}</td><td>{}</td></tr>",
                esc(&t.target), esc(&t.kind), esc(&t.file), t.line, esc(&size), esc(&hops), esc(&run_label(r, run, multi)), v,
                esc(&rp), run.wall_s / 60.0,
                run.cost_usd.map(|c| format!("${c:.2}")).unwrap_or_default()
            );
        }
    }
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Agent report card · {repo}</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Martian+Mono:wdth,wght@75..112.5,200..800&family=Geist:wght@300..700&family=Silkscreen&display=swap">
<style>
:root{{--void:#020403;--panel:#050d08;--rule:#12301f;--phos:#3dff86;--hi:#c8ffdc;--dim:#1e7a47;--ink:#c5dfcd;--mute:#6d917a;--amber:#ffb340;--red:#ff6b5a;color-scheme:dark}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--void);color:var(--ink);font:15px/1.55 Geist,system-ui,sans-serif;padding:clamp(16px,4vw,48px)}}
.wrap{{max-width:1100px;margin:0 auto;display:grid;gap:28px}}
.top{{display:flex;justify-content:space-between;gap:16px;flex-wrap:wrap;align-items:baseline;border-bottom:1px solid var(--rule);padding-bottom:14px}}
.brand{{font:12px Silkscreen,monospace;letter-spacing:.14em;color:var(--phos)}}.meta{{font:11px "Martian Mono",monospace;font-stretch:87.5%;color:var(--mute)}}
h1{{margin:0;font:760 clamp(1.8rem,4.5vw,3rem)/1.02 "Martian Mono",monospace;font-stretch:108%;letter-spacing:-.03em;color:var(--hi)}}
h1 em{{font-style:normal;color:var(--phos);text-shadow:0 0 18px rgba(61,255,134,.5)}}
.lede{{font-size:1.1rem;max-width:68ch;margin:0}}
.arms{{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:16px}}
.arm{{border:1px solid var(--rule);background:var(--panel);padding:20px;display:grid;gap:6px;min-width:0}}.arm.us{{border-color:var(--phos);box-shadow:0 0 36px rgba(61,255,134,.1)}}
.lbl{{font:11px Silkscreen,monospace;letter-spacing:.12em;color:var(--mute)}}.big{{font:760 4rem/1 "Martian Mono",monospace;color:var(--phos);text-shadow:0 0 22px rgba(61,255,134,.4)}}.big span{{font-size:1.6rem;color:var(--dim)}}
.sub{{color:var(--mute);font-size:13px}}dl{{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px;margin:10px 0 0}}dt{{font-size:11px;color:var(--mute)}}dd{{margin:0;font:600 15px "Martian Mono",monospace;font-stretch:90%;color:var(--hi)}}dd.warn{{color:var(--amber)}}
.tbl{{overflow-x:auto;border:1px solid var(--rule)}}table{{border-collapse:collapse;width:100%;font-size:13px;min-width:680px}}
th,td{{padding:9px 12px;border-bottom:1px solid var(--rule);text-align:left;vertical-align:top}}th{{font:10px Silkscreen,monospace;letter-spacing:.1em;color:var(--mute);font-weight:400}}
td code{{font:12px "Martian Mono",monospace;font-stretch:87.5%;color:var(--hi);display:block}}td small{{color:var(--mute);font-size:11px}}
td.ok{{color:var(--phos)}}td.bad{{color:var(--red)}}td.meh{{color:var(--amber)}}
.fine{{font-size:12.5px;color:var(--mute);max-width:80ch}}.fine code{{color:var(--ink)}}
</style></head><body><div class="wrap">
<div class="top"><span class="brand">GRIDSEAK · AGENT REPORT CARD</span><span class="meta">{date} · {agent} · {model} · {mode}</span></div>
<h1>Can you trust your agent on <em>{repo}</em>?</h1>
<p class="lede">{headline} Each question asks for every function that transitively calls a target. The answers come from the compiler, not from another model.</p>
<div class="arms">{cards}</div>
<div class="tbl"><table><thead><tr><th>Target</th><th>True callers</th><th>Hops</th><th>Setup</th><th>Result</th><th>Recall / precision</th><th>Min</th><th>Cost</th></tr></thead><tbody>{rows}</tbody></table></div>
{edit_note}<p class="fine">Answer key: {key}. Pass: recall ≥ 0.95 and precision ≥ 0.9. A <b>silent</b> miss is an incomplete list presented as complete. Pass-rate ranges are Wilson 95% intervals. Costs are the agent's own report at API prices. Reproduce with <code>gridseak audit</code> in this repository; tasks and raw agent output are next to this file.</p>
</div></body></html>"#,
        repo = esc(&r.repo),
        date = esc(&r.date),
        agent = esc(&r.agent),
        model = esc(&r.model),
        mode = esc(&r.mode),
        headline = esc(&headline(r, &groups)),
        key = esc(&r.answer_key),
        edit_note = crate::audit_edit::report_note(r)
            .map(|n| format!("<p class=\"fine\">{}</p>", esc(&n)))
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task {
            id: "t1".into(),
            band: "small".into(),
            target: "T".into(),
            file: "a.go".into(),
            line: 1,
            question: String::new(),
            max_depth: 2,
            kind: callers_kind(),
            edit: None,
            answer: vec![
                Caller {
                    file: "a.go".into(),
                    line: 10,
                    name: "A".into(),
                },
                Caller {
                    file: "b.go".into(),
                    line: 20,
                    name: "S.B".into(),
                },
            ],
        }
    }

    #[test]
    fn grades_exact_answer_and_name_fallback() {
        let text = "done\n```json\n{\"callers\":[{\"file\":\"a.go\",\"line\":10,\"name\":\"A\"},{\"file\":\"./b.go\",\"line\":19,\"name\":\"S.B\"}]}\n```";
        let s = grade(&task(), text);
        assert!(s.pass, "{s:?}");
        assert_eq!(s.n_answer, 2);
    }

    #[test]
    fn silent_and_flagged_misses_differ() {
        let partial =
            "```json\n{\"callers\":[{\"file\":\"a.go\",\"line\":10,\"name\":\"A\"}]}\n```";
        let silent = grade(&task(), partial);
        assert!(!silent.pass && !silent.flagged_incomplete);
        let flagged = grade(&task(), &format!("This list is incomplete.\n{partial}"));
        assert!(!flagged.pass && flagged.flagged_incomplete);
    }

    #[test]
    fn extra_callers_lower_precision() {
        let text = "```json\n{\"callers\":[{\"file\":\"a.go\",\"line\":10},{\"file\":\"b.go\",\"line\":20},{\"file\":\"c.go\",\"line\":1},{\"file\":\"d.go\",\"line\":1}]}\n```";
        let s = grade(&task(), text);
        assert_eq!(s.recall, 1.0);
        assert!(s.precision < 0.9 && !s.pass);
    }

    #[test]
    fn wilson_range_and_model_names() {
        let a = ArmSummary {
            model: "claude-opus-5-5".into(),
            arm: "plain".into(),
            runs: 4,
            pass: 2,
            silent: 1,
            flagged: 1,
            errors: 0,
            minutes: 1.0,
            cost: 4.0,
            tokens: 0,
        };
        let (lo, hi) = a.pass_range();
        assert!(
            lo > 10.0 && lo < 20.0 && hi > 80.0 && hi < 90.0,
            "{lo} {hi}"
        );
        assert_eq!(a.cost_per_correct(), Some(2.0));
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("agent default"), "agent default");
    }

    #[test]
    fn go_code_outside_the_build_is_not_scored() {
        let text = "```json\n{\"callers\":[{\"file\":\"a.go\",\"line\":10},{\"file\":\"b.go\",\"line\":20},{\"file\":\"_examples/x/main.go\",\"line\":5,\"name\":\"main\"},{\"file\":\"pkg/testdata/t.go\",\"line\":3}]}\n```";
        let s = grade(&task(), text);
        assert!(s.pass, "{s:?}");
        assert_eq!(s.out_of_scope.len(), 2);
        assert!(!out_of_compiler_scope("a.go", "internal/x.go"));
        assert!(!out_of_compiler_scope("a.ts", "_x/y.ts"));
    }

    #[test]
    fn missing_json_is_an_error() {
        assert!(grade(&task(), "no answer").error.is_some());
    }
}
