//! Windsurf — writes `~/.codeium/windsurf/mcp_config.json` idempotently.
//!
//! Windsurf's MCP config format mirrors Cursor's (`{ "mcpServers": {...} }`),
//! so we use the same splice helper. The only difference is the path.
//! Windsurf does not currently have a documented per-workspace MCP path,
//! so we always write the home-scope file.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;

use super::common::{home_dir, read_json, write_pretty_json};

pub struct WireOutcome {
    pub config_path: PathBuf,
    pub previous_block: Option<Value>,
}

pub fn wire(binary: &str, dry_run: bool) -> Result<WireOutcome> {
    let config_path = config_path()?;
    let mut doc = read_json(&config_path).unwrap_or_else(|_| serde_json::json!({}));
    if !doc.is_object() {
        doc = serde_json::json!({});
    }

    let previous_block = doc.pointer("/mcpServers/gridseak").cloned();
    let new_block = serde_json::json!({
        "command": binary,
        "args": ["mcp"],
    });
    splice(&mut doc, "gridseak", new_block)?;

    if !dry_run {
        write_pretty_json(&config_path, &doc)?;
    }

    Ok(WireOutcome {
        config_path,
        previous_block,
    })
}

pub fn config_path() -> Result<PathBuf> {
    let home = home_dir().context("$HOME (or %USERPROFILE%) not set")?;
    Ok(home
        .join(".codeium")
        .join("windsurf")
        .join("mcp_config.json"))
}

fn splice(doc: &mut Value, name: &str, block: Value) -> Result<()> {
    let Value::Object(root) = doc else {
        anyhow::bail!("Windsurf mcp_config.json root must be an object");
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
