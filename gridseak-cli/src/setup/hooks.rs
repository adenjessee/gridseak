//! Real host hooks: Claude `PreToolUse` (file edit + Bash) and Cursor
//! `preToolUse` (Write/StrReplace/Delete/Shell) plus `beforeShellExecution`
//! (`rm` / `git rm`). Cursor `preToolUse` `ask` is not enforced; the
//! `cursor-tool` host maps ask → deny.
//!
//! The previous `postEdit` / `preDone` keys were not events either host
//! fires. `setup --verify` fails closed if these files are missing or
//! still contain the stub events.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};

const SKILL_MD: &str = r#"# GridSeak structural gate

The harness hook — not this skill — is the authority.

Claude `PreToolUse` covers Edit/Write/MultiEdit and Bash. Cursor
`preToolUse` covers Write/StrReplace/Delete/Shell; `beforeShellExecution`
still covers `rm` / `git rm`. Cursor `preToolUse` does not enforce ask
— tree-sitter-only is deny-with-reason.
against the latest scan. Compiler/LSP witnesses → deny. Dirty ∩ stale
→ ask (rescan) on Claude; deny-with-reason on Cursor preToolUse.
`deno/` / `_examples/` twins → deny with the canonical file.

Never flatten tiers. Re-query after compact: `gridseak gate --card`.
"#;

pub fn cursor_hooks_path() -> Result<PathBuf> {
    Ok(home()?.join(".cursor").join("hooks.json"))
}

pub fn claude_settings_path() -> Result<PathBuf> {
    Ok(home()?.join(".claude").join("settings.json"))
}

pub fn workspace_claude_settings() -> PathBuf {
    PathBuf::from(".claude").join("settings.json")
}

pub fn workspace_cursor_hooks() -> PathBuf {
    PathBuf::from(".cursor").join("hooks.json")
}

pub fn skill_path() -> Result<PathBuf> {
    Ok(home()?
        .join(".cursor")
        .join("skills")
        .join("gridseak-gate")
        .join("SKILL.md"))
}

/// Legacy stub path — verify must reject this as "hooks present".
#[allow(dead_code)]
pub fn legacy_stub_path() -> Result<PathBuf> {
    Ok(home()?.join(".cursor").join("hooks").join("gridseak.json"))
}

/// `binary` is the command hosts should invoke — an absolute path in
/// practice, because IDE hook runners do not inherit the shell `PATH`.
pub fn hook_wrapper_path() -> Result<PathBuf> {
    Ok(home()?
        .join(".cursor")
        .join("hooks")
        .join("gridseak-gate.sh"))
}

pub fn install(binary: &str, dry_run: bool, workspace: bool) -> Result<Vec<PathBuf>> {
    let mut paths = vec![
        cursor_hooks_path()?,
        claude_settings_path()?,
        skill_path()?,
        hook_wrapper_path()?,
    ];
    if workspace {
        paths.push(workspace_cursor_hooks());
        paths.push(workspace_claude_settings());
    }
    if dry_run {
        return Ok(paths);
    }
    write_cursor_hooks(&cursor_hooks_path()?, binary)?;
    merge_claude_settings(&claude_settings_path()?, binary)?;
    write_skill(&skill_path()?)?;
    if workspace {
        write_cursor_hooks(&workspace_cursor_hooks(), binary)?;
        merge_claude_settings(&workspace_claude_settings(), binary)?;
    }
    Ok(paths)
}

/// Quote a command path for the host's shell if it contains whitespace.
fn shell_word(s: &str) -> String {
    if s.chars().any(char::is_whitespace) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// The binary named in an installed hook command, if any.
pub fn hook_binary(path: &std::path::Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let doc: Value = serde_json::from_str(&text).ok()?;
    let mut stack = vec![&doc];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(cmd)) = map.get("command") {
                    if let Some(bin) = binary_from_hook_command(cmd) {
                        return Some(bin);
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    None
}

fn binary_from_hook_command(cmd: &str) -> Option<String> {
    for word in hook_command_words(cmd) {
        let path = std::path::Path::new(&word);
        if path.is_absolute() && path.file_name().and_then(|n| n.to_str()) == Some("gridseak") {
            return Some(word);
        }
        if path.is_absolute() && word.ends_with("gridseak-gate.sh") {
            if let Some(bin) = bin_from_wrapper(path) {
                return Some(bin);
            }
        }
    }
    if cmd.contains(" gate") {
        return hook_command_words(cmd).into_iter().next();
    }
    None
}

fn hook_command_words(cmd: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    for ch in cmd.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !buf.is_empty() {
                    words.push(std::mem::take(&mut buf));
                }
            }
            c => buf.push(c),
        }
    }
    if !buf.is_empty() {
        words.push(buf);
    }
    words
}

