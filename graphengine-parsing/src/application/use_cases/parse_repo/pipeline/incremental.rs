//! Incremental-scan planner.
//!
//! Given (a) the list of discovered source files, (b) their freshly
//! computed content hashes, and (c) the previous scan's `file_cache`
//! rows, produce an [`IncrementalPlan`] that partitions discovered
//! files into:
//!
//! * `unchanged` — file's current blake3 matches the cached row's
//!   `content_hash`. The orchestrator will reuse the cached slice and
//!   skip re-extraction.
//! * `changed` — file is absent from the cache, present but the hash
//!   differs, or present but the stored language differs from the
//!   current scan language. The orchestrator will re-extract these.
//!
//! Files in the cache that are NOT in the discovery output represent
//! deleted files. They are reported via `removed_paths` so the
//! orchestrator can prune them from the cache at end-of-scan; they
//! do not appear in either the `unchanged` or `changed` set because
//! they should not be carried into the merged `SyntaxResults`.
//!
//! See `docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md` §4.5–§4.7.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::file_hashing::FileHash;
use crate::infrastructure::storage::FileCacheRow;

/// Result of partitioning discovered files against a prior cache.
///
/// `unchanged` and `changed` together cover every discovered file
/// exactly once. `removed_paths` lists cache entries with no
/// matching discovery — handled separately by the end-of-scan prune.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncrementalPlan {
    /// Files whose current hash matches a cached row. The
    /// orchestrator reuses each row's `payload_json` slice and skips
    /// re-extraction. Keyed by file_path so the caller can look the
    /// matching cache row back up.
    pub unchanged: Vec<String>,
    /// Files the orchestrator must extract from scratch. Stored as
    /// `PathBuf` so they can flow straight into
    /// `SyntaxExtractor::extract` without an extra conversion.
    pub changed: Vec<PathBuf>,
    /// Cache rows with no matching discovery entry — i.e. files that
    /// existed at the last scan and have since been deleted.
    /// Reported here so the orchestrator can prune them.
    pub removed_paths: Vec<String>,
    /// Total number of files the orchestrator will touch this scan
    /// (`unchanged.len() + changed.len()`). Convenience for cache-
    /// stats progress emission.
    pub total_count: usize,
}

impl IncrementalPlan {
    /// True if this scan can skip re-extraction entirely (no changed
    /// files). The orchestrator still re-runs resolution and graph
    /// building because cached `references` may bind to nodes in
    /// other (unchanged) files whose IDs survived.
    pub fn is_fully_cached(&self) -> bool {
        self.changed.is_empty() && !self.unchanged.is_empty()
    }

    /// True if this scan must re-extract everything (no cache hits).
    /// Equivalent to a cold-cache scan. The orchestrator should also
    /// detect this case to skip the per-file slice merge step.
    pub fn is_cold(&self) -> bool {
        self.unchanged.is_empty()
    }
}

