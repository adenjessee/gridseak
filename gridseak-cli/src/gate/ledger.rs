//! Append-only JSONL register. Compaction cannot invent these rows.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use super::decision::Decision;

#[derive(Debug, Serialize)]
struct LedgerRow<'a> {
    ts: String,
    scan_id: &'a str,
    symbols: Vec<String>,
    verdict: String,
    tier: &'a str,
    witnesses: &'a [String],
    #[serde(rename = "override")]
    override_used: bool,
    host: &'a str,
    file: &'a str,
    duration_ms: u64,
    extract_source: &'a str,
}

pub fn append_ledger(path: &Path, decision: &Decision) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create ledger dir {}", parent.display()))?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open ledger {}", path.display()))?;
    let row = LedgerRow {
        ts: chrono::Utc::now().to_rfc3339(),
        scan_id: &decision.scan_id,
        symbols: decision.symbols.iter().map(|s| s.fqn.clone()).collect(),
        verdict: format!("{:?}", decision.permission).to_lowercase(),
        tier: &decision.tier,
        witnesses: &decision.witnesses,
        override_used: decision.override_used,
        host: &decision.host,
        file: &decision.file,
        duration_ms: decision.duration_ms,
        extract_source: &decision.extract_source,
    };
    writeln!(f, "{}", serde_json::to_string(&row)?)?;
    Ok(())
}

pub fn recent_denials(path: &Path, limit: usize) -> Vec<String> {
    let Ok(f) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let verdict = v.get("verdict").and_then(|x| x.as_str()).unwrap_or("");
        if verdict != "deny" {
            continue;
        }
        let symbols = v
            .get("symbols")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let tier = v.get("tier").and_then(|x| x.as_str()).unwrap_or("?");
        out.push(format!("{symbols} [{tier}]"));
    }
    out.into_iter().rev().take(limit).collect()
}