fn bin_from_wrapper(path: &std::path::Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("BIN=") {
            let rest = rest.trim().trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

pub fn verify_installed() -> bool {
    let cursor_ok = cursor_hooks_path()
        .ok()
        .is_some_and(|p| file_has_real_hook(&p));
    let claude_ok = claude_settings_path()
        .ok()
        .is_some_and(|p| file_has_real_hook(&p));
    let skill_ok = skill_path().ok().is_some_and(|p| p.is_file());
    cursor_ok && claude_ok && skill_ok
}

pub fn file_has_real_hook(path: &std::path::Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    if text.contains("postEdit") || text.contains("preDone") {
        return false;
    }
    if !(text.contains("beforeShellExecution")
        || text.contains("preToolUse")
        || text.contains("PreToolUse"))
    {
        return false;
    }
    has_edit_gate_command(&text)
}

/// Empty `PreToolUse: []` still contains the key name. Require a real
/// edit-gate command, not SessionStart `gate --card`.
fn has_edit_gate_command(text: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return json_has_edit_gate_command(&v);
    }
    is_edit_gate_command(text)
}

fn json_has_edit_gate_command(v: &Value) -> bool {
    match v {
        Value::String(s) => is_edit_gate_command(s),
        Value::Array(a) => a.iter().any(json_has_edit_gate_command),
        Value::Object(m) => m.values().any(json_has_edit_gate_command),
        _ => false,
    }
}

fn is_edit_gate_command(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if lower.contains("gate --card") || lower.contains("gate.sh card") || lower.ends_with(" card") {
        return false;
    }
    lower.contains("gridseak-gate") || lower.contains("gridseak gate")
}

fn write_cursor_hooks(path: &std::path::Path, binary: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut doc = if path.is_file() {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    if !doc.is_object() {
        doc = json!({});
    }
    let wrapper = write_hook_wrapper(binary)?;
    let hook_cmd = format!("/bin/bash {} cursor", wrapper.display());
    let tool_cmd = format!("/bin/bash {} cursor-tool", wrapper.display());
    let bin = shell_word(binary);
    doc["version"] = json!(1);
    doc["hooks"]["preToolUse"] = json!([
        {
            "command": tool_cmd,
            "matcher": "Write|StrReplace|Delete|Shell",
            "failClosed": true
        }
    ]);
    doc["hooks"]["beforeShellExecution"] = json!([
        {
            "command": hook_cmd,
            "failClosed": true
        }
    ]);
    doc["hooks"]["sessionStart"] = json!([
        { "command": format!("{bin} gate --card") }
    ]);
    fs::write(path, serde_json::to_string_pretty(&doc)?).context("write cursor hooks")?;
    Ok(())
}

fn write_hook_wrapper(binary: &str) -> Result<PathBuf> {
    let path = hook_wrapper_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, hook_wrapper_body(binary)).context("write hook wrapper")?;
    Ok(path)
}

fn hook_wrapper_body(binary: &str) -> String {
    let bin = binary.replace('"', "");
    format!(
        r#"#!/usr/bin/env bash
# Generated by `gridseak setup`. Hosts fail-closed on nonzero / non-JSON.
# Always print a verdict and exit 0 so a lookup crash cannot DoS the IDE.
set -u
FORMAT="${{1:-cursor}}"
BIN="{bin}"
if [ ! -x "$BIN" ]; then
  printf '%s\n' "{{\"permission\":\"ask\",\"user_message\":\"gridseak binary not found at $BIN\",\"failClosed\":true}}"
  exit 0
fi
out_file="$(mktemp -t gridseak-gate.XXXXXX)"
err_file="$(mktemp -t gridseak-gate.XXXXXX)"
set +e
"$BIN" gate --format "$FORMAT" >"$out_file" 2>"$err_file"
status=$?
set +e
if [ "$status" -eq 0 ] && [ -s "$out_file" ]; then
  cat "$out_file"
  rm -f "$out_file" "$err_file"
  exit 0
fi
reason="$(tr '\n' ' ' <"$err_file" | tr -d '"\\')"
if [ -z "$reason" ]; then
  reason="gridseak gate exited $status without a verdict"
fi
printf '%s\n' "{{\"permission\":\"ask\",\"user_message\":\"$reason\",\"failClosed\":true}}"
rm -f "$out_file" "$err_file"
exit 0
"#
    )
}

fn merge_claude_settings(path: &std::path::Path, binary: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut doc = if path.is_file() {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    if !doc.is_object() {
        doc = json!({});
    }
    let wrapper = write_hook_wrapper(binary)?;
    let gate_claude = format!("/bin/bash {} claude", wrapper.display());
    let bin = shell_word(binary);
    let gate_card = format!("{bin} gate --card");
    let pretool = json!([
        {
            "matcher": "Edit|Write|MultiEdit",
            "hooks": [
                { "type": "command", "command": gate_claude }
            ]
        },
        {
            "matcher": "Bash",
            "hooks": [
                { "type": "command", "command": format!("{bin} gate --format claude") }
            ]
        }
    ]);
    let session = json!([
        {
            "matcher": "compact|startup",
            "hooks": [
                { "type": "command", "command": gate_card }
            ]
        }
    ]);
    doc["hooks"]["PreToolUse"] = pretool;
    doc["hooks"]["SessionStart"] = session;
    fs::write(path, serde_json::to_string_pretty(&doc)?).context("write claude settings")?;
    Ok(())
}

fn write_skill(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, SKILL_MD)?;
    Ok(())
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME not set")
}

/// Serializes every test that mutates process-global `HOME`.
/// `hooks` and `verify` used to each own a lock, so they raced.
#[cfg(test)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_home() -> std::sync::MutexGuard<'static, ()> {
    HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn install_writes_real_host_events() {
        let _g = lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let written = install("/opt/gs/bin/gridseak", false, false).unwrap();
        assert!(written.len() >= 3);
        assert!(verify_installed());
        let cursor = fs::read_to_string(cursor_hooks_path().unwrap()).unwrap();
        assert!(cursor.contains("beforeShellExecution"));
        assert!(cursor.contains("preToolUse"));
        assert!(cursor.contains("Write|StrReplace|Delete|Shell"));
        assert!(!cursor.contains("postEdit"));
        assert!(
            cursor.contains("gridseak-gate.sh cursor"),
            "Cursor hook must call the fail-safe wrapper, got: {cursor}"
        );
        assert!(
            cursor.contains("gridseak-gate.sh cursor-tool"),
            "Cursor preToolUse must call the cursor-tool wrapper, got: {cursor}"
        );
        let wrapper = fs::read_to_string(hook_wrapper_path().unwrap()).unwrap();
        assert!(
            wrapper.contains("BIN=\"/opt/gs/bin/gridseak\""),
            "wrapper must pin the absolute binary, got: {wrapper}"
        );
        assert!(
            wrapper.contains("always print a verdict")
                || wrapper.contains("Always print a verdict")
        );
        let claude = fs::read_to_string(claude_settings_path().unwrap()).unwrap();
        assert!(claude.contains("PreToolUse"));
        assert!(claude.contains("Edit|Write|MultiEdit"));
        assert!(
            claude.contains("gridseak-gate.sh claude"),
            "Claude hook must call the fail-safe wrapper, got: {claude}"
        );
        assert_eq!(
            hook_binary(&cursor_hooks_path().unwrap()).as_deref(),
            Some("/opt/gs/bin/gridseak")
        );
        println!("setup hook absolute-path tests passed");
    }

    #[test]
    fn hook_binary_handles_quoted_paths_with_spaces() {
        let _g = lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        install("/Applications/Grid Seak/gridseak", false, false).unwrap();
        let wrapper = fs::read_to_string(hook_wrapper_path().unwrap()).unwrap();
        assert!(
            wrapper.contains("/Applications/Grid Seak/gridseak"),
            "quoted path missing, got: {wrapper}"
        );
        assert_eq!(
            hook_binary(&cursor_hooks_path().unwrap()).as_deref(),
            Some("/Applications/Grid Seak/gridseak")
        );
    }

    #[test]
    fn verify_fails_when_hooks_missing() {
        let _g = lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        assert!(!verify_installed());
    }

    #[test]
    fn empty_claude_pretooluse_is_not_installed() {
        let _g = lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let claude = dir.path().join(".claude").join("settings.json");
        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        fs::write(
            &claude,
            r#"{
  "hooks": {
    "PreToolUse": [],
    "SessionStart": [{
      "hooks": [{"type":"command","command":"gridseak gate --card"}]
    }]
  }
}"#,
        )
        .unwrap();
        assert!(
            !file_has_real_hook(&claude),
            "empty PreToolUse plus card-only SessionStart must fail verify"
        );
    }

    #[test]
    fn verify_rejects_legacy_postedit_stub() {
        let _g = lock_home();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        let stub = dir.path().join(".cursor").join("hooks.json");
        fs::create_dir_all(stub.parent().unwrap()).unwrap();
        fs::write(
            &stub,
            r#"{"hooks":{"postEdit":["gridseak mcp --hint diff_impact"]}}"#,
        )
        .unwrap();
        assert!(!file_has_real_hook(&stub));
    }
}
