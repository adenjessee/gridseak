//! Edit-task family for `gridseak audit` (`--family edits`): grade a coding
//! agent on a real code change, not just a question.
//!
//! Task: "add `ctx context.Context` as the first parameter of `I.M`; update
//! every implementation, fake and caller". Go only.
//!
//! Plan: build the embedded `ifacegen` helper (go/types) once into the audit
//! directory, copy the repository to a throwaway directory, list candidate
//! interfaces with their implementations (including `_test.go` fakes) and call
//! sites, and generate hidden assertion files (`var _ I = T{}` per
//! implementation). A candidate is kept only if the unmodified copy plus its
//! assertions builds and vets cleanly.
//! Run: each run gets a fresh copy (copy-on-write on macOS, mtimes kept) and a
//! headless Claude Code session with write tools. The user's working tree is
//! never touched.
//! Grade: write the hidden assertions into the edited copy, then `go build`
//! the module and `go vet` the affected packages, and check that the
//! interface method's first parameter is now a `context.Context`.
//!
//! The `gridseak` arm does not apply to edit tasks yet: GridSeak's MCP server
//! answers from the scan of the original path, and the agent works in a copy
//! at another path that has no scan (scanning every copy would be slow and the
//! scan would go stale as soon as the agent edits). Edit tasks therefore run
//! in the `plain` arm only, and the report says so.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audit::{AuditArgs, AuditResults, RunResult, Score, Task};

const IFACEGEN_MAIN: &str = include_str!("../assets/ifacegen/main.go");
const IFACEGEN_MOD: &str = include_str!("../assets/ifacegen/go.mod");
const IFACEGEN_SUM: &str = include_str!("../assets/ifacegen/go.sum");

/// Task kind stored on [`Task::kind`] for edit tasks.
pub const KIND: &str = "edit";

/// Most implementations an interface may have to be a task. Interfaces with
/// more are usually satisfied by accident (e.g. one-method marker interfaces)
/// and would make the task about churn, not about finding every site.
const MAX_IMPLS: usize = 12;

/// Method names shared with standard-library interfaces: changing them can
/// break an unrelated `io.Writer`/`fmt.Stringer`/`http.Handler` use that the
/// agent cannot fix without redesign, so they make ill-posed tasks.
const STDLIB_METHODS: &[&str] = &[
    "String",
    "Error",
    "Close",
    "Read",
    "Write",
    "Len",
    "Less",
    "Swap",
    "Unwrap",
    "Format",
    "MarshalJSON",
    "UnmarshalJSON",
    "MarshalText",
    "UnmarshalText",
    "Is",
    "As",
    "Seek",
    "Flush",
    "Hijack",
    "Lock",
    "Unlock",
    "Next",
];

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// Everything needed to run and grade one edit task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditSpec {
    pub pkg: String,
    pub iface: String,
    pub method: String,
    pub iface_file: String,
    pub impls: usize,
    pub test_impls: usize,
    pub call_sites: usize,
    /// Packages (`./dir`) to vet after the edit: the interface's, every
    /// implementation's and every caller's.
    pub vet_dirs: Vec<String>,
    /// Hidden assertion files written into the copy before grading.
    pub assert_files: BTreeMap<String, String>,
    /// Implementations that cannot be asserted without an import cycle
    /// (unexported or test-only types in a package the interface's package
    /// imports). Their misses surface only if the build or vet breaks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unasserted: Vec<String>,
}

/// Output of `ifacegen assert`.
#[derive(Debug, Deserialize)]
struct Asserts {
    files: BTreeMap<String, String>,
    #[serde(default)]
    skipped: Option<Vec<String>>,
}

