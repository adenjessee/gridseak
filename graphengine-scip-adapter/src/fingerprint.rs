//! Language-scoped SCIP index fingerprint: indexer version + blake3 of the file set.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::provisioning::registry::{cache_dir, entry_for};
use crate::provisioning::IndexerLanguage;

/// Sidecar written next to a cached `index.scip`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFingerprint {
    pub language: String,
    pub indexer_version: String,
    pub file_set_blake3: String,
    pub generated_unix_secs: u64,
}

impl IndexFingerprint {
    pub fn encode(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.language, self.indexer_version, self.file_set_blake3, self.generated_unix_secs
        )
    }

    pub fn short_id(&self) -> String {
        self.file_set_blake3.chars().take(12).collect()
    }

    pub fn age_seconds(&self, now_unix: u64) -> u64 {
        now_unix.saturating_sub(self.generated_unix_secs)
    }
}

/// Hash the language's file set (sorted relative paths + file blake3).
pub fn fingerprint_workspace(
    workspace: &Path,
    language: IndexerLanguage,
) -> Option<IndexFingerprint> {
    let entry = entry_for(language)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(entry.pinned_version.as_bytes());
    hasher.update(b"\n");
    let mut files = collect_language_files(workspace, entry.detection_globs);
    files.sort();
    for path in &files {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        let abs = workspace.join(path);
        if let Ok(bytes) = fs::read(&abs) {
            hasher.update(blake3::hash(&bytes).as_bytes());
        }
        hasher.update(b"\n");
    }
    Some(IndexFingerprint {
        language: language.as_str().to_string(),
        indexer_version: entry.pinned_version.to_string(),
        file_set_blake3: hasher.finalize().to_hex().to_string(),
        generated_unix_secs: unix_now(),
    })
}

pub fn sidecar_path(workspace: &Path, language: IndexerLanguage) -> Option<PathBuf> {
    let entry = entry_for(language)?;
    Some(cache_dir(workspace, entry).join("fingerprint.txt"))
}

pub fn write_sidecar(workspace: &Path, language: IndexerLanguage, fp: &IndexFingerprint) {
    if let Some(path) = sidecar_path(workspace, language) {
        let _ = fs::create_dir_all(path.parent().unwrap_or(workspace));
        let _ = fs::write(path, fp.encode());
    }
}

pub fn read_sidecar(workspace: &Path, language: IndexerLanguage) -> Option<IndexFingerprint> {
    let path = sidecar_path(workspace, language)?;
    let raw = fs::read_to_string(path).ok()?;
    parse_encoded(&raw)
}

/// True when the on-disk sidecar matches the current file-set hash
/// (timestamp is ignored).
pub fn is_fresh(workspace: &Path, language: IndexerLanguage) -> bool {
    let Some(current) = fingerprint_workspace(workspace, language) else {
        return false;
    };
    match read_sidecar(workspace, language) {
        Some(stored) => {
            stored.language == current.language
                && stored.indexer_version == current.indexer_version
                && stored.file_set_blake3 == current.file_set_blake3
        }
        None => false,
    }
}

fn parse_encoded(raw: &str) -> Option<IndexFingerprint> {
    let parts: Vec<&str> = raw.trim().split('|').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(IndexFingerprint {
        language: parts[0].to_string(),
        indexer_version: parts[1].to_string(),
        file_set_blake3: parts[2].to_string(),
        generated_unix_secs: parts[3].parse().ok()?,
    })
}

fn collect_language_files(workspace: &Path, globs: &[&str]) -> Vec<String> {
    let exts: Vec<&str> = globs
        .iter()
        .filter(|g| g.starts_with("*."))
        .map(|g| g.trim_start_matches('*'))
        .collect();
    let mut out = Vec::new();
    walk(workspace, workspace, &exts, &mut out, 0);
    out
}

fn walk(root: &Path, dir: &Path, exts: &[&str], out: &mut Vec<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, exts, out, depth + 1);
        } else if exts.iter().any(|e| name.ends_with(e)) {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn fingerprint_changes_when_file_changes() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.ts"), "export const a = 1;").unwrap();
        let first = fingerprint_workspace(dir.path(), IndexerLanguage::TypeScript).unwrap();
        fs::write(dir.path().join("a.ts"), "export const a = 2;").unwrap();
        let second = fingerprint_workspace(dir.path(), IndexerLanguage::TypeScript).unwrap();
        assert_ne!(first.file_set_blake3, second.file_set_blake3);
    }

    #[test]
    fn sidecar_round_trip() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.ts"), "x").unwrap();
        let fp = fingerprint_workspace(dir.path(), IndexerLanguage::TypeScript).unwrap();
        write_sidecar(dir.path(), IndexerLanguage::TypeScript, &fp);
        let back = read_sidecar(dir.path(), IndexerLanguage::TypeScript).unwrap();
        assert_eq!(back.file_set_blake3, fp.file_set_blake3);
        assert!(is_fresh(dir.path(), IndexerLanguage::TypeScript));
    }
}
