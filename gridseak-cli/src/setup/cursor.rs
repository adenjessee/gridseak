//! Cursor — full automation. Writes `~/.cursor/mcp.json` and
//! `~/.cursor/rules/gridseak.mdc` idempotently.
//!
//! - `mcp.json` gets `mcpServers.gridseak` spliced in; other entries are
//!   preserved (so users who already have other MCP servers don't lose
//!   them on `gridseak setup`).
//! - `gridseak.mdc` gets the rule body from [`super::rule`] so the user's
//!   Cursor agent learns when to call each of the thirteen MCP tools the
//!   moment they restart Cursor.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;

use super::common::{home_dir, read_json, write_file, write_pretty_json};
use super::routing_rule::ROUTING_RULE_BODY;
use super::rule::CURSOR_RULE_BODY;

pub struct WireOutcome {
    pub mcp_path: PathBuf,
    pub rule_path: Option<PathBuf>,
    pub routing_rule_path: Option<PathBuf>,
    pub previous_block: Option<Value>,
}

pub fn wire(
    binary: &str,
    workspace_scope: bool,
    write_rule: bool,
    dry_run: bool,
) -> Result<WireOutcome> {
    let mcp_path = mcp_path(workspace_scope)?;
    let rule_path = if write_rule { Some(rule_path()?) } else { None };
    let routing_rule_path = if write_rule {
        Some(routing_rule_path()?)
    } else {
        None
    };

    let mut doc = read_json(&mcp_path).unwrap_or_else(|_| serde_json::json!({}));
    if !doc.is_object() {
        doc = serde_json::json!({});
    }

    let previous_block = doc.pointer("/mcpServers/gridseak").cloned();
    let new_block = serde_json::json!({
        "command": binary,
        "args": ["mcp"],
    });

    splice_mcp_server(&mut doc, "gridseak", new_block)?;

    if !dry_run {
        write_pretty_json(&mcp_path, &doc)?;
        if let Some(path) = &rule_path {
            write_file(path, CURSOR_RULE_BODY)?;
        }
        if let Some(path) = &routing_rule_path {
            write_file(path, ROUTING_RULE_BODY)?;
        }
    }

    Ok(WireOutcome {
        mcp_path,
        rule_path,
        routing_rule_path,
        previous_block,
    })
}

pub fn mcp_path(workspace: bool) -> Result<PathBuf> {
    if workspace {
        let cwd = std::env::current_dir().context("could not get cwd")?;
        Ok(cwd.join(".cursor").join("mcp.json"))
    } else {
        let home = home_dir().context("$HOME (or %USERPROFILE%) not set")?;
        Ok(home.join(".cursor").join("mcp.json"))
    }
}

pub fn rule_path() -> Result<PathBuf> {
    let home = home_dir().context("$HOME not set")?;
    Ok(home.join(".cursor").join("rules").join("gridseak.mdc"))
}

pub fn routing_rule_path() -> Result<PathBuf> {
    let home = home_dir().context("$HOME not set")?;
    Ok(home
        .join(".cursor")
        .join("rules")
        .join("gridseak-routing.mdc"))
}

fn splice_mcp_server(doc: &mut Value, name: &str, block: Value) -> Result<()> {
    let Value::Object(root) = doc else {
        anyhow::bail!("Cursor mcp.json root must be an object");
    };
    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    if let Value::Object(s) = servers {
        s.insert(name.to_string(), block);
        Ok(())
    } else {
        anyhow::bail!("existing `mcpServers` entry is not an object; refusing to overwrite")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_inserts_into_empty_doc() {
        let mut doc = serde_json::json!({});
        splice_mcp_server(
            &mut doc,
            "gridseak",
            serde_json::json!({ "command": "x", "args": ["y"] }),
        )
        .unwrap();
        assert_eq!(doc["mcpServers"]["gridseak"]["command"], "x");
    }

    #[test]
    fn splice_preserves_unrelated_entries() {
        let mut doc = serde_json::json!({
            "mcpServers": { "other": { "command": "o" } },
            "unrelated": 7
        });
        splice_mcp_server(&mut doc, "gridseak", serde_json::json!({ "command": "g" })).unwrap();
        assert!(doc["mcpServers"].as_object().unwrap().contains_key("other"));
        assert!(doc["mcpServers"]
            .as_object()
            .unwrap()
            .contains_key("gridseak"));
        assert_eq!(doc["unrelated"], 7);
    }
}