/// Edit-specific part of a run's grade.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EditScore {
    pub build_ok: bool,
    pub vet_ok: bool,
    pub signature_changed: bool,
    pub files_changed: usize,
    pub n_errors: usize,
    pub errors: Vec<String>,
    /// Errors in the hidden assertion files: an implementation was missed.
    pub assert_errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Impl {
    #[serde(default)]
    dir: String,
    #[serde(default)]
    test: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Method {
    name: String,
    #[serde(default)]
    ctx_first: bool,
    #[serde(default)]
    call_sites: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct Cand {
    pkg: String,
    dir: String,
    file: String,
    line: u32,
    iface: String,
    #[serde(default)]
    methods: Option<Vec<Method>>,
    #[serde(default)]
    impls: Option<Vec<Impl>>,
    #[serde(default)]
    caller_dirs: Option<Vec<String>>,
}

impl Cand {
    fn impls(&self) -> &[Impl] {
        self.impls.as_deref().unwrap_or(&[])
    }
    fn methods(&self) -> &[Method] {
        self.methods.as_deref().unwrap_or(&[])
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Plan up to `n` interface-change tasks for the Go module at `root`.
pub fn plan(root: &Path, n: usize, seed: u64, out: &Path) -> Result<Vec<Task>> {
    require_go_module(root)?;
    let gen = build_ifacegen(out)?;
    let clean = work_dir("plan");
    println!("  edits: copying the repository to {} …", clean.display());
    copy_repo(root, &clean, out)?;
    let result = plan_in(&clean, &gen, n, seed, out);
    let _ = std::fs::remove_dir_all(&clean);
    result
}

fn plan_in(clean: &Path, gen: &Path, n: usize, seed: u64, out: &Path) -> Result<Vec<Task>> {
    let listed = run_capture(Command::new(gen).arg("list").arg(clean), clean)?;
    if !listed.ok {
        bail!(
            "listing interfaces failed (does the module load with `go list ./...`?):\n{}",
            tail(&listed.output, 1500)
        );
    }
    let cands: Vec<Cand> =
        serde_json::from_str(listed.stdout.trim()).context("parse ifacegen list output")?;
    let picks = select(&cands, seed);
    if picks.is_empty() {
        bail!("no suitable interfaces: need an exported interface with 1–{MAX_IMPLS} implementations and a called method without a context parameter");
    }

    let build = go_build(clean);
    if !build.ok {
        bail!(
            "the unmodified repository does not build (`go build ./...`), so edit tasks have no clean baseline:\n{}",
            tail(&build.output, 1500)
        );
    }
    let mut tasks = Vec::new();
    let mut rejected = Vec::new();
    for (c, mi) in picks {
        if tasks.len() >= n {
            break;
        }
        let m = &c.methods()[mi];
        let gen_out = run_capture(
            Command::new(gen)
                .arg("assert")
                .arg(clean)
                .arg(&c.pkg)
                .arg(&c.iface),
            clean,
        )?;
        if !gen_out.ok {
            rejected.push(format!(
                "{}.{}: assertion generation failed",
                c.iface, m.name
            ));
            continue;
        }
        let Asserts {
            files: assert_files,
            skipped: unasserted,
        } = serde_json::from_str(gen_out.stdout.trim()).context("parse ifacegen assert output")?;
        let vet_dirs = vet_dirs(&c);
        write_asserts(clean, &assert_files)?;
        let vet = go_vet(clean, &vet_dirs);
        remove_asserts(clean, &assert_files);
        if !vet.ok {
            let why = go_errors(&vet.output)
                .first()
                .cloned()
                .unwrap_or_else(|| tail(vet.output.trim(), 300).replace('\n', " "));
            rejected.push(format!(
                "{}.{}: baseline vet fails with the assertions: {why}",
                c.iface, m.name
            ));
            continue;
        }
        let spec = EditSpec {
            pkg: c.pkg.clone(),
            iface: c.iface.clone(),
            method: m.name.clone(),
            iface_file: c.file.clone(),
            impls: c.impls().len(),
            test_impls: c.impls().iter().filter(|i| i.test).count(),
            call_sites: m.call_sites,
            vet_dirs,
            assert_files,
            unasserted: unasserted.unwrap_or_default(),
        };
        tasks.push(Task {
            id: format!("e{}", tasks.len() + 1),
            band: band(spec.impls).into(),
            target: format!("{}.{}", c.iface, m.name),
            file: c.file.clone(),
            line: c.line,
            question: question(&c.iface, &m.name, &c.file, &c.pkg),
            max_depth: 0,
            answer: Vec::new(),
            kind: KIND.into(),
            edit: Some(spec),
        });
    }
    if !rejected.is_empty() {
        std::fs::write(out.join("edits-rejected.txt"), rejected.join("\n") + "\n")?;
    }
    if tasks.is_empty() {
        bail!(
            "no edit task passed the baseline check (unmodified code + hidden assertions must build and vet); see {}",
            out.join("edits-rejected.txt").display()
        );
    }
    Ok(tasks)
}

fn require_go_module(root: &Path) -> Result<()> {
    if !root.join("go.mod").is_file() {
        bail!(
            "edit tasks (`--family edits`) support Go modules only, and {} has no go.mod at its root. \
             Use `--family callers` for other languages.",
            root.display()
        );
    }
    if Command::new("go")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        bail!("edit tasks need the Go toolchain (`go`) on PATH");
    }
    Ok(())
}

fn build_ifacegen(out: &Path) -> Result<PathBuf> {
    let dir = out.join("ifacegen");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("main.go"), IFACEGEN_MAIN)?;
    std::fs::write(dir.join("go.mod"), IFACEGEN_MOD)?;
    std::fs::write(dir.join("go.sum"), IFACEGEN_SUM)?;
    let bin = dir.join("ifacegen");
    let r = run_capture(
        Command::new("go")
            .args(["build", "-o"])
            .arg(&bin)
            .arg(".")
            .env("GOWORK", "off")
            .env("GOFLAGS", "-mod=mod"),
        &dir,
    )?;
    if !r.ok {
        bail!(
            "building the interface helper failed (it needs golang.org/x/tools from the Go module proxy):\n{}",
            tail(&r.output, 1500)
        );
    }
    Ok(bin)
}

fn band(impls: usize) -> &'static str {
    match impls {
        0..=2 => "small",
        3..=5 => "medium",
        _ => "large",
    }
}

/// Usable (candidate, method index) pairs, shuffled by `seed` and then taken
/// round-robin across the small/medium/large implementation-count bands.
fn select(cands: &[Cand], seed: u64) -> Vec<(Cand, usize)> {
    let mut usable: Vec<(Cand, usize)> = cands
        .iter()
        .filter(|c| !c.impls().is_empty() && c.impls().len() <= MAX_IMPLS)
        .filter_map(|c| {
            // The method with the most call sites that has no context yet.
            let mi = c
                .methods()
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    !m.ctx_first && m.call_sites > 0 && !STDLIB_METHODS.contains(&m.name.as_str())
                })
                .max_by_key(|(i, m)| (m.call_sites, usize::MAX - i))
                .map(|(i, _)| i)?;
            Some((c.clone(), mi))
        })
        .collect();
    usable.sort_by(|a, b| (&a.0.pkg, &a.0.iface).cmp(&(&b.0.pkg, &b.0.iface)));
    let mut rng = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        | 1;
    for i in (1..usable.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        usable.swap(i, (rng % (i as u64 + 1)) as usize);
    }
    let mut bands: [Vec<(Cand, usize)>; 3] = Default::default();
    for u in usable {
        let b = match band(u.0.impls().len()) {
            "small" => 0,
            "medium" => 1,
            _ => 2,
        };
        bands[b].push(u);
    }
    let mut out = Vec::new();
    let mut idx = [0usize; 3];
    loop {
        let mut any = false;
        for b in 0..3 {
            if let Some(u) = bands[b].get(idx[b]) {
                out.push(u.clone());
                idx[b] += 1;
                any = true;
            }
        }
        if !any {
            return out;
        }
    }
}

