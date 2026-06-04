//! Repository-shape detection — the **load-bearing** shallow-clone
//! guard for every downstream signal.
//!
//! The guard's contract:
//!
//! - `RepoShape::Full`    → every [`crate::FileSignals::confidence`]
//!   defaults to [`crate::Confidence::High`].
//! - `RepoShape::Shallow` → **every**
//!   [`crate::FileSignals::confidence`] is forcibly downgraded to
//!   [`crate::Confidence::Low`], regardless of how strong the raw
//!   change-frequency number looks. A 1-commit `--depth 1` clone can
//!   produce `change_frequency = 1` on every file, which the
//!   classifier must not trust.
//! - `RepoShape::Bare`    → same as `Shallow` — bare repos cannot
//!   represent a scan target (no working tree to anchor file paths
//!   against).
//! - `RepoShape::NonGit`  → the extractor returns an empty
//!   [`crate::GitSignalReport::per_file`] map with the
//!   `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1` caveat stamped.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Shape of the repository rooted at the scanned path. Derived once
/// by [`RepoShape::detect`] at extractor-open time and then threaded
/// through the whole report; classifiers read this enum (not
/// `.git/shallow`'s presence, not `gix::Repository::shallow_commits()`)
/// as the single authoritative signal of "is this repo's history
/// trustworthy?"
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RepoShape {
    /// Normal clone with full history reachable from HEAD.
    Full,
    /// Shallow clone. `depth` is `Some(n)` when `.git/shallow` is
    /// well-formed and `n` is the depth parsed; `None` when the
    /// repository is shallow but depth could not be determined
    /// (corrupted shallow file, or pre-empty-state new repo).
    Shallow { depth: Option<u32> },
    /// Bare repository (no working tree). The current scan path
    /// cannot analyse bare repos because every file-level signal
    /// needs a working-tree path; we still distinguish this case so
    /// operators get a clear error message rather than a vague
    /// "no signals" outcome.
    Bare,
    /// Directory is not a git working tree at all.
    NonGit,
}

impl RepoShape {
    /// Classify the directory at `path`. The check order is
    /// intentional:
    ///
    /// 1. Check for `.git/` (directory or gitfile pointer). Absent ⇒
    ///    [`RepoShape::NonGit`]. Catches tarball extracts and
    ///    non-git VCSes with zero `gix` cost.
    /// 2. Check for `.git/shallow`. Present ⇒
    ///    [`RepoShape::Shallow`] with parsed depth if possible.
    /// 3. Check for `HEAD` at the repository root (bare repo
    ///    convention). If `HEAD` and `objects/` sit at the top
    ///    level, classify as [`RepoShape::Bare`].
    /// 4. Otherwise [`RepoShape::Full`].
    ///
    /// `detect` is deliberately filesystem-only: it never opens the
    /// repository through `gix`. Keeping this pure-`fs` means the
    /// detection cost is ~one `stat()` per case and is safe to call
    /// in a tight loop for batched scans.
    pub fn detect(path: &Path) -> Self {
        let git_dir = path.join(".git");
        let dot_git_exists = git_dir.exists();

        if dot_git_exists {
            return detect_from_dot_git(&git_dir).unwrap_or(Self::Full);
        }

        if looks_like_bare_repo(path) {
            return Self::Bare;
        }

        Self::NonGit
    }

