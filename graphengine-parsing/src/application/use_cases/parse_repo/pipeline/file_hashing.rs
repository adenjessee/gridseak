//! Per-file content hashing for the S1 incremental-scanning cache.
//!
//! Computes a blake3 digest of every discovered source file's raw
//! bytes. Hashing raw bytes (not normalised text) is deliberate: the
//! incremental cache must invalidate on any source change, including
//! comment edits, whitespace shifts, and annotation tweaks — comments
//! can be doc-string magic or annotation hints the extractor consumes.
//! See `docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md` §4.1.
//!
//! Hashing is parallelised with Rayon; on a mid-size repo (~600 files,
//! ~30 MB of source) the full pass measures sub-100 ms on warm cache,
//! which is dwarfed by the parse step it gates.

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::warn;

use super::super::super::super::errors::ParsingError;

/// One file's hashed content paired with its (lossy-string-converted)
/// path key. The path is the same string the rest of the pipeline
/// uses for `file_extraction_coverage.file_path` and for `Node`
/// `location.file` — keeping the key consistent across tables lets a
/// later join-by-path work without normalisation surprises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHash {
    /// Lossy string form of the file path. We store the same key the
    /// rest of the pipeline uses for per-file persistence so a join
    /// across tables stays byte-identical.
    pub file_path: String,
    /// blake3 hex digest of the file's raw bytes. Lowercase, 64 chars.
    pub content_hash: String,
}

/// Result of hashing one file. Holds the canonical path so later
/// merge logic can look the path up against discovery's output.
fn hash_one(path: &Path) -> Result<FileHash, ParsingError> {
    let bytes = std::fs::read(path).map_err(|err| {
        ParsingError::extraction(format!(
            "failed to read '{}' for incremental hashing: {}",
            path.display(),
            err
        ))
    })?;
    let digest = blake3::hash(&bytes);
    Ok(FileHash {
        file_path: path.to_string_lossy().into_owned(),
        content_hash: digest.to_hex().to_string(),
    })
}

/// Hash all `files` in parallel. Returns a `BTreeMap` keyed by the
/// lossy path string so downstream lookups are deterministic in
/// iteration order (handy for snapshot tests and progress emission).
///
/// Errors from individual reads are aggregated: any unreadable file
/// fails the whole pass. The pre-existing `file_discovery` step has
/// already filtered to "supported source files we can canonicalise",
/// so an unreadable file at this stage is a hard error (permissions,
/// disappearing-mid-scan) rather than a soft skip.
pub fn hash_files(files: &[PathBuf]) -> Result<BTreeMap<String, FileHash>, ParsingError> {
    if files.is_empty() {
        return Ok(BTreeMap::new());
    }

    let results: Vec<Result<FileHash, ParsingError>> =
        files.par_iter().map(|path| hash_one(path)).collect();

    let mut map = BTreeMap::new();
    let mut errors: Vec<String> = Vec::new();
    for result in results {
        match result {
            Ok(file_hash) => {
                map.insert(file_hash.file_path.clone(), file_hash);
            }
            Err(err) => errors.push(err.to_string()),
        }
    }

    if !errors.is_empty() {
        warn!(
            "incremental-hashing: {} of {} files failed to read",
            errors.len(),
            files.len()
        );
        return Err(ParsingError::extraction(format!(
            "incremental hashing failed for {} files: first error: {}",
            errors.len(),
            errors.first().cloned().unwrap_or_default()
        )));
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_fixture(dir: &TempDir, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create fixture");
        file.write_all(contents).expect("write fixture");
        path
    }

    #[test]
    fn hash_files_empty_input_returns_empty_map() {
        let map = hash_files(&[]).expect("empty input must not error");
        assert!(map.is_empty());
    }

    #[test]
    fn hash_files_returns_one_entry_per_file_keyed_by_path() {
        let dir = TempDir::new().unwrap();
        let a = write_fixture(&dir, "a.rs", b"fn a() {}");
        let b = write_fixture(&dir, "b.rs", b"fn b() {}");

        let map = hash_files(&[a.clone(), b.clone()]).expect("hash files");
        assert_eq!(map.len(), 2);
        assert!(map.contains_key(&a.to_string_lossy().into_owned()));
        assert!(map.contains_key(&b.to_string_lossy().into_owned()));
    }

    #[test]
    fn hash_files_produces_64_char_lowercase_hex_digest() {
        let dir = TempDir::new().unwrap();
        let path = write_fixture(&dir, "x.rs", b"hello world");

        let map = hash_files(std::slice::from_ref(&path)).expect("hash one file");
        let entry = map.get(&path.to_string_lossy().into_owned()).unwrap();
        assert_eq!(entry.content_hash.len(), 64);
        assert!(entry
            .content_hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn hash_files_byte_identical_inputs_produce_identical_hashes() {
        let dir = TempDir::new().unwrap();
        let a = write_fixture(&dir, "a.rs", b"identical bytes");
        let b = write_fixture(&dir, "b.rs", b"identical bytes");

        let map = hash_files(&[a.clone(), b.clone()]).expect("hash");
        let hash_a = &map[&a.to_string_lossy().into_owned()].content_hash;
        let hash_b = &map[&b.to_string_lossy().into_owned()].content_hash;
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn hash_files_single_byte_change_produces_different_hash() {
        let dir = TempDir::new().unwrap();
        let a = write_fixture(&dir, "a.rs", b"fn a() { 1 }");
        let b = write_fixture(&dir, "b.rs", b"fn a() { 2 }");

        let map = hash_files(&[a.clone(), b.clone()]).expect("hash");
        let hash_a = &map[&a.to_string_lossy().into_owned()].content_hash;
        let hash_b = &map[&b.to_string_lossy().into_owned()].content_hash;
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn hash_files_comment_only_change_invalidates_cache_key() {
        // The S1 cache hashes raw bytes, not normalised text. A
        // comment-only edit must produce a different hash so the
        // cache invalidates — comments may be doc-string magic the
        // extractor consumes (Apex annotations, Rust doc-tests, etc.).
        let dir = TempDir::new().unwrap();
        let a = write_fixture(&dir, "a.rs", b"// before\nfn x() {}");
        let b = write_fixture(&dir, "b.rs", b"// after\nfn x() {}");

        let map = hash_files(&[a.clone(), b.clone()]).expect("hash");
        let hash_a = &map[&a.to_string_lossy().into_owned()].content_hash;
        let hash_b = &map[&b.to_string_lossy().into_owned()].content_hash;
        assert_ne!(
            hash_a, hash_b,
            "comment-only edit must invalidate the S1 cache key"
        );
    }

    #[test]
    fn hash_files_unreadable_file_returns_extraction_error() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("does-not-exist.rs");

        let result = hash_files(&[nonexistent]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("incremental hashing failed"),
            "error message must signal incremental hashing failure: {}",
            err
        );
    }

    #[test]
    fn hash_files_iteration_order_is_deterministic() {
        // BTreeMap keyed by path means callers can rely on a stable
        // order for snapshot-style assertions and progress emission.
        let dir = TempDir::new().unwrap();
        let a = write_fixture(&dir, "a.rs", b"a");
        let b = write_fixture(&dir, "b.rs", b"b");
        let c = write_fixture(&dir, "c.rs", b"c");

        let map = hash_files(&[c.clone(), a.clone(), b.clone()]).expect("hash");
        let keys: Vec<&String> = map.keys().collect();
        assert_eq!(keys.len(), 3);
        assert!(keys[0] < keys[1]);
        assert!(keys[1] < keys[2]);
    }
}