fn as_pkg_dir(d: &str) -> String {
    if d.is_empty() || d == "." {
        ".".into()
    } else {
        format!("./{}", d.trim_start_matches("./"))
    }
}

fn vet_dirs(c: &Cand) -> Vec<String> {
    let mut s: BTreeSet<String> = BTreeSet::new();
    s.insert(as_pkg_dir(&c.dir));
    for i in c.impls() {
        s.insert(as_pkg_dir(&i.dir));
    }
    for d in c.caller_dirs.as_deref().unwrap_or(&[]) {
        s.insert(as_pkg_dir(d));
    }
    s.into_iter().collect()
}

fn question(iface: &str, method: &str, file: &str, pkg: &str) -> String {
    format!(
        "Add a new first parameter `ctx context.Context` to the `{method}` method of the `{iface}` interface \
         declared in {file} (package {pkg}). Update the interface, every implementation of it in this repository \
         (including test fakes and mocks), and every call site, so that the repository builds and its tests compile. \
         Call sites that already have a context.Context in scope should pass it; otherwise pass context.TODO()."
    )
}

/// One line for the plan listing printed before the runs.
pub fn plan_line(t: &Task) -> String {
    match &t.edit {
        Some(e) => format!(
            "  {:<6} edit {:<39} {:>4} impls ({} test), {} call sites{}",
            t.band,
            t.target,
            e.impls,
            e.test_impls,
            e.call_sites,
            if e.unasserted.is_empty() {
                String::new()
            } else {
                format!(", {} not assertable", e.unasserted.len())
            }
        ),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

const EDIT_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "Edit",
    "Write",
    "Bash(go:*)",
    "Bash(gofmt:*)",
    "Bash(sed:*)",
    "Bash(perl:*)",
    "Bash(grep:*)",
    "Bash(rg:*)",
    "Bash(find:*)",
    "Bash(ls:*)",
    "Bash(cat:*)",
    "Bash(head:*)",
    "Bash(tail:*)",
    "Bash(wc:*)",
];

/// Whether an arm applies to edit tasks (see the module docs).
pub fn arm_applies(arm: &str) -> bool {
    arm == "plain"
}

/// Run one edit task in a fresh copy of `root` and grade it.
pub fn run_task(
    t: &Task,
    arm: &str,
    args: &AuditArgs,
    root: &Path,
    out: &Path,
) -> Result<RunResult> {
    let spec = t.edit.as_ref().context("edit task without an edit spec")?;
    let ws = work_dir(&format!("{}-{arm}", t.id));
    copy_repo(root, &ws, out)?;
    let r = run_in(t, spec, arm, args, &ws, root, out);
    let _ = std::fs::remove_dir_all(&ws);
    r
}

fn run_in(
    t: &Task,
    spec: &EditSpec,
    arm: &str,
    args: &AuditArgs,
    ws: &Path,
    root: &Path,
    out: &Path,
) -> Result<RunResult> {
    let prompt = format!(
        "You are working in the repository in the current directory.\n\n{}\n\nWhen you are done, reply with a short \
         summary of what you changed. If the build or vet still fails, or you could not finish, say so.",
        t.question
    );
    let mut cmd = Command::new("claude");
    cmd.current_dir(ws)
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
        .args(["--tools", "Read,Grep,Glob,Bash,Edit,Write"])
        .args(["--max-budget-usd", &format!("{:.2}", args.max_usd_per_task)]);
    if let Some(m) = &args.model {
        cmd.args(["--model", m]);
    }
    cmd.arg("--allowedTools").args(EDIT_TOOLS);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let mut child = cmd
        .spawn()
        .context("start `claude` (is Claude Code installed and logged in?)")?;
    let mut stdout = child.stdout.take().context("agent stdout")?;
    let mut stderr = child.stderr.take().context("agent stderr")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
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
    let raw_err = err_reader.join().unwrap_or_default();
    let wall_s = start.elapsed().as_secs_f64();
    std::fs::write(out.join(format!("{}-{arm}.json", t.id)), &raw)?;

    let mut result = RunResult {
        task: t.id.clone(),
        arm: arm.into(),
        model: String::new(),
        rep: 1,
        wall_s,
        cost_usd: None,
        turns: None,
        tokens: None,
        score: Score::default(),
    };
    let mut text = String::new();
    match serde_json::from_str::<Value>(raw.trim()) {
        Ok(v) => {
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
            text = v["result"].as_str().unwrap_or("").to_string();
        }
        Err(_) if timed_out => {
            // Grade what the agent left behind; a timeout is not a pass.
            result.score.error = Some("timed out".into());
        }
        Err(_) => {
            if !raw_err.trim().is_empty() {
                std::fs::write(out.join(format!("{}-{arm}.stderr", t.id)), &raw_err)?;
            }
            result.score.error = Some("agent produced no result".into());
            return Ok(result);
        }
    }

    let files_changed = save_diff(root, ws, &out.join(format!("{}-{arm}.diff", t.id)));
    let mut edit = grade_copy(ws, spec);
    edit.files_changed = files_changed;
    result.score.flagged_incomplete = admits_failure(&text);
    result.score.pass = result.score.error.is_none() && edit_pass(&edit);
    result.score.recall = if result.score.pass { 1.0 } else { 0.0 };
    result.score.precision = result.score.recall;
    result.score.n_answer = files_changed;
    result.score.missed = edit.assert_errors.clone();
    result.score.extra = edit.errors.iter().take(10).cloned().collect();
    result.score.edit = Some(edit);
    Ok(result)
}

pub fn edit_pass(e: &EditScore) -> bool {
    e.build_ok && e.vet_ok && e.signature_changed
}

/// Write the hidden assertions into an (edited) copy, then build, vet and
/// check the signature.
pub fn grade_copy(ws: &Path, spec: &EditSpec) -> EditScore {
    let _ = write_asserts(ws, &spec.assert_files);
    let build = go_build(ws);
    let vet = go_vet(ws, &spec.vet_dirs);
    let sig = std::fs::read_to_string(ws.join(&spec.iface_file))
        .map(|src| signature_changed(&src, &spec.iface, &spec.method))
        .unwrap_or(false);
    let mut errors = go_errors(&build.output);
    for e in go_errors(&vet.output) {
        if !errors.contains(&e) {
            errors.push(e);
        }
    }
    let assert_errors: Vec<String> = errors
        .iter()
        .filter(|e| e.contains("zz_ifacegen_assert"))
        .take(12)
        .cloned()
        .collect();
    EditScore {
        build_ok: build.ok,
        vet_ok: vet.ok,
        signature_changed: sig,
        files_changed: 0,
        n_errors: errors.len(),
        errors: errors.into_iter().take(30).collect(),
        assert_errors,
    }
}

/// Changed files between the original and the edited copy, written as a
/// unified diff for review. Best effort; returns the number of changed files.
fn save_diff(root: &Path, ws: &Path, dest: &Path) -> usize {
    let o = Command::new("diff")
        .args(["-ruN", "-x", ".git", "-x", ".gridseak"])
        .arg(root)
        .arg(ws)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(o) = o else { return 0 };
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let _ = std::fs::write(dest, &text);
    text.lines().filter(|l| l.starts_with("diff -ruN")).count()
}

/// Whether the agent's reply admits it did not finish or the build fails.
fn admits_failure(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "incomplete",
        "not complete",
        "could not finish",
        "couldn't finish",
        "unable to complete",
        "still fail",
        "does not build",
        "doesn't build",
        "does not compile",
        "doesn't compile",
        "not able to",
    ]
    .iter()
    .any(|p| t.contains(p))
}

