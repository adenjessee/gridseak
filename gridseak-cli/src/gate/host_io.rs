//! Claude PreToolUse / Cursor beforeShellExecution / Cursor preToolUse JSON.
//!
//! Captured Cursor 3.20.21 `preToolUse` stdin (2026-09-16, this repo):
//!   Write (this host remaps StrReplace → Write):
//!     { "hook_event_name":"preToolUse", "tool_name":"Write",
//!       "tool_input":{"file_path":"<abs>","content":"<new body>"},
//!       "workspace_roots":["<workspace>"] }
//!   Delete:
//!     { "hook_event_name":"preToolUse", "tool_name":"Delete",
//!       "tool_input":{"file_path":"<abs>"} }
//! No top-level `cwd`. No `old_string` on Write — the gate must read
//! the existing file from disk to see removed names.
//!
//! Cursor `preToolUse` output is `{permission, user_message, agent_message}`.
//! `ask` is accepted by the schema but **not enforced**. `CursorTool`
//! therefore maps Ask → Deny with a documented reason prefix.

use serde::Deserialize;
use serde_json::{json, Value};

use super::decision::{Decision, Permission};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFormat {
    Claude,
    Cursor,
    CursorTool,
    Json,
}

impl HostFormat {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "pretooluse" => HostFormat::Claude,
            "cursor" | "beforeshellexecution" => HostFormat::Cursor,
            "cursor-tool" | "cursortool" | "pretooluse-cursor" => HostFormat::CursorTool,
            _ => HostFormat::Json,
        }
    }
}

pub fn detect_host(hint: &str) -> HostFormat {
    HostFormat::parse(hint)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HostPayload {
    #[serde(default, alias = "hookEventName")]
    #[allow(dead_code)]
    pub hook_event_name: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub old_string: Option<String>,
    #[serde(default)]
    pub new_string: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default, alias = "content")]
    pub contents: Option<String>,
    /// Cursor `beforeShellExecution` working directory, when provided.
    #[serde(default, alias = "working_directory", alias = "workspace_root")]
    pub cwd: Option<String>,
    /// Captured Cursor `preToolUse` workspace roots (no top-level cwd).
    #[serde(default)]
    pub workspace_roots: Option<Vec<String>>,
}

