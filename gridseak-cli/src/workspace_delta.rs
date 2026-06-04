//! Workspace delta since the last completed scan — dirty paths + git buckets.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use git2::{Repository, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::IGNORED_DIRS;

pub const WORKSPACE_SNAPSHOT_KEY: &str = "workspace_snapshot";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GitBuckets {
    pub unstaged: Vec<String>,
    pub staged: Vec<String>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub scope: String,
    pub completed_at: String,
    pub dirty_buckets: GitBuckets,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceDelta {
    pub since_scan_seconds: i64,
    pub dirty_paths: Vec<String>,
    pub buckets: GitBuckets,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intersects_query: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_directive: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AnalysisReadiness {
    pub graph_ready: bool,
    pub analysis_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments_pending: Vec<String>,
}

pub fn compute_workspace_delta(
    project_root: &Path,
    scan_completed_at: &str,
    query_paths: Option<&[String]>,
) -> anyhow::Result<WorkspaceDelta> {
    let scan_ts = DateTime::parse_from_rfc3339(scan_completed_at)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let since_scan_seconds = (Utc::now() - scan_ts).num_seconds().max(0);

    let buckets = git_buckets(project_root)?;
    let mut dirty: BTreeSet<String> = BTreeSet::new();
    dirty.extend(buckets.unstaged.iter().cloned());
    dirty.extend(buckets.staged.iter().cloned());
    dirty.extend(buckets.untracked.iter().cloned());
    dirty.extend(mtime_dirty_paths(project_root, scan_ts)?);

    let dirty_paths: Vec<String> = dirty.into_iter().collect();
    let intersects_query = query_paths.map(|paths| {
        paths
            .iter()
            .filter(|p| path_intersects_dirty(p, &dirty_paths))
            .cloned()
            .collect()
    });

    let agent_directive = intersects_query.as_ref().and_then(|hits: &Vec<String>| {
        if hits.is_empty() {
            None
        } else {
            Some(format!(
                "Query target intersects {} dirty file(s) since scan — run gridseak_scan before citing structural facts for: {}",
                hits.len(),
                hits.join(", ")
            ))
        }
    });

    Ok(WorkspaceDelta {
        since_scan_seconds,
        dirty_paths,
        buckets,
        intersects_query,
        agent_directive,
    })
}

pub fn git_buckets(project_root: &Path) -> anyhow::Result<GitBuckets> {
    let repo = match Repository::discover(project_root) {
        Ok(r) => r,
        Err(_) => {
            return Ok(GitBuckets {
                unstaged: Vec::new(),
                staged: Vec::new(),
                untracked: Vec::new(),
            });
        }
    };

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .include_unmodified(false)
        .exclude_submodules(true);

    let statuses = repo.statuses(Some(&mut opts))?;

    let mut unstaged = BTreeSet::new();
    let mut staged = BTreeSet::new();
    let mut untracked = BTreeSet::new();

    for entry in statuses.iter() {
        let Some(path) = entry.path().map(normalize_rel_path) else {
            continue;
        };
        let st = entry.status();
        if st.contains(Status::WT_MODIFIED)
            || st.contains(Status::WT_DELETED)
            || st.contains(Status::WT_RENAMED)
        {
            unstaged.insert(path.clone());
        }
        if st.contains(Status::INDEX_MODIFIED)
            || st.contains(Status::INDEX_DELETED)
            || st.contains(Status::INDEX_RENAMED)
            || st.contains(Status::INDEX_NEW)
        {
            staged.insert(path.clone());
        }
        if st.contains(Status::WT_NEW) {
            untracked.insert(path);
        }
    }

    Ok(GitBuckets {
        unstaged: unstaged.into_iter().collect(),
        staged: staged.into_iter().collect(),
        untracked: untracked.into_iter().collect(),
    })
}

fn mtime_dirty_paths(project_root: &Path, scan_ts: DateTime<Utc>) -> anyhow::Result<Vec<String>> {
    let scan_system = scan_ts.timestamp();
    let mut out = BTreeSet::new();
    for entry in WalkDir::new(project_root)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !IGNORED_DIRS.iter().any(|d| *d == name.as_ref())
        })
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = path.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let modified_ts: DateTime<Utc> = modified.into();
        if modified_ts.timestamp() > scan_system {
            if let Ok(rel) = path.strip_prefix(project_root) {
                out.insert(normalize_rel_path(&rel.to_string_lossy()));
            }
        }
    }
    Ok(out.into_iter().collect())
}

pub fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

pub fn path_intersects_dirty(query: &str, dirty_paths: &[String]) -> bool {
    let q = normalize_rel_path(query);
    dirty_paths.iter().any(|d| {
        let d = normalize_rel_path(d);
        d == q || q.ends_with(&format!("/{d}")) || d.ends_with(&format!("/{q}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_intersection_matches_relative_and_suffix() {
        let dirty = vec!["gridseak-cli/src/main.rs".to_string()];
        assert!(path_intersects_dirty("gridseak-cli/src/main.rs", &dirty));
        assert!(path_intersects_dirty("./gridseak-cli/src/main.rs", &dirty));
        assert!(!path_intersects_dirty("other.rs", &dirty));
    }
}
