//! Typed error surface for [`crate::GitSignalExtractor`].
//!
//! The crate makes a deliberate choice: `Extract` errors are
//! **structured** and **recoverable**. A Layer 0 extractor failure
//! must never abort a scan — the engine gracefully falls back to
//! "no git signals available" and the [`crate::caveats`]
//! `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1` token is stamped onto the
//! report. This is the measured-fallback discipline the sprint as a
//! whole has adopted: report what happened, do not self-declare
//! authority, and do not panic.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Constructor-time error from [`crate::GitSignalExtractor::open`].
///
/// These are the only failure modes that can happen before a commit
/// walk begins. Extraction-time failures live in [`ExtractError`].
#[derive(Debug, Error)]
pub enum OpenError {
    /// The supplied path does not exist, is not a directory, or is
    /// not readable. Bubbles up as a distinct variant so the pipeline
    /// can tell "I was given a bad path" apart from "this path is
    /// fine but is not a git tree."
    #[error("repository path {path:?} is not readable: {source}")]
    PathNotReadable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The path is readable but is neither a git working tree nor a
    /// bare repository. Not fatal to the scan — the caller is
    /// expected to emit `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1` and
    /// continue with an empty report.
    #[error("directory {path:?} is not a git working tree")]
    NotAGitRepository { path: PathBuf },

    /// `gix` could open the directory but failed at a lower level
    /// (corrupted pack, unsupported ref format, permission denied on
    /// `.git/objects/`, etc.). Surfaced verbatim so operators can
    /// diagnose without a separate debug flag.
    #[error("gix failed to open repository at {path:?}: {message}")]
    GixOpenFailed { path: PathBuf, message: String },
}

/// Extraction-time error from [`crate::GitSignalExtractor::extract`].
///
/// Every variant is recoverable: the pipeline falls back to "report
/// the error, emit no per-file data for the affected cohort, stamp
/// an appropriate caveat, proceed."
#[derive(Debug, Error)]
pub enum ExtractError {
    /// The commit walk exceeded
    /// [`crate::HistoryWindow::max_wall_clock`]. Returned with the
    /// number of commits actually walked so the caller can decide
    /// whether to downgrade confidence to `Medium` (we walked most
    /// of the window) or emit no signal at all (we only got a
    /// handful of commits before the budget expired).
    #[error("history walk exceeded wall-clock budget after {commits_walked} commits")]
    WallClockExceeded { commits_walked: usize },

    /// Errors bubbled up from `gix` during revwalk / object lookup.
    /// Stringified because the `gix::revision::walk::Error` type has
    /// lifetime parameters that don't fit a stored error.
    #[error("gix walk failed: {0}")]
    WalkFailed(String),

    /// The repository has no HEAD ref (newly `git init`-ed with no
    /// commits). Different from [`OpenError::NotAGitRepository`]:
    /// this is a valid git tree, just an empty one. Treated as
    /// [`crate::RepoShape::Shallow`] with `depth: 0`.
    #[error("repository at {path:?} has no HEAD commit")]
    EmptyRepository { path: PathBuf },
}
