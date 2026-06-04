//! Regression test for the directory-exclusion fix discovered
//! during the Gate 2 `gridseak-self` correlation measurement.
//!
//! Before the fix, `collect_changed_files` populated
//! `GitSignalReport::per_file` with intermediate tree (directory)
//! entries in addition to leaf blob entries. The symptom on a
//! real repository was the top-10 hotspots being all directories
//! (`graphengine-parsing`, `graphengine-parsing/src`, ...) with
//! inflated `change_frequency` numbers — because a directory's
//! oid changes every time any descendant file's oid changes.
//!
//! This test builds a nested-directory fixture and verifies that
//! the extractor's `per_file` map contains **only** the leaf file
//! paths, not the enclosing directory paths.

mod common;

use std::fs;

use graphengine_git_signals::{GitSignalExtractor, HistoryWindow};

#[test]
fn per_file_contains_only_blob_paths_not_directory_paths() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path();
    common::git_init(root);

    // Deliberately nested: a top-level dir, a mid-level dir, a
    // leaf file. Before the fix, every level would show up in
    // per_file. After the fix, only the leaf.
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::write(root.join("a/b/leaf.txt"), "first\n").unwrap();
    common::git_commit_all(root, common::ALICE, "seed", "2026-04-10 12:00:00 +0000");

    fs::write(root.join("a/b/leaf.txt"), "second\n").unwrap();
    common::git_commit_all(root, common::ALICE, "edit", "2026-04-11 12:00:00 +0000");

    let extractor = GitSignalExtractor::open(root).expect("open");
    let report = extractor
        .extract(&HistoryWindow::default_ci())
        .expect("extract");

    let keys: Vec<String> = report
        .per_file
        .keys()
        .map(|p| p.display().to_string())
        .collect();

    assert!(
        keys.iter().any(|k| k == "a/b/leaf.txt"),
        "leaf blob must be in per_file, got {:?}",
        keys
    );
    assert!(
        !keys.iter().any(|k| k == "a" || k == "a/b"),
        "intermediate directory paths must NOT be in per_file, got {:?}",
        keys
    );
    assert_eq!(
        keys.len(),
        1,
        "exactly one blob path expected, got {:?}",
        keys
    );
}