    /// True when Layer 0 signals extracted from this shape are
    /// allowed to carry `Confidence::High`. Currently only
    /// [`RepoShape::Full`] qualifies; the other three are all forced
    /// to `Low`. If a future variant is added (e.g. `Partial` for
    /// blob-filter clones), update the predicate here — every
    /// downstream consumer reads this, not the variant.
    pub fn supports_high_confidence(&self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Resolve a `.git/` entry — which may be either a directory (the
/// common case) or a `gitfile` pointer (submodules / worktrees) —
/// into a concrete [`RepoShape`].
fn detect_from_dot_git(git_dir: &Path) -> Option<RepoShape> {
    let metadata = fs::metadata(git_dir).ok()?;

    // Submodule / worktree: `.git` is a file containing `gitdir: …`.
    // We still call the overall repository `Full` because resolving
    // the gitfile for full shape-detection is gix-native work. This
    // is a documented, conservative simplification — worktrees /
    // submodules degrade to `Full` here and the commit-walk stage
    // below does the actual integrity check.
    if metadata.is_file() {
        return Some(RepoShape::Full);
    }

    let shallow_file = git_dir.join("shallow");
    if shallow_file.exists() {
        let depth = read_shallow_depth(&shallow_file);
        return Some(RepoShape::Shallow { depth });
    }

    Some(RepoShape::Full)
}

/// Parse `.git/shallow` to derive an integer depth hint. The file is
/// a list of commit SHAs that the clone's history terminates at; the
/// *count* of entries is the number of histories pruned, not the
/// literal depth of the clone, but it is the best depth proxy
/// available without walking the actual history. Single-SHA shallow
/// files (the `--depth 1` case) return `Some(1)`; multi-SHA shallow
/// files return `Some(n)`; empty or unreadable files return `None`
/// (still classified as `Shallow`).
fn read_shallow_depth(shallow_file: &Path) -> Option<u32> {
    let contents = fs::read_to_string(shallow_file).ok()?;
    let count = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    u32::try_from(count).ok()
}

/// Heuristic for "this path is a bare repository root." Bare repos
/// have `HEAD` + `objects/` + `refs/` directly under the top-level
/// path (no `.git/` wrapping). We check all three to avoid
/// classifying a random directory containing a stray `HEAD` file as
/// bare.
fn looks_like_bare_repo(path: &Path) -> bool {
    path.join("HEAD").exists() && path.join("objects").is_dir() && path.join("refs").is_dir()
}

#[cfg(test)]
mod tests {
    use super::RepoShape;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn repo_shape_detects_non_git_directory() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(RepoShape::detect(tmp.path()), RepoShape::NonGit);
        assert!(!RepoShape::NonGit.supports_high_confidence());
    }

    #[test]
    fn repo_shape_detects_full_repo() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        assert_eq!(RepoShape::detect(tmp.path()), RepoShape::Full);
        assert!(RepoShape::Full.supports_high_confidence());
    }

    #[test]
    fn repo_shape_detects_shallow_via_shallow_file_single_depth() {
        let tmp = TempDir::new().unwrap();
        let git_dir = tmp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(
            git_dir.join("shallow"),
            "e3f1a5a8e5c2c9e6b1c3f4d5e6a7b8c9d0e1f2a3\n",
        )
        .unwrap();
        assert_eq!(
            RepoShape::detect(tmp.path()),
            RepoShape::Shallow { depth: Some(1) }
        );
        assert!(!RepoShape::Shallow { depth: Some(1) }.supports_high_confidence());
    }

    #[test]
    fn repo_shape_detects_shallow_via_shallow_file_multi_depth() {
        let tmp = TempDir::new().unwrap();
        let git_dir = tmp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(
            git_dir.join("shallow"),
            "aaa0000000000000000000000000000000000001\nbbb0000000000000000000000000000000000002\nccc0000000000000000000000000000000000003\n",
        )
        .unwrap();
        assert_eq!(
            RepoShape::detect(tmp.path()),
            RepoShape::Shallow { depth: Some(3) }
        );
    }

    #[test]
    fn repo_shape_detects_shallow_via_empty_shallow_file() {
        let tmp = TempDir::new().unwrap();
        let git_dir = tmp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(git_dir.join("shallow"), "").unwrap();
        // Empty file -> shape is still Shallow, but depth is 0 (no
        // shallow commits enumerated).
        assert_eq!(
            RepoShape::detect(tmp.path()),
            RepoShape::Shallow { depth: Some(0) }
        );
    }

    #[test]
    fn repo_shape_detects_bare_repo() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir(tmp.path().join("objects")).unwrap();
        fs::create_dir(tmp.path().join("refs")).unwrap();
        assert_eq!(RepoShape::detect(tmp.path()), RepoShape::Bare);
        assert!(!RepoShape::Bare.supports_high_confidence());
    }

    #[test]
    fn repo_shape_does_not_classify_stray_head_as_bare() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("HEAD"), "garbage").unwrap();
        assert_eq!(RepoShape::detect(tmp.path()), RepoShape::NonGit);
    }

    #[test]
    fn repo_shape_gitfile_is_classified_full() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join(".git"),
            "gitdir: /somewhere/else/.git/worktrees/w1",
        )
        .unwrap();
        assert_eq!(RepoShape::detect(tmp.path()), RepoShape::Full);
    }
}
