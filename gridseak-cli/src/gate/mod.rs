//! `gridseak gate` — unskippable structural verdict at the edit boundary.
//!
//! Hosts (Claude `PreToolUse`, Cursor `preToolUse` + `beforeShellExecution`)
//! pipe a pending edit. The gate parses removed/renamed function symbols,
//! runs `no_callers` against the latest scan artifact, and returns
//! `deny` / `ask` / `allow` with a named tier. Compaction cannot
//! invent this register.

pub mod artifact;
pub mod decision;
pub mod fixtures;
pub mod host_io;
pub mod ledger;
pub mod parse_edit;
pub mod preferred;
pub mod ts_names;

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use gridseak_local_store::ProjectStore;
use serde::Serialize;

use crate::gate::decision::{decide, Decision, Permission};
use crate::gate::host_io::{HostFormat, HostPayload};
use crate::gate::ledger::append_ledger;
use crate::gate::parse_edit::PendingEdit;

#[derive(Args, Debug, Clone)]
pub struct GateArgs {
    /// Output shape: `claude` (canonical PreToolUse JSON), `cursor`
    /// (`permission` + `user_message`), or `json` (internal decision).
    #[arg(long, default_value = "json")]
    pub format: String,

    /// Latest scan sqlite. Overrides project-store lookup.
    #[arg(long)]
    pub artifact: Option<PathBuf>,

    /// Scan id recorded in the JSONL ledger (informational).
    #[arg(long)]
    pub scan_id: Option<String>,

    /// Project ref for store lookup when `--artifact` is omitted.
    #[arg(long, default_value = ".")]
    pub project: String,

    /// File path of the pending edit (fixtures / `--old --new`).
    #[arg(long)]
    pub file: Option<String>,

    /// Previous file contents (pair with `--new`).
    #[arg(long)]
    pub old: Option<PathBuf>,

    /// New file contents (pair with `--old`).
    #[arg(long)]
    pub new: Option<PathBuf>,

    /// Unified diff path (`-` = stdin).
    #[arg(long)]
    pub diff: Option<String>,

    /// Direct FQN / short name (fixture shortcut). Repeatable.
    #[arg(long)]
    pub symbol: Vec<String>,

    /// Treat the scan as stale (fixture: force `ask` when the file is dirty).
    #[arg(long, default_value_t = false)]
    pub stale: bool,

    /// Dirty paths that intersect the edit (workspace_delta). Repeatable.
    #[arg(long)]
    pub dirty: Vec<String>,

    /// Print the 10-line compaction card and exit.
    #[arg(long, default_value_t = false)]
    pub card: bool,

    /// JSONL ledger path (default: `~/.gridseak/gate.jsonl`).
    #[arg(long)]
    pub ledger: Option<PathBuf>,

    /// Skip ledger append (tests).
    #[arg(long, default_value_t = false)]
    pub no_ledger: bool,

