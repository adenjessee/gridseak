//! T7 §6.2 acceptance criterion #5 (co-change cluster emission).
//!
//! Fixture where files `a.rs` and `b.rs` are edited in the same
//! commit at least three times (the `CO_CHANGE_MIN_FULL` threshold
//! inside the extractor) produces a two-file cluster with the
//! expected `co_commit_count`.

use std::fs;
use std::path::PathBuf;

use graphengine_git_signals::{Confidence, GitSignalExtractor, HistoryWindow, RepoShape};
use tempfile::TempDir;

mod common;
use common::{git_commit_all, git_init, ALICE};

#[test]
fn co_change_two_file_pair_emits_cluster() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    git_init(&root);

    let a = root.join("a.rs");
    let b = root.join("b.rs");

    for (i, date) in [
        "2026-04-10 12:00:00 +0000",
        "2026-04-11 12:00:00 +0000",
        "2026-04-12 12:00:00 +0000",
        "2026-04-13 12:00:00 +0000",
        "2026-04-14 12:00:00 +0000",
    ]
    .iter()
    .enumerate()
    {
        fs::write(&a, format!("// a v{i}\n")).unwrap();
        fs::write(&b, format!("// b v{i}\n")).unwrap();
        git_commit_all(&root, ALICE, &format!("rev {i}"), date);
    }

    let extractor = GitSignalExtractor::open(&root).expect("open");
    assert_eq!(extractor.repo_shape(), RepoShape::Full);

    let report = extractor
        .extract(&HistoryWindow::default_ci())
        .expect("extract");

    assert_eq!(
        report.commits_walked, 5,
        "five commits were authored; walker should visit all of them"
    );

    let matching: Vec<_> = report
        .co_change_clusters
        .iter()
        .filter(|c| c.files == vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")])
        .collect();

    assert_eq!(
        matching.len(),
        1,
        "exactly one (a.rs, b.rs) cluster should be emitted; clusters={:?}",
        report.co_change_clusters
    );
    let cluster = matching[0];
    assert_eq!(cluster.co_commit_count, 5);
    assert_eq!(cluster.confidence, Confidence::High);
}
