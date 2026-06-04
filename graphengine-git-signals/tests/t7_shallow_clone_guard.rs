//! T7 §6.2 acceptance criterion #3 and the single load-bearing
//! invariant the crate exists to protect.
//!
//! A shallow-clone fixture (`.git/shallow` is populated by hand)
//! must downgrade every [`FileSignals::confidence`] to
//! [`Confidence::Low`] regardless of how strong the raw numeric
//! signal looks. This test failing is the named rollback criterion
//! for T7 (see T7 §7).

use std::fs;

use graphengine_git_signals::{
    Confidence, GitSignalExtractor, HistoryWindow, RepoShape, CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1,
};
use tempfile::TempDir;

mod common;
use common::{git_commit_all, git_init, ALICE};

/// Build a fixture with real commits, then forge `.git/shallow` so
/// the repository *reports* itself as shallow. We don't use a
/// `git clone --depth 1` because the point of the test is the
/// shape-detection path, not the clone mechanics.
#[test]
fn shallow_clone_downgrades_all_signals_to_low() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    git_init(&root);

    let touched = root.join("file.rs");
    fs::write(&touched, "a\n").unwrap();
    git_commit_all(&root, ALICE, "c1", "2026-04-10 12:00:00 +0000");
    fs::write(&touched, "b\n").unwrap();
    git_commit_all(&root, ALICE, "c2", "2026-04-11 12:00:00 +0000");
    fs::write(&touched, "c\n").unwrap();
    git_commit_all(&root, ALICE, "c3", "2026-04-12 12:00:00 +0000");

    let shallow_file = root.join(".git").join("shallow");
    fs::write(&shallow_file, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n").unwrap();

    let extractor = GitSignalExtractor::open(&root).expect("open forged-shallow repo");
    assert_eq!(
        extractor.repo_shape(),
        RepoShape::Shallow { depth: Some(1) }
    );

    let report = extractor
        .extract(&HistoryWindow::default_ci())
        .expect("extract should succeed on shallow repo");

    assert_eq!(
        report.repository_shape,
        RepoShape::Shallow { depth: Some(1) }
    );
    assert!(
        report
            .integrity_caveats
            .iter()
            .any(|c| c == CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1),
        "insufficient-history caveat must be stamped on shallow reports: got {:?}",
        report.integrity_caveats
    );

    assert!(
        !report.per_file.is_empty(),
        "shallow-clone guard still emits numeric per-file signals; it only downgrades confidence"
    );
    for (path, signals) in &report.per_file {
        assert_eq!(
            signals.confidence,
            Confidence::Low,
            "file {:?} must carry Confidence::Low on shallow repo, got {:?}",
            path,
            signals.confidence,
        );
    }
}