/// Compute the plan. Pure function; no I/O.
///
/// `discovered_files` is the orchestrator's post-discovery list of
/// `PathBuf`s. `current_hashes` is the matching `BTreeMap` produced
/// by [`super::file_hashing::hash_files`]. `cache` is the rows loaded
/// from the previous parse DB via
/// [`crate::infrastructure::storage::FileCacheRepository::load_all`].
///
/// `current_language` is the scan's language identifier (e.g. `"rust"`).
/// A cached row whose `language` field differs from this is treated
/// as changed because the extractor behaviour for the same file path
/// depends on the language. This catches the "user changed their
/// language config but the file still exists" edge case.
///
/// File-path keys must match byte-for-byte across the three inputs.
/// `file_discovery` canonicalises paths, [`hash_files`] writes the
/// canonical path string, and the orchestrator writes the same string
/// to `file_cache.file_path` on persist. The pipeline is responsible
/// for keeping these three sites in sync; this function performs no
/// normalisation of its own.
pub fn compute_plan(
    discovered_files: &[PathBuf],
    current_hashes: &BTreeMap<String, FileHash>,
    cache: &BTreeMap<String, FileCacheRow>,
    current_language: &str,
) -> IncrementalPlan {
    let mut unchanged: Vec<String> = Vec::new();
    let mut changed: Vec<PathBuf> = Vec::new();

    for path in discovered_files {
        let key = path.to_string_lossy().into_owned();
        let current = match current_hashes.get(&key) {
            Some(h) => h,
            None => {
                // Hash missing for a discovered file — the hashing
                // pass dropped it (file disappeared mid-scan, etc.).
                // Treat as changed so it gets re-extracted; the
                // extractor will surface a real error if appropriate.
                changed.push(path.clone());
                continue;
            }
        };

        match cache.get(&key) {
            Some(row)
                if row.content_hash == current.content_hash && row.language == current_language =>
            {
                unchanged.push(key);
            }
            _ => changed.push(path.clone()),
        }
    }

    let discovery_set: std::collections::HashSet<String> = discovered_files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    // Removed-file detection is **language-scoped**. The file_cache
    // table is shared across all languages in a persistent parse DB
    // (S1-ε), so a Rust pass loading the full cache would otherwise
    // see Python rows as "missing from discovery" and try to prune
    // them — clobbering the Python pass's own rows. Only consider
    // cache entries that belong to *this* language as candidates
    // for removal.
    let mut removed_paths: Vec<String> = cache
        .iter()
        .filter(|(cached_path, row)| {
            row.language == current_language && !discovery_set.contains(*cached_path)
        })
        .map(|(path, _)| path.clone())
        .collect();
    removed_paths.sort();

    let total_count = unchanged.len() + changed.len();

    IncrementalPlan {
        unchanged,
        changed,
        removed_paths,
        total_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fh(path: &str, hash: &str) -> FileHash {
        FileHash {
            file_path: path.to_string(),
            content_hash: hash.to_string(),
        }
    }

    fn fcr(path: &str, hash: &str, language: &str) -> FileCacheRow {
        FileCacheRow {
            file_path: path.to_string(),
            content_hash: hash.to_string(),
            language: language.to_string(),
            payload_json: "{}".to_string(),
            cached_at: "2026-05-25T00:00:00+00:00".to_string(),
        }
    }

    fn hashes(items: &[(&str, &str)]) -> BTreeMap<String, FileHash> {
        items
            .iter()
            .map(|(p, h)| (p.to_string(), fh(p, h)))
            .collect()
    }

    fn cache(items: &[(&str, &str)]) -> BTreeMap<String, FileCacheRow> {
        items
            .iter()
            .map(|(p, h)| (p.to_string(), fcr(p, h, "rust")))
            .collect()
    }

    #[test]
    fn empty_discovery_and_empty_cache_yields_empty_plan() {
        let plan = compute_plan(&[], &BTreeMap::new(), &BTreeMap::new(), "rust");
        assert!(plan.unchanged.is_empty());
        assert!(plan.changed.is_empty());
        assert!(plan.removed_paths.is_empty());
        assert_eq!(plan.total_count, 0);
        assert!(!plan.is_fully_cached());
        assert!(plan.is_cold());
    }

    #[test]
    fn cold_cache_marks_every_discovered_file_as_changed() {
        let files = vec![PathBuf::from("/a.rs"), PathBuf::from("/b.rs")];
        let h = hashes(&[("/a.rs", "h-a"), ("/b.rs", "h-b")]);
        let plan = compute_plan(&files, &h, &BTreeMap::new(), "rust");

        assert!(plan.unchanged.is_empty());
        assert_eq!(plan.changed.len(), 2);
        assert!(plan.is_cold());
        assert!(!plan.is_fully_cached());
    }

    #[test]
    fn matching_hash_and_language_marks_file_as_unchanged() {
        let files = vec![PathBuf::from("/a.rs")];
        let h = hashes(&[("/a.rs", "h-a")]);
        let c = cache(&[("/a.rs", "h-a")]);
        let plan = compute_plan(&files, &h, &c, "rust");

        assert_eq!(plan.unchanged, vec!["/a.rs"]);
        assert!(plan.changed.is_empty());
        assert!(plan.is_fully_cached());
    }

    #[test]
    fn hash_mismatch_marks_file_as_changed() {
        let files = vec![PathBuf::from("/a.rs")];
        let h = hashes(&[("/a.rs", "h-new")]);
        let c = cache(&[("/a.rs", "h-old")]);
        let plan = compute_plan(&files, &h, &c, "rust");

        assert!(plan.unchanged.is_empty());
        assert_eq!(plan.changed, vec![PathBuf::from("/a.rs")]);
        assert!(!plan.is_fully_cached());
    }

    #[test]
    fn language_mismatch_marks_file_as_changed_even_when_hash_matches() {
        let files = vec![PathBuf::from("/a.rs")];
        let h = hashes(&[("/a.rs", "h-a")]);
        let mut c: BTreeMap<String, FileCacheRow> = BTreeMap::new();
        c.insert("/a.rs".to_string(), fcr("/a.rs", "h-a", "javascript"));

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.changed, vec![PathBuf::from("/a.rs")]);
        assert!(plan.unchanged.is_empty());
    }

    #[test]
    fn mixed_changed_and_unchanged_partitions_correctly() {
        let files = vec![
            PathBuf::from("/a.rs"),
            PathBuf::from("/b.rs"),
            PathBuf::from("/c.rs"),
        ];
        let h = hashes(&[("/a.rs", "h-a"), ("/b.rs", "h-b-new"), ("/c.rs", "h-c")]);
        let c = cache(&[("/a.rs", "h-a"), ("/b.rs", "h-b-old"), ("/c.rs", "h-c")]);

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.unchanged.len(), 2);
        assert!(plan.unchanged.contains(&"/a.rs".to_string()));
        assert!(plan.unchanged.contains(&"/c.rs".to_string()));
        assert_eq!(plan.changed, vec![PathBuf::from("/b.rs")]);
        assert!(!plan.is_cold());
        assert!(!plan.is_fully_cached());
    }

    #[test]
    fn cache_entry_with_no_discovery_match_appears_in_removed_paths() {
        let files = vec![PathBuf::from("/a.rs")];
        let h = hashes(&[("/a.rs", "h-a")]);
        let c = cache(&[("/a.rs", "h-a"), ("/deleted.rs", "h-d")]);

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.unchanged, vec!["/a.rs"]);
        assert_eq!(plan.removed_paths, vec!["/deleted.rs"]);
    }

    #[test]
    fn discovered_file_with_missing_hash_falls_through_to_changed() {
        // hash_files dropped this file (mid-scan deletion, etc.).
        // The planner defensively marks it as changed; the
        // re-extraction attempt will surface a real error if the
        // file truly cannot be read.
        let files = vec![PathBuf::from("/a.rs")];
        let h = BTreeMap::new();
        let c = cache(&[("/a.rs", "h-a")]);

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.changed, vec![PathBuf::from("/a.rs")]);
        assert!(plan.unchanged.is_empty());
    }

    #[test]
    fn removed_paths_iteration_order_is_deterministic() {
        let files = vec![PathBuf::from("/a.rs")];
        let h = hashes(&[("/a.rs", "h-a")]);
        let c = cache(&[
            ("/a.rs", "h-a"),
            ("/z.rs", "h-z"),
            ("/m.rs", "h-m"),
            ("/b.rs", "h-b"),
        ]);

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.removed_paths, vec!["/b.rs", "/m.rs", "/z.rs"]);
    }

    #[test]
    fn other_language_cache_rows_are_invisible_to_this_pass() {
        // S1-ε persistent-DB scenario: the cache table holds rows
        // from a prior Python pass (`foo.py`) and a prior Rust pass
        // (`foo.rs`). When the Rust pass re-runs and discovers only
        // `foo.rs`, the planner must NOT report `foo.py` as removed
        // (that would let the orchestrator prune another language's
        // rows).
        let files = vec![PathBuf::from("/foo.rs")];
        let h = hashes(&[("/foo.rs", "h-rs")]);
        let mut c: BTreeMap<String, FileCacheRow> = BTreeMap::new();
        c.insert("/foo.rs".to_string(), fcr("/foo.rs", "h-rs", "rust"));
        c.insert("/foo.py".to_string(), fcr("/foo.py", "h-py", "python"));

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.unchanged, vec!["/foo.rs"]);
        assert!(
            plan.removed_paths.is_empty(),
            "python cache row must not appear in rust pass's removed_paths; got {:?}",
            plan.removed_paths
        );
    }

    #[test]
    fn same_language_deleted_file_does_appear_in_removed_paths() {
        // Counterpoint to the test above: when the cache row IS the
        // current language and discovery doesn't see it, we DO want
        // it pruned.
        let files = vec![PathBuf::from("/foo.rs")];
        let h = hashes(&[("/foo.rs", "h-rs")]);
        let mut c: BTreeMap<String, FileCacheRow> = BTreeMap::new();
        c.insert("/foo.rs".to_string(), fcr("/foo.rs", "h-rs", "rust"));
        c.insert("/gone.rs".to_string(), fcr("/gone.rs", "h-gone", "rust"));

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.removed_paths, vec!["/gone.rs"]);
    }

    #[test]
    fn total_count_sums_unchanged_and_changed() {
        let files = vec![PathBuf::from("/a.rs"), PathBuf::from("/b.rs")];
        let h = hashes(&[("/a.rs", "h-a"), ("/b.rs", "h-b-new")]);
        let c = cache(&[("/a.rs", "h-a"), ("/b.rs", "h-b-old")]);

        let plan = compute_plan(&files, &h, &c, "rust");
        assert_eq!(plan.total_count, 2);
        assert_eq!(plan.unchanged.len() + plan.changed.len(), 2);
    }

    #[test]
    fn fully_cached_plan_correctly_distinguishes_from_cold() {
        let files = vec![PathBuf::from("/a.rs")];
        let h = hashes(&[("/a.rs", "h-a")]);
        let c = cache(&[("/a.rs", "h-a")]);

        let plan = compute_plan(&files, &h, &c, "rust");
        assert!(plan.is_fully_cached());
        assert!(!plan.is_cold());
    }
}
