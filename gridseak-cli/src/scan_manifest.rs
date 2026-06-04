//! Scan manifest for future live overlay / trace consumers (Phase 6 foundation).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::workspace_delta::{
    git_buckets, normalize_rel_path, WorkspaceSnapshot, WORKSPACE_SNAPSHOT_KEY,
};

#[derive(Debug, Serialize)]
pub struct ScanManifest {
    pub scan_id: String,
    pub baseline: bool,
    pub scope: String,
    pub completed_at: String,
    pub dirty_paths: Vec<String>,
    pub change_status_by_file: std::collections::BTreeMap<String, String>,
}

pub fn write_scan_manifest(
    scan_id: &str,
    completed_at: &str,
    project_root: &Path,
    graph_artifact: &Path,
) -> Result<PathBuf> {
    let buckets = git_buckets(project_root)?;
    let mut dirty_paths = buckets.unstaged.clone();
    dirty_paths.extend(buckets.staged.clone());
    dirty_paths.extend(buckets.untracked.clone());
    dirty_paths.sort();
    dirty_paths.dedup();

    let mut change_status_by_file = std::collections::BTreeMap::new();
    for p in &buckets.unstaged {
        change_status_by_file.insert(normalize_rel_path(p), "ChangedThisSession".into());
    }
    for p in &buckets.staged {
        change_status_by_file.insert(normalize_rel_path(p), "StagedThisSession".into());
    }
    for p in &buckets.untracked {
        change_status_by_file.insert(normalize_rel_path(p), "UntrackedThisSession".into());
    }

    let manifest = ScanManifest {
        scan_id: scan_id.to_string(),
        baseline: true,
        scope: "working_tree".into(),
        completed_at: completed_at.to_string(),
        dirty_paths: dirty_paths.clone(),
        change_status_by_file,
    };

    let out = graph_artifact
        .parent()
        .unwrap_or(project_root)
        .join("scan_manifest.json");
    fs::write(&out, serde_json::to_string_pretty(&manifest)?)
        .context("write scan_manifest.json")?;

    let snapshot = WorkspaceSnapshot {
        scope: "working_tree".into(),
        completed_at: completed_at.to_string(),
        dirty_buckets: buckets,
    };
    if let Ok(conn) = rusqlite::Connection::open(graph_artifact) {
        let json = serde_json::to_string(&snapshot)?;
        let _ = graphengine_parsing::infrastructure::storage::parse_meta_store::upsert_parse_meta(
            &conn,
            WORKSPACE_SNAPSHOT_KEY,
            &json,
        );
    }

    Ok(out)
}
