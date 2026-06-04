//! Shared helpers for building synthetic git fixtures under a tempdir.
//!
//! The helpers deliberately shell out to `git` rather than using
//! `gix`'s object-authoring APIs. Rationale:
//!
//! - `gix`'s high-level "create this commit with these parents,
//!   these blobs, these tree entries, this signature" API is still
//!   churning as of `gix = 0.81`. Integration tests that depend on
//!   it would break on every minor bump.
//! - `git` is guaranteed available on every CI image we run on, and
//!   the fixture builder here sets `GIT_*` environment variables to
//!   pin author / committer / timestamps so the tests are
//!   deterministic.
//! - The production code path uses `gix` to *read* history, which is
//!   the thing we actually want under test — mixing `git` for the
//!   fixture and `gix` for the read keeps the two concerns cleanly
//!   separate.

use std::path::Path;
use std::process::Command;

/// Synthetic author/committer identity used by every fixture. Emails
/// are distinct so the ownership-dispersion tests can count authors
/// without relying on name-vs-email normalisation.
pub const ALICE: (&str, &str) = ("Alice", "alice@example.com");
#[allow(dead_code)] // reserved for multi-author ownership-dispersion fixtures
pub const BOB: (&str, &str) = ("Bob", "bob@example.com");
#[allow(dead_code)] // reserved for multi-author ownership-dispersion fixtures
pub const CAROL: (&str, &str) = ("Carol", "carol@example.com");

/// Initialise an empty git repository at `path`. Sets `init.defaultBranch
/// = main` so the fixture works identically on systems whose global
/// config still defaults to `master`.
pub fn git_init(path: &Path) {
    run_git(path, &["init", "--quiet", "--initial-branch", "main"]);
    run_git(path, &["config", "commit.gpgsign", "false"]);
    run_git(path, &["config", "user.email", ALICE.1]);
    run_git(path, &["config", "user.name", ALICE.0]);
}

/// Stage every change under `path` and commit as `author` with
/// `message`. Uses the `GIT_AUTHOR_DATE` / `GIT_COMMITTER_DATE` env
/// vars to pin the commit timestamp so the test is deterministic
/// regardless of wall-clock. `date_iso` must be an ISO-8601 string
/// accepted by git (e.g. `"2026-04-10 12:00:00 +0000"`).
pub fn git_commit_all(path: &Path, author: (&str, &str), message: &str, date_iso: &str) {
    run_git(path, &["add", "-A"]);
    Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            message,
            "--author",
            &format!("{} <{}>", author.0, author.1),
        ])
        .current_dir(path)
        .env("GIT_AUTHOR_DATE", date_iso)
        .env("GIT_COMMITTER_DATE", date_iso)
        .env("GIT_COMMITTER_NAME", author.0)
        .env("GIT_COMMITTER_EMAIL", author.1)
        .status()
        .expect("git commit should succeed under tempdir");
}

/// Run a git sub-command at `path`, panicking on non-zero exit. Used
/// for setup-only steps where we don't care about the output.
fn run_git(path: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(path)
        .status()
        .expect("git binary should be available on PATH for tests");
    assert!(status.success(), "git {:?} failed under {path:?}", args);
}
