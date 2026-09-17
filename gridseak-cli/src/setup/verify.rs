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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    all_required_pass &= check_hooks();

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
    if !Path::new(command).is_absolute() {
        println!(
            "{label} FAIL  {} command={command} is not absolute — IDE MCP spawn has no PATH",
            path.display()
        );
        return false;
    }
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

fn check_hooks() -> bool {
    let label = "  hooks + skill";
    if !super::hooks::verify_installed() {
        println!(
            "{label} FAIL  missing real host hooks (PreToolUse / beforeShellExecution) \
             — re-run `gridseak setup`. Advisory postEdit stubs do not count."
        );
        return false;
    }
    println!("{label} OK    PreToolUse + beforeShellExecution + gate skill present");
    check_hook_binary_has_gate() && check_hook_wrapper_json()
}

/// The hook command must name a binary that (a) hosts can spawn without
/// the shell `PATH` and (b) actually has the `gate` subcommand. A bare
/// `gridseak` that resolves to a stale install fail-closes every shell
/// command in Cursor — this is the check that catches it.
fn check_hook_binary_has_gate() -> bool {
    let label = "  hook binary";
    let Ok(path) = super::hooks::cursor_hooks_path() else {
        return true;
    };
    let Some(bin) = super::hooks::hook_binary(&path) else {
        println!(
            "{label} FAIL  could not read the gate command from {}",
            path.display()
        );
        return false;
    };
    let is_absolute = Path::new(&bin).is_absolute();
    let has_gate = Command::new(&bin)
        .args(["gate", "--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    match (is_absolute, has_gate) {
        (true, true) => {
            println!("{label} OK    {bin} (absolute, has `gate`)");
            true
        }
        (false, true) => {
            println!(
                "{label} FAIL  {bin} is not an absolute path — IDE hook runners do not \
                 inherit your shell PATH. Re-run `gridseak setup` (or pass --command)."
            );
            false
        }
        (_, false) => {
            println!(
                "{label} FAIL  `{bin} gate --help` failed — this binary has no `gate` \
                 subcommand, so Cursor's failClosed hook will block every shell command. \
                 Re-run `gridseak setup` from the build you want."
            );
            false
        }
    }
}

/// Shared hook-health report used by `setup --verify` and `gridseak doctor`.
#[derive(Debug, Clone)]
pub struct HookHealth {
    pub status: String,
    pub detail: String,
    pub binary: Option<String>,
    pub absolute: bool,
    pub has_gate: bool,
    pub wrapper_json_ok: bool,
}

pub fn hook_health() -> HookHealth {
    let Ok(path) = super::hooks::cursor_hooks_path() else {
        return HookHealth {
            status: "not_configured".into(),
            detail: "HOME not set".into(),
            binary: None,
            absolute: false,
            has_gate: false,
            wrapper_json_ok: false,
        };
    };
    if !path.is_file() {
        return HookHealth {
            status: "not_configured".into(),
            detail: format!("{} missing — run `gridseak setup`", path.display()),
            binary: None,
            absolute: false,
            has_gate: false,
            wrapper_json_ok: false,
        };
    }
    let Some(bin) = super::hooks::hook_binary(&path) else {
        return HookHealth {
            status: "fail".into(),
            detail: format!("could not read the gate command from {}", path.display()),
            binary: None,
            absolute: false,
            has_gate: false,
            wrapper_json_ok: false,
        };
    };
    let absolute = Path::new(&bin).is_absolute();
    let has_gate = Command::new(&bin)
        .args(["gate", "--help"])
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout).contains("Usage: gridseak gate")
        })
        .unwrap_or(false);
    let wrapper_json_ok = wrapper_returns_json();
    let (status, detail) = match (absolute, has_gate, wrapper_json_ok) {
        (true, true, true) => (
            "ok".into(),
            format!("{bin} (absolute, has `gate`, wrapper JSON)"),
        ),
        (false, _, _) => (
            "fail".into(),
            format!("{bin} is not an absolute path — IDE hook runners do not inherit PATH"),
        ),
        (_, false, _) => (
            "fail".into(),
            format!("`{bin} gate --help` failed — this binary has no `gate` subcommand"),
        ),
        (_, _, false) => (
            "fail".into(),
            "hook wrapper did not return valid JSON for a pwd payload".into(),
        ),
    };
    HookHealth {
        status,
        detail,
        binary: Some(bin),
        absolute,
        has_gate,
        wrapper_json_ok,
    }
}

