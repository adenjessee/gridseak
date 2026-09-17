//! `gridseak setup` — wire GridSeak's MCP server into supported IDEs.
//!
//! This is the multi-IDE orchestrator. Each IDE target lives in its own
//! sub-module ([`cursor`], [`claude_code`], [`codex`], [`windsurf`]) so a
//! fix to one path can't break another.
//!
//! Default behaviour (`gridseak setup`):
//!
//! 1. **Cursor** — fully automated. Writes `~/.cursor/mcp.json` and
//!    `~/.cursor/rules/gridseak.mdc` (pass `--no-rule` to skip the rule).
//! 2. **Claude Code** — prints the official `claude mcp add` command.
//! 3. **Codex** — prints the TOML snippet to paste into `~/.codex/config.toml`.
//! 4. **Windsurf** — fully automated when the user has Windsurf installed
//!    (detected by the existence of `~/.codeium/windsurf/`); otherwise
//!    skipped silently.
//!
//! `--only <ide>` restricts to one target. `--verify` runs the post-install
//! sanity checks. `--dry-run` shows what would happen without writing files.

mod claude_code;
mod codex;
mod common;
mod cursor;
mod hooks;
mod routing_rule;
mod rule;
mod verify;
pub use verify::hook_health;
mod windsurf;

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};

#[derive(Args, Debug, Clone)]
pub struct SetupArgs {
    /// Override the binary we register. Default: this executable's
    /// canonical absolute path. Cursor's GUI and hook runner do not
    /// inherit a login-shell PATH, so a bare `gridseak` hits an older
    /// install (or nothing). Pass an absolute path only.
    #[arg(long)]
    pub command: Option<String>,

    /// Limit setup to one IDE target. Repeatable.
    #[arg(long, value_enum)]
    pub only: Vec<IdeTarget>,

    /// Skip writing the Cursor rule file (`~/.cursor/rules/gridseak.mdc`).
    /// You can still get the rule body via `gridseak setup --print-rule`.
    #[arg(long, default_value_t = false)]
    pub no_rule: bool,

    /// Write Cursor's mcp.json to `<cwd>/.cursor/mcp.json` instead of
    /// `~/.cursor/mcp.json`. Useful for repo-scoped configurations.
    #[arg(long, default_value_t = false)]
    pub workspace: bool,

    /// Print what would be written; don't write anything.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Post-install sanity check. Confirms mcp.json + rule file exist and
    /// the binary resolves. Use this whenever the agent doesn't seem to
    /// be calling GridSeak tools.
    #[arg(long, default_value_t = false)]
    pub verify: bool,