    /// Backup: deny → process exit 2 + stderr reason. Default stays
    /// Claude-canonical (exit 0 + JSON verdict).
    #[arg(long, default_value_t = false)]
    pub strict_exit: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateRunResult {
    pub decision: Decision,
    pub host_stdout: String,
    pub exit_code: i32,
}

pub fn run(store: &ProjectStore, args: GateArgs) -> Result<i32> {
    if args.card {
        print!(
            "{}",
            compaction_card(store, &args.project, args.ledger.as_ref())?
        );
        return Ok(0);
    }

    let result = evaluate(store, args, None)?;
    print!("{}", result.host_stdout);
    Ok(result.exit_code)
}

/// Evaluate without printing. `stdin_override` is for fixtures.
pub fn evaluate(
    store: &ProjectStore,
    args: GateArgs,
    stdin_override: Option<&str>,
) -> Result<GateRunResult> {
    let format = HostFormat::parse(&args.format);
    let started = std::time::Instant::now();
    let edit = resolve_edit(&args, stdin_override, format)?;
    let needs_scan = edit.needs_scan() || !args.symbol.is_empty();

    // Host hooks are fail-closed on nonzero / non-JSON. A `pwd` or
    // `cargo test` must not look up a project — Cursor's hook cwd is
    // often not a registered root, and that used to crash the hook
    // and block every shell command.
    if !needs_scan {
        return finalize(
            &args,
            format,
            started,
            decision::allow_unscanned(
                "no function/method symbols removed or renamed",
                &edit.file,
                &edit.extract_source,
            ),
        );
    }

    let (artifact, scan_id) = match resolve_artifact(store, &args, &edit) {
        Ok(pair) => pair,
        Err(err) => {
            return finalize(
                &args,
                format,
                started,
                decision::ask_unresolved(&format!("{err}"), &edit.file, &edit.extract_source),
            );
        }
    };

    let request = decision::Request {
        edit,
        artifact,
        scan_id: scan_id.clone(),
        stale: args.stale,
        dirty_paths: args.dirty.clone(),
        extra_symbols: args.symbol.clone(),
    };
    let mut decision = decide(&request)?;
    if decision.file.is_empty() {
        decision.file = request.edit.file.clone();
    }
    if decision.extract_source.is_empty() {
        decision.extract_source = request.edit.extract_source.clone();
    }
    finalize(&args, format, started, decision)
}

fn finalize(
    args: &GateArgs,
    format: HostFormat,
    started: std::time::Instant,
    mut decision: Decision,
) -> Result<GateRunResult> {
    decision.duration_ms = started.elapsed().as_millis() as u64;
    decision.host = match format {
        HostFormat::Claude => "claude",
        HostFormat::Cursor => "cursor",
        HostFormat::CursorTool => "cursor-tool",
        HostFormat::Json => "json",
    }
    .into();

    if !args.no_ledger {
        let ledger_path = args.ledger.clone().unwrap_or_else(default_ledger_path);
        append_ledger(&ledger_path, &decision)?;
    }

    let host_stdout = host_io::render(&format, &decision);
    // Default: Claude-canonical exit 0 + stdout JSON. `--strict-exit`
    // is the documented backup: deny → exit 2 + stderr reason.
    let exit_code = if args.strict_exit && decision.permission == Permission::Deny {
        let _ = write_stderr_backup(&decision);
        2
    } else {
        0
    };
    Ok(GateRunResult {
        decision,
        host_stdout,
        exit_code,
    })
}

pub fn compaction_card(
    store: &ProjectStore,
    project: &str,
    ledger: Option<&PathBuf>,
) -> Result<String> {
    let scan_prefix = store
        .resolve_project(project)
        .ok()
        .and_then(|p| p.latest_scan.map(|s| s.id))
        .unwrap_or_else(|| "none".into());
    let short: String = scan_prefix.chars().take(8).collect();
    let ledger_path = ledger.cloned().unwrap_or_else(default_ledger_path);
    let open = ledger::recent_denials(&ledger_path, 5);
    let mut lines = vec![
        format!("GridSeak gate card  scan={short}…"),
        "Never flatten tiers. The gate is the authority, not the rule.".into(),
        "Re-query: gridseak gate --card".into(),
        format!("Ledger: {}", ledger_path.display()),
    ];
    if open.is_empty() {
        lines.push("Open denials: none".into());
    } else {
        lines.push(format!("Open denials ({})", open.len()));
        for row in open {
            lines.push(format!("  - {row}"));
        }
    }
    lines.push("Compiler/LSP callers → deny. Tree-sitter only → ask.".into());
    lines.push("Dirty path ∩ symbol file + stale scan → ask (rescan).".into());
    lines.push("Twin edit under deno/ or _examples/ → deny + canonical file.".into());
    lines.push("Evidence: deterministic_local_analysis.".into());
    Ok(lines.join("\n") + "\n")
}

fn resolve_edit(
    args: &GateArgs,
    stdin_override: Option<&str>,
    format: HostFormat,
) -> Result<PendingEdit> {
    if let (Some(old), Some(new)) = (&args.old, &args.new) {
        let old_text = std::fs::read_to_string(old)
            .with_context(|| format!("read --old {}", old.display()))?;
        let new_text = std::fs::read_to_string(new)
            .with_context(|| format!("read --new {}", new.display()))?;
        let file = args
            .file
            .clone()
            .unwrap_or_else(|| old.display().to_string());
        return Ok(parse_edit::from_old_new(&file, &old_text, &new_text));
    }

    if let Some(diff_spec) = &args.diff {
        let text = if diff_spec == "-" {
            read_stdin(stdin_override, format)?
        } else {
            std::fs::read_to_string(diff_spec)
                .with_context(|| format!("read --diff {diff_spec}"))?
        };
        return parse_edit::from_unified_diff(&text)
            .into_iter()
            .next()
            .context("diff contained no file hunks");
    }

    if !args.symbol.is_empty() {
        return Ok(PendingEdit {
            file: args.file.clone().unwrap_or_default(),
            old_text: String::new(),
            new_text: String::new(),
            deleted_ranges: Vec::new(),
            removed_names: args.symbol.clone(),
            whole_file_delete: false,
            extract_source: "cli-symbol".into(),
            cwd: None,
        });
    }

    let raw = read_stdin(stdin_override, format)?;
    if raw.trim().is_empty() {
        return Ok(PendingEdit {
            file: args.file.clone().unwrap_or_default(),
            old_text: String::new(),
            new_text: String::new(),
            deleted_ranges: Vec::new(),
            removed_names: Vec::new(),
            whole_file_delete: false,
            extract_source: String::new(),
            cwd: None,
        });
    }
    let payload: HostPayload = serde_json::from_str(&raw)
        .with_context(|| "stdin is not host JSON; pass --diff / --old --new / --symbol")?;
    Ok(parse_edit::from_host_payload(&payload))
}

fn resolve_artifact(
    store: &ProjectStore,
    args: &GateArgs,
    edit: &PendingEdit,
) -> Result<(PathBuf, String)> {
    if let Some(path) = &args.artifact {
        let id = args
            .scan_id
            .clone()
            .unwrap_or_else(|| "cli-artifact".into());
        return Ok((path.clone(), id));
    }
    let project = resolve_gate_project(store, args, edit)
        .context("no project; pass --artifact or run `gridseak scan .`")?;
    let scan = project
        .latest_scan
        .context("project has no scans — run `gridseak scan .` first")?;
    let artifact = scan
        .graph_artifact_path
        .clone()
        .context("latest scan has no graph artifact")?;
    Ok((PathBuf::from(artifact), scan.id))
}

fn resolve_gate_project(
    store: &ProjectStore,
    args: &GateArgs,
    edit: &PendingEdit,
) -> Result<gridseak_local_store::ProjectDto> {
    let reference = args.project.trim();
    let implicit = reference.is_empty() || reference == "." || reference == "./";

    // The file being deleted/edited wins over hook cwd. Cursor often
    // runs the hook with cwd = the agent workspace while `rm` targets
    // another registered root (the pinned chi G1 tree).
    if let Some(project) = project_for_edit_file(store, edit)? {
        return Ok(project);
    }
    if implicit {
        if let Ok(project) = store.resolve_project_lenient(reference) {
            return Ok(project);
        }
    } else if let Ok(project) = store.resolve_project(reference) {
        return Ok(project);
    }
    if implicit {
        store.resolve_project_lenient(reference)
    } else {
        store.resolve_project(reference)
    }
}

fn project_for_edit_file(
    store: &ProjectStore,
    edit: &PendingEdit,
) -> Result<Option<gridseak_local_store::ProjectDto>> {
    if edit.file.is_empty() {
        if let Some(cwd) = &edit.cwd {
            return project_covering_path(store, std::path::Path::new(cwd));
        }
        return Ok(None);
    }
    let file = std::path::Path::new(&edit.file);
    let joined = if file.is_absolute() {
        file.to_path_buf()
    } else if let Some(cwd) = &edit.cwd {
        std::path::Path::new(cwd).join(file)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(file)
    };
    project_covering_path(store, &joined)
}

fn project_covering_path(
    store: &ProjectStore,
    path: &std::path::Path,
) -> Result<Option<gridseak_local_store::ProjectDto>> {
    let mut cur = if path.is_file() {
        path.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    loop {
        if let Some(project) = store.project_for_root_path(&cur)? {
            return Ok(Some(project));
        }
        if !cur.pop() {
            return Ok(None);
        }
    }
}

fn read_stdin(stdin_override: Option<&str>, format: HostFormat) -> Result<String> {
    if let Some(s) = stdin_override {
        return Ok(s.to_string());
    }
    // `#[tokio::main]` plus IDE PTY wrappers make `IsTerminal` lie and
    // report a TTY on a piped hook payload. Host formats always read.
    if matches!(format, HostFormat::Json) && atty_stdin() {
        return Ok(String::new());
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn atty_stdin() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
}

pub fn default_ledger_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".gridseak").join("gate.jsonl")
}

pub fn write_stderr_backup(decision: &Decision) -> Result<()> {
    if decision.permission != Permission::Deny {
        return Ok(());
    }
    let mut err = std::io::stderr();
    writeln!(err, "{}", decision.reason)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::fixtures::write_fixture_db;
    use crate::gate::host_io::detect_host;
    use crate::gate::parse_edit::from_old_new;
    use gridseak_local_store::ProjectStore;
    use tempfile::TempDir;

    #[test]
    fn host_format_aliases() {
        assert!(matches!(HostFormat::parse("claude"), HostFormat::Claude));
        assert!(matches!(HostFormat::parse("cursor"), HostFormat::Cursor));
        assert!(matches!(
            HostFormat::parse("cursor-tool"),
            HostFormat::CursorTool
        ));
        assert!(matches!(detect_host("PreToolUse"), HostFormat::Claude));
    }

    fn empty_store(dir: &std::path::Path) -> ProjectStore {
        ProjectStore::open(
            dir.join("projects.sqlite"),
            dir.join("reports"),
            dir.join("graphs"),
        )
        .unwrap()
    }

    fn deny_args(dir: &std::path::Path, artifact: PathBuf, ledger: PathBuf) -> GateArgs {
        let old = dir.join("old.go");
        let new = dir.join("new.go");
        std::fs::write(
            &old,
            "package chi\n\nfunc NewRouter() *Mux {\n\treturn &Mux{}\n}\n",
        )
        .unwrap();
        std::fs::write(&new, "package chi\n").unwrap();
        GateArgs {
            format: "json".into(),
            artifact: Some(artifact),
            scan_id: Some("fixture".into()),
            project: ".".into(),
            file: Some("chi.go".into()),
            old: Some(old),
            new: Some(new),
            diff: None,
            symbol: vec![],
            stale: false,
            dirty: vec![],
            card: false,
            ledger: Some(ledger),
            no_ledger: false,
            strict_exit: true,
        }
    }

    #[test]
    fn strict_exit_deny_is_two() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("gate.sqlite");
        write_fixture_db(&artifact);
        let store = empty_store(dir.path());
        let ledger = dir.path().join("gate.jsonl");
        let args = deny_args(dir.path(), artifact, ledger.clone());
        let result = evaluate(&store, args, None).unwrap();
        assert_eq!(result.exit_code, 2, "{}", result.decision.reason);
        assert_eq!(result.decision.permission, Permission::Deny);
        assert_eq!(result.decision.host, "json");
        assert_eq!(result.decision.file, "chi.go");
        assert!(
            result.decision.duration_ms < 5_000,
            "duration_ms={}",
            result.decision.duration_ms
        );
        let row = std::fs::read_to_string(&ledger).unwrap();
        let v: serde_json::Value = serde_json::from_str(row.lines().next().unwrap()).unwrap();
        assert_eq!(v["host"], "json");
        assert_eq!(v["file"], "chi.go");
        assert!(v["duration_ms"].as_u64().is_some());
        assert_eq!(v["verdict"], "deny");
    }