fn wrapper_returns_json() -> bool {
    let Ok(wrapper) = super::hooks::hook_wrapper_path() else {
        return false;
    };
    if !wrapper.is_file() {
        return false;
    }
    let mut child = match Command::new("/bin/bash")
        .arg(&wrapper)
        .arg("cursor")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(br#"{"command":"pwd"}"#);
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|l| l.trim().starts_with('{'))
        .unwrap_or("");
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| {
            v.get("permission")
                .and_then(Value::as_str)
                .map(|s| !s.is_empty())
        })
        .unwrap_or(false)
}

fn check_hook_wrapper_json() -> bool {
    let label = "  hook crash-safe";
    let health = hook_health();
    if health.wrapper_json_ok {
        println!("{label} OK    wrapper returns JSON for a pwd payload");
        true
    } else {
        println!("{label} FAIL  {}", health.detail);
        false
    }
}

#[cfg(test)]
mod setup_verify_g2 {
    use super::super::hooks;
    use super::*;
    use tempfile::TempDir;

    fn fake_gate_bin(dir: &std::path::Path) -> PathBuf {
        let p = dir.join("gridseak");
        std::fs::write(
            &p,
            r#"#!/bin/sh
if [ "$1" = "gate" ]; then
  if [ "$2" = "--help" ]; then
    echo "Usage: gridseak gate"
    exit 0
  fi
  echo '{"permission":"allow","user_message":"ok","failClosed":true}'
  exit 0
fi
exit 1
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&p, perms).unwrap();
        }
        p
    }

    #[test]
    fn fails_non_absolute_hook_command() {
        let _g = hooks::lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let hooks_path = dir.path().join(".cursor").join("hooks.json");
        std::fs::create_dir_all(hooks_path.parent().unwrap()).unwrap();
        std::fs::write(
            &hooks_path,
            r#"{"version":1,"hooks":{"beforeShellExecution":[{"command":"gridseak gate --format cursor"}]}}"#,
        )
        .unwrap();
        let health = hook_health();
        assert_eq!(health.status, "fail", "{health:?}");
        assert!(!health.absolute);
    }

    #[test]
    fn fails_gateless_binary() {
        let _g = hooks::lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let bin = dir.path().join("stale-gridseak");
        std::fs::write(&bin, b"#!/bin/sh\necho old\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin, perms).unwrap();
        }
        hooks::install(&bin.display().to_string(), false, false).unwrap();
        let health = hook_health();
        assert_eq!(health.status, "fail", "{health:?}");
        assert!(!health.has_gate);
    }

    #[test]
    fn fails_crashing_wrapper() {
        let _g = hooks::lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let bin = fake_gate_bin(dir.path());
        hooks::install(&bin.display().to_string(), false, false).unwrap();
        std::fs::write(hooks::hook_wrapper_path().unwrap(), "#!/bin/bash\nexit 1\n").unwrap();
        let health = hook_health();
        assert_eq!(health.status, "fail", "{health:?}");
        assert!(!health.wrapper_json_ok);
    }

    #[test]
    fn passes_absolute_gate_and_json_wrapper() {
        let _g = hooks::lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let bin = fake_gate_bin(dir.path());
        hooks::install(&bin.display().to_string(), false, false).unwrap();
        let health = hook_health();
        assert_eq!(health.status, "ok", "{health:?}");
        assert!(health.absolute && health.has_gate && health.wrapper_json_ok);
        println!("setup verify G2 checks passed");
    }
}
