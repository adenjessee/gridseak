//! `gridseak setup --verify` — post-install sanity check.
//!
//! Catches the most common silent failure mode: setup ran without error
//! but the agent isn't actually calling our tools (binary path wrong,
//! mcp.json schema drift, rule file missing, etc.). Each check prints
//! its own pass/fail line so the user can see exactly which step broke.
//!
//! Returns `Ok(())` if all REQUIRED checks pass. Cursor wiring + rule
//! file are required; Claude Code / Codex / Windsurf are best-effort
//! because they're either user-controlled or optional.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use serde_json::Value;

use super::common::read_json;
use super::cursor;

pub fn run() -> Result<()> {
    println!("gridseak setup --verify");
    println!();

    let mut all_required_pass = true;

    let cursor_mcp = cursor::mcp_path(false)?;
    all_required_pass &= check_cursor_mcp(&cursor_mcp);

    let cursor_rule = cursor::rule_path()?;
    all_required_pass &= check_cursor_rule(&cursor_rule);

    let cursor_routing = cursor::routing_rule_path()?;
    all_required_pass &= check_cursor_routing_rule(&cursor_routing);

    all_required_pass &= check_binary_resolvable();

    println!();
    if all_required_pass {
        println!("OK. Restart your IDE if you have not already; GridSeak's MCP tools");
        println!("should appear in your agent's tool list. Ask: \"what's risky to");
        println!("refactor here?\" — the agent should call gridseak_get_recommendations.");
        Ok(())
    } else {
        anyhow::bail!("one or more required checks failed; see lines above")
    }
}

fn check_cursor_mcp(path: &Path) -> bool {
    let label = "  Cursor mcp.json";
    if !path.exists() {
        println!(
            "{label} FAIL  {} (missing — run `gridseak setup`)",
            path.display()
        );
        return false;
    }
    let doc = match read_json(&path.to_path_buf()) {
        Ok(v) => v,
        Err(e) => {
            println!("{label} FAIL  {} ({})", path.display(), e);
            return false;
        }
    };
    let block = match doc.pointer("/mcpServers/gridseak") {
        Some(v) => v.clone(),
        None => {
            println!(
                "{label} FAIL  {} (no `mcpServers.gridseak` entry — run `gridseak setup`)",
                path.display()
            );
            return false;
        }
    };
    let command = match block.get("command").and_then(Value::as_str) {
        Some(v) => v,
        None => {
            println!(
                "{label} FAIL  {} (block has no `command` field)",
                path.display()
            );
            return false;
        }
    };
    println!("{label} OK    {} (command={})", path.display(), command);
    true
}

fn check_cursor_rule(path: &Path) -> bool {
    let label = "  Cursor rule";
    if !path.exists() {
        println!(
            "{label} FAIL  {} (missing — run `gridseak setup` without --no-rule)",
            path.display()
        );
        return false;
    }
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len < 100 {
        println!(
            "{label} FAIL  {} ({}b — looks truncated)",
            path.display(),
            len
        );
        return false;
    }
    println!("{label} OK    {} ({}b)", path.display(), len);
    true
}

fn check_cursor_routing_rule(path: &Path) -> bool {
    let label = "  Cursor routing rule";
    if !path.exists() {
        println!(
            "{label} FAIL  {} (missing — run `gridseak setup` without --no-rule)",
            path.display()
        );
        return false;
    }
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len < 100 {
        println!(
            "{label} FAIL  {} ({}b — looks truncated)",
            path.display(),
            len
        );
        return false;
    }
    println!("{label} OK    {} ({}b)", path.display(), len);
    true
}

fn check_binary_resolvable() -> bool {
    let label = "  gridseak binary";
    // We're running, so `current_exe()` finds us. The bigger question
    // is whether the *path written into mcp.json* points somewhere that
    // exists. We can't introspect that path without re-reading mcp.json
    // (which we already did), so cross-validate.
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            println!("{label} FAIL  could not resolve current_exe ({e})");
            return false;
        }
    };
    let version = Command::new(&exe)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "<unknown>".into());
    println!("{label} OK    {} ({})", exe.display(), version.trim());

    // Also resolve `gridseak` on PATH and warn if it differs from the
    // current_exe. A common footgun is two installs (one from cargo, one
    // from the install script) sitting on PATH at once.
    if let Ok(output) = Command::new("which").arg("gridseak").output() {
        if output.status.success() {
            let path_resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_resolved.is_empty()
                && PathBuf::from(&path_resolved).canonicalize().ok() != exe.canonicalize().ok()
            {
                println!(
                    "  PATH gridseak    NOTE  shell PATH resolves `gridseak` to {} but \
                     current_exe is {} — make sure mcp.json points to the one you want",
                    path_resolved,
                    exe.display()
                );
            }
        }
    }

    true
}