impl HostPayload {
    pub fn tool_name(&self) -> Option<String> {
        self.tool_name.clone().or_else(|| {
            self.tool_input.as_ref().and_then(|v| {
                v.get("tool_name")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
        })
    }

    pub fn is_delete_tool(&self) -> bool {
        self.tool_name()
            .map(|n| n.eq_ignore_ascii_case("delete"))
            .unwrap_or(false)
    }

    pub fn working_dir(&self) -> Option<String> {
        if let Some(c) = &self.cwd {
            if !c.is_empty() {
                return Some(c.clone());
            }
        }
        self.workspace_roots
            .as_ref()
            .and_then(|roots| roots.first())
            .cloned()
    }

    pub fn file_path(&self) -> Option<String> {
        if let Some(p) = &self.file_path {
            return Some(p.clone());
        }
        if let Some(p) = &self.path {
            return Some(p.clone());
        }
        self.tool_input.as_ref().and_then(|v| {
            v.get("file_path")
                .or_else(|| v.get("path"))
                .or_else(|| v.get("target_file"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
    }

    pub fn old_text(&self) -> Option<String> {
        if let Some(s) = &self.old_string {
            return Some(s.clone());
        }
        self.tool_input.as_ref().and_then(|v| {
            v.get("old_string")
                .or_else(|| v.get("old_str"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
    }

    pub fn new_text(&self) -> Option<String> {
        if let Some(s) = &self.new_string {
            return Some(s.clone());
        }
        if let Some(s) = &self.contents {
            return Some(s.clone());
        }
        self.tool_input.as_ref().and_then(|v| {
            v.get("new_string")
                .or_else(|| v.get("contents"))
                .or_else(|| v.get("content"))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
    }

    pub fn shell_command(&self) -> Option<String> {
        if let Some(c) = &self.command {
            return Some(c.clone());
        }
        self.tool_input.as_ref().and_then(|v| {
            v.get("command")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
    }
}

const ASK_TO_DENY_PREFIX: &str = "Cursor preToolUse does not enforce ask — treating as deny. ";

pub fn render(format: &HostFormat, decision: &Decision) -> String {
    match format {
        HostFormat::Claude => {
            let perm = match decision.permission {
                Permission::Deny => "deny",
                Permission::Ask => "ask",
                Permission::Allow => "allow",
            };
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": perm,
                    "permissionDecisionReason": decision.reason,
                }
            })
            .to_string()
                + "\n"
        }
        HostFormat::Cursor => {
            let perm = match decision.permission {
                Permission::Deny => "deny",
                Permission::Ask => "ask",
                Permission::Allow => "allow",
            };
            json!({
                "permission": perm,
                "user_message": decision.reason,
                "failClosed": true,
            })
            .to_string()
                + "\n"
        }
        HostFormat::CursorTool => {
            // Ask is not enforced on preToolUse. Fail closed: Ask → Deny.
            let (perm, reason) = match decision.permission {
                Permission::Allow => ("allow", decision.reason.clone()),
                Permission::Deny => ("deny", decision.reason.clone()),
                Permission::Ask => ("deny", format!("{ASK_TO_DENY_PREFIX}{}", decision.reason)),
            };
            json!({
                "permission": perm,
                "user_message": reason,
                "agent_message": reason,
            })
            .to_string()
                + "\n"
        }
        HostFormat::Json => serde_json::to_string_pretty(decision).unwrap_or_default() + "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::decision::{Decision, Permission};

    fn deny() -> Decision {
        Decision {
            permission: Permission::Deny,
            reason: "compiler callers exist".into(),
            scan_id: "abc".into(),
            symbols: vec![],
            tier: "compiler".into(),
            witnesses: vec!["middleware::TestStripSlashes".into()],
            override_used: false,
            host: "cursor".into(),
            file: "chi.go".into(),
            duration_ms: 1,
            extract_source: "tree-sitter".into(),
        }
    }

    fn ask() -> Decision {
        let mut d = deny();
        d.permission = Permission::Ask;
        d.reason = "tree-sitter only".into();
        d.tier = "treesitter".into();
        d
    }

    #[test]
    fn claude_canonical_deny_json() {
        let s = render(&HostFormat::Claude, &deny());
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    #[test]
    fn cursor_deny_fail_closed() {
        let s = render(&HostFormat::Cursor, &deny());
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["permission"], "deny");
        assert_eq!(v["failClosed"], true);
    }

    #[test]
    fn cursor_tool_maps_ask_to_deny() {
        let s = render(&HostFormat::CursorTool, &ask());
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["permission"], "deny");
        assert!(
            v["user_message"]
                .as_str()
                .unwrap()
                .contains("does not enforce ask"),
            "{}",
            v
        );
        assert_eq!(v["user_message"], v["agent_message"]);
        assert!(v.get("failClosed").is_none());
    }

    #[test]
    fn captured_write_payload_shape() {
        let raw = r#"{
            "hook_event_name":"preToolUse",
            "tool_name":"Write",
            "tool_input":{"file_path":"/tmp/chi.go","content":"package chi\n"},
            "workspace_roots":["/tmp"]
        }"#;
        let p: HostPayload = serde_json::from_str(raw).unwrap();
        assert_eq!(p.tool_name().as_deref(), Some("Write"));
        assert_eq!(p.file_path().as_deref(), Some("/tmp/chi.go"));
        assert_eq!(p.new_text().as_deref(), Some("package chi\n"));
        assert_eq!(p.working_dir().as_deref(), Some("/tmp"));
        assert!(!p.is_delete_tool());
    }

    #[test]
    fn captured_delete_payload_shape() {
        let raw = r#"{
            "hook_event_name":"preToolUse",
            "tool_name":"Delete",
            "tool_input":{"file_path":"/tmp/chi.go"},
            "workspace_roots":["/tmp"]
        }"#;
        let p: HostPayload = serde_json::from_str(raw).unwrap();
        assert!(p.is_delete_tool());
        assert_eq!(p.file_path().as_deref(), Some("/tmp/chi.go"));
    }
}