    /// Emit the Cursor rule body to stdout and exit. Useful for piping
    /// into a workspace-scoped rule path or inspecting what setup would
    /// write.
    #[arg(long, default_value_t = false)]
    pub print_rule: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdeTarget {
    Cursor,
    ClaudeCode,
    Codex,
    Windsurf,
}

pub fn run(args: SetupArgs) -> Result<()> {
    if args.print_rule {
        print!("{}", rule::CURSOR_RULE_BODY);
        return Ok(());
    }
    if args.verify {
        return verify::run();
    }

    let binary = resolve_setup_binary(args.command.clone())?;

    let targets = resolve_targets(&args);

    println!("gridseak setup — wiring the GridSeak MCP server into your IDE(s).");
    println!("Binary: {binary}");
    if args.dry_run {
        println!("(dry-run) no files will be written.");
    }
    println!();

    for target in &targets {
        match target {
            IdeTarget::Cursor => wire_cursor(&args, &binary)?,
            IdeTarget::ClaudeCode => claude_code::print_instructions(&binary),
            IdeTarget::Codex => codex::print_instructions(&binary),
            IdeTarget::Windsurf => wire_windsurf(&args, &binary)?,
        }
        println!();
    }

    match hooks::install(&binary, args.dry_run, args.workspace) {
        Ok(paths) => {
            println!("Hooks + shadow skill:");
            for p in paths {
                println!("  {}", p.display());
            }
        }
        Err(err) => println!("Hooks not installed ({err})."),
    }

    println!("Done.");
    println!();
    println!("Next steps:");
    println!("  1. Restart your IDE so the MCP server registers.");
    println!("     Always-on catalog is `gridseak mcp --slim` (router + 4 verbs).");
    println!("     Or `/plugin install` from plugin/ — see plugin/README.md.");
    println!("  2. From this repo, run `gridseak scan .` to produce the first scan.");
    println!("  3. Open a fresh chat and ask: \"what's risky to refactor here?\"");
    println!("     — your agent should call gridseak_get_recommendations within");
    println!("     its first two tool calls. If it doesn't, run `gridseak setup --verify`.");
    Ok(())
}

/// IDE GUIs spawn MCP servers and hooks without the user's shell PATH.
/// A bare `gridseak` either fails to spawn (MCP "down" → auth prompts)
/// or resolves to a stale install with no `gate` (fail-closed DoS).
fn default_binary() -> Result<String> {
    let exe = std::env::current_exe().context("could not resolve current_exe")?;
    canonicalize_setup_command(&exe)
}

fn resolve_setup_binary(explicit: Option<String>) -> Result<String> {
    match explicit {
        Some(cmd) => canonicalize_setup_command(std::path::Path::new(&cmd)),
        None => default_binary(),
    }
}

fn canonicalize_setup_command(cmd: &std::path::Path) -> Result<String> {
    if !cmd.is_absolute() {
        anyhow::bail!(
            "refusing bare command `{}` — IDE hook runners do not inherit PATH. \
             Pass --command with an absolute path, or run this binary so setup \
             can write current_exe().",
            cmd.display()
        );
    }
    cmd.canonicalize()
        .map(|p| p.display().to_string())
        .with_context(|| format!("cannot canonicalize {}", cmd.display()))
}

fn resolve_targets(args: &SetupArgs) -> Vec<IdeTarget> {
    if !args.only.is_empty() {
        return args.only.clone();
    }
    // Default: every target. Cursor + Windsurf auto-write; Claude Code +
    // Codex print. We don't probe for installed IDEs because the cost of
    // an unwanted print is small and a missed install is silent failure.
    vec![
        IdeTarget::Cursor,
        IdeTarget::ClaudeCode,
        IdeTarget::Codex,
        IdeTarget::Windsurf,
    ]
}

fn wire_cursor(args: &SetupArgs, binary: &str) -> Result<()> {
    println!("[Cursor]");
    let outcome = cursor::wire(binary, args.workspace, !args.no_rule, args.dry_run)?;
    println!("  mcp.json:  {}", outcome.mcp_path.display());
    match &outcome.previous_block {
        Some(old) => println!(
            "  replacing previous gridseak block: {}",
            serde_json::to_string(old).unwrap_or_default()
        ),
        None => println!("  no previous gridseak block — inserting fresh."),
    }
    if let Some(rule_path) = &outcome.rule_path {
        println!(
            "  rule:      {} ({}b)",
            rule_path.display(),
            rule::CURSOR_RULE_BODY.len()
        );
    } else {
        println!("  rule:      (skipped — --no-rule)");
    }
    if let Some(routing_path) = &outcome.routing_rule_path {
        println!(
            "  routing:   {} ({}b)",
            routing_path.display(),
            routing_rule::ROUTING_RULE_BODY.len()
        );
    }
    if !args.dry_run {
        println!("  written.");
    }
    Ok(())
}

fn wire_windsurf(args: &SetupArgs, binary: &str) -> Result<()> {
    println!("[Windsurf]");
    let path = windsurf::config_path()?;
    if !path.parent().map(|p| p.exists()).unwrap_or(false) {
        println!(
            "  skipped — {} does not exist (Windsurf not installed?)",
            path.parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
        return Ok(());
    }
    let outcome = windsurf::wire(binary, args.dry_run)?;
    println!("  config:    {}", outcome.config_path.display());
    match &outcome.previous_block {
        Some(old) => println!(
            "  replacing previous gridseak block: {}",
            serde_json::to_string(old).unwrap_or_default()
        ),
        None => println!("  no previous gridseak block — inserting fresh."),
    }
    if !args.dry_run {
        println!("  written.");
    }
    Ok(())
}

#[cfg(test)]
mod setup_binary {
    use super::canonicalize_setup_command;
    use std::path::Path;

    #[test]
    fn refuses_bare_path_name() {
        let err = canonicalize_setup_command(Path::new("gridseak")).unwrap_err();
        assert!(err.to_string().contains("refusing bare command"), "{err}");
        println!("setup binary resolver tests passed");
    }

    #[test]
    fn accepts_absolute_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("gridseak");
        std::fs::write(&p, b"x").unwrap();
        let got = canonicalize_setup_command(&p).unwrap();
        assert!(std::path::Path::new(&got).is_absolute(), "{got}");
        assert!(got.ends_with("gridseak"), "{got}");
    }
}