// ---------------------------------------------------------------------------
// Grading helpers (pure)
// ---------------------------------------------------------------------------

/// Whether `iface`'s `method` in `src` now takes a `context.Context` first.
/// Looks only inside the `type <iface> interface { … }` block.
pub fn signature_changed(src: &str, iface: &str, method: &str) -> bool {
    let mut inside = false;
    let mut depth = 0i32;
    for line in src.lines() {
        let l = line.trim();
        if !inside {
            let decl = l.strip_prefix("type ").unwrap_or(l).trim_start();
            if let Some(rest) = decl.strip_prefix(iface) {
                let rest = rest.trim_start();
                if rest.starts_with("interface") && rest.contains('{') {
                    inside = true;
                    depth = brace_delta(rest);
                    if depth <= 0 {
                        // One-line interface: `type I interface { M(ctx context.Context) }`.
                        let body = rest.split_once('{').map(|x| x.1).unwrap_or("");
                        return method_line_has_ctx(body.trim(), method);
                    }
                }
            }
            continue;
        }
        if depth == 1 && method_line_has_ctx(l, method) {
            return true;
        }
        depth += brace_delta(l);
        if depth <= 0 {
            return false;
        }
    }
    false
}

fn brace_delta(s: &str) -> i32 {
    s.chars().fold(0, |d, c| match c {
        '{' => d + 1,
        '}' => d - 1,
        _ => d,
    })
}