    #[test]
    fn default_exit_stays_zero_on_deny() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("gate.sqlite");
        write_fixture_db(&artifact);
        let store = empty_store(dir.path());
        let mut args = deny_args(dir.path(), artifact, dir.path().join("gate.jsonl"));
        args.strict_exit = false;
        args.no_ledger = true;
        let result = evaluate(&store, args, None).unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.decision.permission, Permission::Deny);
    }

    #[test]
    fn rm_host_json_denies_via_evaluate() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("gate.sqlite");
        write_fixture_db(&artifact);
        let store = empty_store(dir.path());
        let args = GateArgs {
            format: "cursor".into(),
            artifact: Some(artifact),
            scan_id: Some("fixture".into()),
            project: ".".into(),
            file: None,
            old: None,
            new: None,
            diff: None,
            symbol: vec![],
            stale: false,
            dirty: vec![],
            card: false,
            ledger: Some(dir.path().join("gate.jsonl")),
            no_ledger: false,
            strict_exit: true,
        };
        let stdin = r#"{"command":"rm chi.go"}"#;
        let result = evaluate(&store, args, Some(stdin)).unwrap();
        assert_eq!(result.exit_code, 2, "{}", result.decision.reason);
        assert_eq!(result.decision.permission, Permission::Deny);
        assert_eq!(result.decision.host, "cursor");
        assert_eq!(result.decision.file, "chi.go");
        assert_eq!(result.decision.extract_source, "scan-range");
    }

    #[test]
    fn extract_source_labeled_on_tree_sitter_delete() {
        let edit = from_old_new(
            "chi.go",
            "func NewRouter() *Mux { return nil }\n",
            "package chi\n",
        );
        assert_eq!(edit.extract_source, "tree-sitter");
    }

    fn host_args(format: &str) -> GateArgs {
        GateArgs {
            format: format.into(),
            artifact: None,
            scan_id: None,
            project: ".".into(),
            file: None,
            old: None,
            new: None,
            diff: None,
            symbol: vec![],
            stale: false,
            dirty: vec![],
            card: false,
            ledger: None,
            no_ledger: true,
            strict_exit: false,
        }
    }

    #[test]
    fn cursor_pwd_allows_without_project() {
        let dir = TempDir::new().unwrap();
        let store = empty_store(dir.path());
        let result = evaluate(&store, host_args("cursor"), Some(r#"{"command":"pwd"}"#)).unwrap();
        assert_eq!(result.exit_code, 0, "{}", result.decision.reason);
        assert_eq!(result.decision.permission, Permission::Allow);
        let v: serde_json::Value = serde_json::from_str(result.host_stdout.trim()).unwrap();
        assert_eq!(v["permission"], "allow");
        println!("host gate allows unscanned shell");
    }

    #[test]
    fn cursor_rm_without_project_asks() {
        let dir = TempDir::new().unwrap();
        let store = empty_store(dir.path());
        let result = evaluate(
            &store,
            host_args("cursor"),
            Some(r#"{"command":"rm chi.go"}"#),
        )
        .unwrap();
        assert_eq!(result.exit_code, 0, "{}", result.decision.reason);
        assert_eq!(result.decision.permission, Permission::Ask);
        assert_eq!(result.decision.file, "chi.go");
        let v: serde_json::Value = serde_json::from_str(result.host_stdout.trim()).unwrap();
        assert_eq!(v["permission"], "ask");
        println!("host gate asks when scan is unresolved");
    }

    #[test]
    fn cursor_tool_write_delete_newrouter_denies() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("gate.sqlite");
        write_fixture_db(&artifact);
        let chi = dir.path().join("chi.go");
        std::fs::write(
            &chi,
            "package chi\n\nfunc NewRouter() *Mux {\n\treturn &Mux{}\n}\n",
        )
        .unwrap();
        let store = empty_store(dir.path());
        let mut args = host_args("cursor-tool");
        args.artifact = Some(artifact);
        args.scan_id = Some("fixture".into());
        args.ledger = Some(dir.path().join("gate.jsonl"));
        args.no_ledger = false;
        let stdin = format!(
            r#"{{"hook_event_name":"preToolUse","tool_name":"Write","tool_input":{{"file_path":"{}","content":"package chi\n"}},"workspace_roots":["{}"]}}"#,
            chi.display(),
            dir.path().display()
        );
        let result = evaluate(&store, args, Some(&stdin)).unwrap();
        assert_eq!(
            result.decision.permission,
            Permission::Deny,
            "{}",
            result.decision.reason
        );
        assert_eq!(result.decision.host, "cursor-tool");
        let v: serde_json::Value = serde_json::from_str(result.host_stdout.trim()).unwrap();
        assert_eq!(v["permission"], "deny");
        assert!(v.get("agent_message").is_some());
        println!("cursor-tool host fixtures passed");
    }

    #[test]
    fn cursor_tool_delete_chi_go_denies() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("gate.sqlite");
        write_fixture_db(&artifact);
        let store = empty_store(dir.path());
        let mut args = host_args("cursor-tool");
        args.artifact = Some(artifact);
        args.scan_id = Some("fixture".into());
        args.no_ledger = true;
        let stdin = r#"{"tool_name":"Delete","tool_input":{"file_path":"chi.go"},"workspace_roots":["/tmp"]}"#;
        let result = evaluate(&store, args, Some(stdin)).unwrap();
        assert_eq!(
            result.decision.permission,
            Permission::Deny,
            "{}",
            result.decision.reason
        );
        let v: serde_json::Value = serde_json::from_str(result.host_stdout.trim()).unwrap();
        assert_eq!(v["permission"], "deny");
    }

    #[test]
    fn cursor_tool_harmless_write_allows() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("gate.sqlite");
        write_fixture_db(&artifact);
        let chi = dir.path().join("chi.go");
        let body = "package chi\n\nfunc NewRouter() *Mux {\n\treturn &Mux{}\n}\n";
        std::fs::write(&chi, body).unwrap();
        let store = empty_store(dir.path());
        let mut args = host_args("cursor-tool");
        args.artifact = Some(artifact);
        args.scan_id = Some("fixture".into());
        args.no_ledger = true;
        let stdin = format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{}","content":"package chi\n\n// note\nfunc NewRouter() *Mux {{\n\treturn &Mux{{}}\n}}\n"}}}}"#,
            chi.display()
        );
        let result = evaluate(&store, args, Some(&stdin)).unwrap();
        assert_eq!(
            result.decision.permission,
            Permission::Allow,
            "{}",
            result.decision.reason
        );
        let v: serde_json::Value = serde_json::from_str(result.host_stdout.trim()).unwrap();
        assert_eq!(v["permission"], "allow");
    }

    #[test]
    fn cursor_tool_unregistered_cwd_does_not_crash() {
        let dir = TempDir::new().unwrap();
        let store = empty_store(dir.path());
        let result = evaluate(
            &store,
            host_args("cursor-tool"),
            Some(r#"{"tool_name":"Write","tool_input":{"file_path":"/no/such/unregistered/note.txt","content":"hi\n"},"workspace_roots":["/no/such/unregistered"]}"#),
        )
        .unwrap();
        assert_eq!(result.exit_code, 0, "{}", result.decision.reason);
        let v: serde_json::Value = serde_json::from_str(result.host_stdout.trim()).unwrap();
        assert!(
            v["permission"] == "allow" || v["permission"] == "deny",
            "must not crash; got {}",
            v
        );
        println!("cursor-tool host fixtures passed");
    }
}
