//! Filesystem + JSON helpers shared across IDE targets.
//!
//! Kept in a single module so a fix to JSON formatting or home-dir
//! resolution only happens once. Both [`super::cursor`] and
//! [`super::windsurf`] write `{ "mcpServers": {...} }` JSON files, so
//! their I/O contracts are identical even though they target different
//! IDE config paths.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn read_json(path: &PathBuf) -> Result<Value> {
    let raw = std::fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    Ok(serde_json::from_str(&raw)?)
}

pub fn write_pretty_json(path: &PathBuf, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let serialized = serde_json::to_string_pretty(value)? + "\n";
    std::fs::write(path, serialized).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn write_file(path: &PathBuf, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