fn method_line_has_ctx(line: &str, method: &str) -> bool {
    let Some(rest) = line.strip_prefix(method) else {
        return false;
    };
    let Some(params) = rest.trim_start().strip_prefix('(') else {
        return false;
    };
    let first = params.split([',', ')']).next().unwrap_or("").trim();
    // `ctx context.Context`, `_ context.Context` or an unnamed `context.Context`.
    let ty = first.rsplit(' ').next().unwrap_or("");
    ty == "context.Context"
}

/// Compiler/vet diagnostics (`path.go:line:col: msg`) from tool output.
pub fn go_errors(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in output.lines() {
        let l = l.trim();
        let Some(i) = l.find(".go:") else { continue };
        let rest = &l[i + 4..];
        let mut parts = rest.splitn(3, ':');
        let line_ok = parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let col_ok = parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        if line_ok && col_ok && !out.iter().any(|x: &String| x == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// Report cell for an edit run, e.g. `build ✓ vet ✓ signature ✓`.
pub fn detail(s: &Score) -> Option<String> {
    let e = s.edit.as_ref()?;
    let m = |b: bool| if b { "✓" } else { "✗" };
    Some(format!(
        "build {} vet {} signature {}",
        m(e.build_ok),
        m(e.vet_ok),
        m(e.signature_changed)
    ))
}

/// Size columns for an edit task row: (implementations, call sites).
pub fn size_cells(t: &Task) -> Option<(String, String)> {
    let e = t.edit.as_ref()?;
    Some((
        format!("{} impls", e.impls),
        format!("{} call sites", e.call_sites),
    ))
}

/// Adjust the run's mode and answer-key labels when it includes edit tasks.
pub fn annotate(r: &mut AuditResults, has_callers: bool) {
    if !r.tasks.iter().any(|t| t.kind == KIND) {
        return;
    }
    const EDIT_KEY: &str =
        "Go compiler (`go build` + `go vet` with hidden implementation assertions)";
    const EDIT_MODE: &str = "edits: write tools in a throwaway copy";
    if has_callers {
        r.answer_key = format!("{}; edit tasks: {EDIT_KEY}", r.answer_key);
        r.mode = format!("{}; {EDIT_MODE}", r.mode);
    } else {
        r.answer_key = EDIT_KEY.into();
        r.mode = EDIT_MODE.into();
    }
}

/// Extra report prose when the results contain edit tasks.
pub fn report_note(r: &AuditResults) -> Option<String> {
    let n = r.tasks.iter().filter(|t| t.kind == KIND).count();
    if n == 0 {
        return None;
    }
    Some(format!(
        "{n} edit task(s): add `ctx context.Context` as the first parameter of an interface method and update every \
         implementation, test fake and caller. Each run works in a throwaway copy of the repository; it passes when \
         `go build ./...` and `go vet` on the affected packages are clean with hidden assertions that every original \
         implementation still satisfies the interface, and the method's signature actually changed. Edit tasks run \
         without GridSeak only: its tools answer from the scan of the original path, and the agent's copy has none."
    ))
}

// ---------------------------------------------------------------------------
// Files and processes
// ---------------------------------------------------------------------------

struct Captured {
    ok: bool,
    stdout: String,
    output: String,
}

fn run_capture(cmd: &mut Command, cwd: &Path) -> Result<Captured> {
    let o = cmd
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("run {cmd:?}"))?;
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    Ok(Captured {
        ok: o.status.success(),
        output: format!("{stdout}{stderr}"),
        stdout,
    })
}

fn go_build(ws: &Path) -> Captured {
    run_capture(
        Command::new("go").args(["build", "-o", "/dev/null", "./..."]),
        ws,
    )
    .unwrap_or(Captured {
        ok: false,
        stdout: String::new(),
        output: "could not run go build".into(),
    })
}

fn go_vet(ws: &Path, dirs: &[String]) -> Captured {
    run_capture(Command::new("go").arg("vet").args(dirs), ws).unwrap_or(Captured {
        ok: false,
        stdout: String::new(),
        output: "could not run go vet".into(),
    })
}

fn write_asserts(ws: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    for (rel, src) in files {
        let p = ws.join(rel);
        // A package the agent deleted gets no assertion: the build/vet of
        // its callers will show what broke.
        if p.parent().is_some_and(|d| d.is_dir()) {
            std::fs::write(&p, src).with_context(|| format!("write {}", p.display()))?;
        }
    }
    Ok(())
}

fn remove_asserts(ws: &Path, files: &BTreeMap<String, String>) {
    for rel in files.keys() {
        let _ = std::fs::remove_file(ws.join(rel));
    }
}

fn tail(s: &str, n: usize) -> &str {
    let mut i = s.len().saturating_sub(n);
    while !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}

/// A fresh, empty path under the system temp dir for one copy.
fn work_dir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "gridseak-audit-{}-{}-{tag}",
        std::process::id(),
        chrono::Local::now().format("%H%M%S")
    ));
    let _ = std::fs::remove_dir_all(&p);
    p
}

