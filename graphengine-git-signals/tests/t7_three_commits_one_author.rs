//! T7 §6.2 acceptance criterion #2.
//!
//! Three-commit, single-author fixture: proves the happy-path
//! [`GitSignalExtractor::extract`] flow produces the expected
//! [`FileSignals`] on a `Full`-shape repository with a trivial
//! history shape.

use std::fs;
use std::path::PathBuf;

use graphengine_git_signals::{
    GitSignalExtractor, HistoryWindow, RepoShape, CAVEAT_LAYER0_GIT_SIGNALS_V1,
};
use tempfile::TempDir;

mod common;
use common::{git_commit_all, git_init, ALICE};

/// Builds the fixture, runs an extraction, asserts every acceptance-
/// criterion field from T7 §6.2 #2 on the single touched file.
#[test]
fn single_author_ownership_is_zero_dispersion() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    git_init(&root);

    let touched = root.join("hello.rs");
    fs::write(&touched, "fn main() {}\n").unwrap();
    git_commit_all(&root, ALICE, "initial", "2026-04-10 12:00:00 +0000");

    fs::write(&touched, "fn main() { println!(); }\n").unwrap();
    git_commit_all(&root, ALICE, "body", "2026-04-11 12:00:00 +0000");

    fs::write(&touched, "fn main() { println!(\"ok\"); }\n").unwrap();
    git_commit_all(&root, ALICE, "arg", "2026-04-12 12:00:00 +0000");

    let extractor = GitSignalExtractor::open(&root).expect("open full-shape repo");
    assert_eq!(extractor.repo_shape(), RepoShape::Full);

    let report = extractor
        .extract(&HistoryWindow::default_ci())
        .expect("extract on full repo");

    assert_eq!(report.repository_shape, RepoShape::Full);
    assert!(report
        .integrity_caveats
        .iter()
        .any(|c| c == CAVEAT_LAYER0_GIT_SIGNALS_V1));
    assert_eq!(report.commits_walked, 3);

    let rel = PathBuf::from("hello.rs");
    let signals = report.per_file.get(&rel).unwrap_or_else(|| {
        panic!(
            "per_file must include hello.rs — got {:?}",
            report.per_file.keys().collect::<Vec<_>>()
        )
    });

    assert_eq!(
        signals.change_frequency, 3,
        "three commits touched hello.rs"
    );
    assert_eq!(signals.distinct_authors, 1, "all three commits are alice");
    assert!(
        (signals.ownership_dispersion).abs() < 1e-6,
        "single-author dispersion should be 0, got {}",
        signals.ownership_dispersion
    );
    assert_eq!(
        signals.confidence,
        graphengine_git_signals::Confidence::High,
        "full-shape repo must emit High confidence"
    );
}
