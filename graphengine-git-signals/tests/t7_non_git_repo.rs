//! T7 §6.2 acceptance criterion #4.
//!
//! A directory that is not a git working tree must classify as
//! [`RepoShape::NonGit`], emit
//! `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1`, and produce an empty
//! [`GitSignalReport::per_file`] map. No panic, no error.

use std::fs;

use graphengine_git_signals::{
    GitSignalExtractor, HistoryWindow, RepoShape, CAVEAT_LAYER0_UNSUPPORTED_VCS_V1,
};
use tempfile::TempDir;

#[test]
fn non_git_directory_emits_caveat_and_no_signals() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();

    fs::write(root.join("a.txt"), "hello\n").unwrap();
    fs::write(root.join("b.txt"), "world\n").unwrap();

    let extractor = GitSignalExtractor::open(&root).expect("open plain dir");
    assert_eq!(extractor.repo_shape(), RepoShape::NonGit);

    let report = extractor
        .extract(&HistoryWindow::default_ci())
        .expect("extract on non-git dir returns placeholder report");

    assert_eq!(report.repository_shape, RepoShape::NonGit);
    assert!(report.per_file.is_empty());
    assert!(report.co_change_clusters.is_empty());
    assert!(report
        .integrity_caveats
        .iter()
        .any(|c| c == CAVEAT_LAYER0_UNSUPPORTED_VCS_V1));
    assert_eq!(report.commits_walked, 0);
}