/// Top-level entries of `root` to copy: everything except GridSeak's own
/// `.gridseak` directory and whichever entry holds the audit output.
fn entries_to_copy(root: &Path, out: &Path) -> Result<Vec<PathBuf>> {
    let skip_out = out
        .strip_prefix(root)
        .ok()
        .and_then(|r| r.components().next())
        .map(|c| c.as_os_str().to_os_string());
    let mut v = Vec::new();
    for e in std::fs::read_dir(root)? {
        let e = e?;
        let name = e.file_name();
        if name == ".gridseak" || Some(&name) == skip_out.as_ref() {
            continue;
        }
        v.push(e.path());
    }
    v.sort();
    Ok(v)
}

/// Copy the repository at `root` to `dst` keeping mtimes (GridSeak's
/// staleness check is mtime-based). macOS: `cp -cpR` (copy-on-write clones);
/// elsewhere, or if cloning fails, a plain recursive copy.
pub fn copy_repo(root: &Path, dst: &Path, out: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    let entries = entries_to_copy(root, out)?;
    if entries.is_empty() {
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        let ok = Command::new("cp")
            .arg("-cpR")
            .args(&entries)
            .arg(dst)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
        std::fs::remove_dir_all(dst)?;
        std::fs::create_dir_all(dst)?;
    }
    for e in &entries {
        let name = e.file_name().context("entry name")?;
        copy_tree(e, &dst.join(name))?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        // `join("")` would append a trailing separator, which Linux rejects
        // for a file ("Is a directory") when `src` is itself a file.
        let target = if rel.as_os_str().is_empty() {
            dst.to_path_buf()
        } else {
            dst.join(rel)
        };
        let ft = entry.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if ft.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &target)?;
            #[cfg(not(unix))]
            std::fs::copy(entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
            if let Some(m) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
                if let Ok(f) = std::fs::File::options().write(true).open(&target) {
                    let _ = f.set_modified(m);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_check_reads_only_the_interface_block() {
        let before = "package p\n\ntype Adapter interface {\n\tAdapt(body []byte, options map[string]any) ([]byte, []Warning, error)\n}\n\ntype Other interface {\n\tAdapt(ctx context.Context) error\n}\n";
        assert!(!signature_changed(before, "Adapter", "Adapt"));
        assert!(signature_changed(before, "Other", "Adapt"));
        let after = before.replace("Adapt(body", "Adapt(ctx context.Context, body");
        assert!(signature_changed(&after, "Adapter", "Adapt"));
        let unnamed = before.replace("Adapt(body []byte", "Adapt(context.Context, []byte");
        assert!(signature_changed(&unnamed, "Adapter", "Adapt"));
    }

    #[test]
    fn signature_check_handles_grouped_and_one_line_decls() {
        let grouped = "type (\n\tSelector interface {\n\t\t// Select picks one.\n\t\tSelect(ctx context.Context, p UpstreamPool) *Upstream\n\t}\n)\n";
        assert!(signature_changed(grouped, "Selector", "Select"));
        assert!(!signature_changed(grouped, "Selector", "Pick"));
        assert!(signature_changed(
            "type I interface { M(ctx context.Context) }",
            "I",
            "M"
        ));
        assert!(!signature_changed(
            "type I interface { M(x int) }",
            "I",
            "M"
        ));
        // A method of the same name on a different, prefixed interface is not it.
        assert!(!signature_changed(
            "type IX interface {\n M(ctx context.Context)\n}\ntype I interface {\n M()\n}\n",
            "I",
            "M"
        ));
    }

    #[test]
    fn go_errors_keeps_only_positioned_diagnostics() {
        let out = "# example.com/p\n./a.go:12:5: cannot use x (variable of type T) as I value\nvet: p/zz_ifacegen_assert_test.go:3:9: missing method\nnote: module requires Go 1.22\n./a.go:12:5: cannot use x (variable of type T) as I value\n";
        let e = go_errors(out);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[1].contains("zz_ifacegen_assert"));
    }

    fn cand(iface: &str, impls: usize, methods: &[(&str, bool, usize)]) -> Cand {
        Cand {
            pkg: "example.com/p".into(),
            dir: "p".into(),
            file: "p/p.go".into(),
            line: 3,
            iface: iface.into(),
            methods: Some(
                methods
                    .iter()
                    .map(|(n, c, s)| Method {
                        name: (*n).into(),
                        ctx_first: *c,
                        call_sites: *s,
                    })
                    .collect(),
            ),
            impls: Some(
                (0..impls)
                    .map(|i| Impl {
                        dir: format!("impl{i}"),
                        test: i == 0,
                    })
                    .collect(),
            ),
            caller_dirs: Some(vec!["cmd".into(), "".into()]),
        }
    }

    #[test]
    fn selection_skips_unusable_interfaces_and_is_seeded() {
        let cands = vec![
            cand("NoImpls", 0, &[("Do", false, 3)]),
            cand("TooMany", MAX_IMPLS + 1, &[("Do", false, 3)]),
            cand("HasCtx", 2, &[("Do", true, 3)]),
            cand("Uncalled", 2, &[("Do", false, 0)]),
            cand("Stringer", 2, &[("String", false, 9)]),
            cand("Small", 1, &[("A", false, 1), ("B", false, 4)]),
            cand("Medium", 4, &[("Run", false, 2)]),
            cand("Large", 8, &[("Get", false, 2)]),
        ];
        let picks = select(&cands, 1);
        let names: Vec<&str> = picks.iter().map(|(c, _)| c.iface.as_str()).collect();
        assert_eq!(names, ["Small", "Medium", "Large"]);
        // The most-called eligible method is chosen.
        assert_eq!(picks[0].0.methods()[picks[0].1].name, "B");
        assert_eq!(select(&cands, 7).len(), 3);
    }

    #[test]
    fn vet_dirs_cover_interface_impls_and_callers() {
        let d = vet_dirs(&cand("I", 2, &[("M", false, 1)]));
        assert_eq!(d, [".", "./cmd", "./impl0", "./impl1", "./p"]);
    }

    #[test]
    fn copy_skips_gridseak_dir_and_audit_output() {
        let src = tempfile::tempdir().unwrap();
        let r = src.path();
        std::fs::create_dir_all(r.join("pkg")).unwrap();
        std::fs::write(r.join("go.mod"), "module x\n").unwrap();
        std::fs::write(r.join("pkg/a.go"), "package pkg\n").unwrap();
        std::fs::create_dir_all(r.join(".gridseak/audit/run")).unwrap();
        std::fs::create_dir_all(r.join("reports/run")).unwrap();
        let dst = tempfile::tempdir().unwrap();
        let d = dst.path().join("copy");
        copy_repo(r, &d, &r.join("reports/run")).unwrap();
        assert!(d.join("pkg/a.go").is_file() && d.join("go.mod").is_file());
        assert!(!d.join(".gridseak").exists() && !d.join("reports").exists());
        let m = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(m(&r.join("pkg/a.go")), m(&d.join("pkg/a.go")));
    }

    #[test]
    fn copy_tree_copies_a_single_file() {
        let src = tempfile::tempdir().unwrap();
        let f = src.path().join("go.mod");
        std::fs::write(&f, "module x\n").unwrap();
        let dst = tempfile::tempdir().unwrap();
        let t = dst.path().join("go.mod");
        copy_tree(&f, &t).unwrap();
        assert_eq!(std::fs::read_to_string(&t).unwrap(), "module x\n");
    }

    #[test]
    fn copy_tree_fallback_keeps_mtimes() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("a/b")).unwrap();
        let f = src.path().join("a/b/c.go");
        std::fs::write(&f, "package b\n").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let dst = tempfile::tempdir().unwrap();
        copy_tree(&src.path().join("a"), &dst.path().join("a")).unwrap();
        let got = std::fs::metadata(dst.path().join("a/b/c.go"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(got, old);
    }

    #[test]
    fn edit_detail_and_pass() {
        let mut s = Score::default();
        assert!(detail(&s).is_none());
        let e = EditScore {
            build_ok: true,
            vet_ok: false,
            signature_changed: true,
            ..Default::default()
        };
        assert!(!edit_pass(&e));
        s.edit = Some(e);
        assert_eq!(detail(&s).unwrap(), "build ✓ vet ✗ signature ✓");
    }

    #[test]
    fn non_go_repository_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let err = require_go_module(d.path()).unwrap_err().to_string();
        assert!(err.contains("Go modules only"), "{err}");
    }

    #[test]
    fn failure_admissions_are_detected() {
        assert!(admits_failure("The build still fails in pkg/x."));
        assert!(!admits_failure("Done. go build ./... and go vet pass."));
    }

    #[test]
    fn assert_output_parses_with_and_without_skips() {
        let a: Asserts = serde_json::from_str(
            r#"{"files":{"p/zz_ifacegen_assert_test.go":"package p\n"},"skipped":null}"#,
        )
        .unwrap();
        assert_eq!(a.files.len(), 1);
        assert!(a.skipped.unwrap_or_default().is_empty());
    }

    const MOD_FILES: &[(&str, &str)] = &[
        ("go.mod", "module example.com/m\n\ngo 1.22\n"),
        (
            "p/p.go",
            "package p\n\ntype Store interface {\n\tGet(key string) string\n}\n\nfunc Use(s Store) string { return s.Get(\"k\") }\n",
        ),
        (
            "impl/impl.go",
            "package impl\n\ntype Mem struct{}\n\nfunc (Mem) Get(key string) string { return key }\n",
        ),
        (
            "impl/fake_test.go",
            "package impl\n\ntype fake struct{}\n\nfunc (*fake) Get(key string) string { return \"\" }\n",
        ),
        (
            "cmd/m/main.go",
            "package main\n\nimport (\n\t\"fmt\"\n\n\t\"example.com/m/impl\"\n\t\"example.com/m/p\"\n)\n\nfunc main() { fmt.Println(p.Use(impl.Mem{})) }\n",
        ),
    ];

    fn edit(dir: &Path, file: &str, from: &str, to: &str) {
        let p = dir.join(file);
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains(from), "{file} lacks {from}");
        std::fs::write(&p, s.replace(from, to)).unwrap();
    }

    /// End to end on a tiny module: plan one task, then grade an unmodified
    /// copy, an interface-only change, a change that misses the test fake,
    /// and a complete change.
    #[test]
    #[ignore = "needs the Go toolchain and golang.org/x/tools (module proxy)"]
    fn hidden_assertions_catch_broken_changes() {
        let repo = tempfile::tempdir().unwrap();
        for (f, s) in MOD_FILES {
            let p = repo.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        }
        let out = tempfile::tempdir().unwrap();
        let tasks = plan(repo.path(), 1, 1, out.path()).unwrap();
        assert_eq!(tasks.len(), 1);
        let spec = tasks[0].edit.clone().unwrap();
        assert_eq!(
            (spec.iface.as_str(), spec.method.as_str()),
            ("Store", "Get")
        );
        assert_eq!((spec.impls, spec.test_impls), (2, 1));

        let grade = |prep: &dyn Fn(&Path)| {
            let ws = tempfile::tempdir().unwrap();
            let ws = ws.path().join("ws");
            copy_repo(repo.path(), &ws, out.path()).unwrap();
            prep(&ws);
            grade_copy(&ws, &spec)
        };
        let iface = |ws: &Path| {
            edit(
                ws,
                "p/p.go",
                "package p\n",
                "package p\n\nimport \"context\"\n",
            );
            edit(
                ws,
                "p/p.go",
                "Get(key string)",
                "Get(ctx context.Context, key string)",
            );
            edit(ws, "p/p.go", "s.Get(\"k\")", "s.Get(context.TODO(), \"k\")");
        };
        let mem = |ws: &Path| {
            edit(
                ws,
                "impl/impl.go",
                "package impl\n",
                "package impl\n\nimport \"context\"\n",
            );
            edit(
                ws,
                "impl/impl.go",
                "Get(key string)",
                "Get(ctx context.Context, key string)",
            );
        };
        let fake = |ws: &Path| {
            edit(
                ws,
                "impl/fake_test.go",
                "package impl\n",
                "package impl\n\nimport \"context\"\n",
            );
            edit(
                ws,
                "impl/fake_test.go",
                "Get(key string)",
                "Get(ctx context.Context, key string)",
            );
        };

        let unmodified = grade(&|_| {});
        assert!(unmodified.build_ok && unmodified.vet_ok && !unmodified.signature_changed);
        assert!(!edit_pass(&unmodified));

        let only_iface = grade(&|ws| iface(ws));
        assert!(
            only_iface.signature_changed && !only_iface.build_ok,
            "{only_iface:?}"
        );

        let missed_fake = grade(&|ws| {
            iface(ws);
            mem(ws);
        });
        assert!(
            missed_fake.build_ok && !missed_fake.vet_ok,
            "{missed_fake:?}"
        );
        assert!(!missed_fake.assert_errors.is_empty(), "{missed_fake:?}");

        let complete = grade(&|ws| {
            iface(ws);
            mem(ws);
            fake(ws);
        });
        assert!(edit_pass(&complete), "{complete:?}");
    }
}
