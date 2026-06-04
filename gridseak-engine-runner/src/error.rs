//! Typed error surface for [`crate::run_pipeline`].
//!
//! Why a typed enum and not `anyhow::Error`: the polyglot test gate in
//! Stage 0 needs to assert that "missing parser binary fails loudly with
//! a specific shape," which a typed enum makes trivial
//! (`matches!(err, RunError::BinaryMissing { .. })`). Equally important,
//! consumers map the same `RunError` variants to different presentations
//! — CLI prints to stderr, desktop emits a Tauri event with a notice
//! UI — so the variant set is part of the public contract, not just a
//! debugging aid.
//!
//! `#[non_exhaustive]` is deliberate. Adding variants in later stages
//! (e.g. a dedicated `LicenseRequired` variant once enforcement lands)
//! must not silently break consumers' `match` arms.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RunError {
    #[error("required {which} binary not found at {path}")]
    BinaryMissing { which: BinaryKind, path: PathBuf },

    #[error("language registry load failed: {0}")]
    LanguageRegistry(String),

    #[error(
        "no parseable languages remain after filtering discovery-only entries; \
         pass at least one host language (e.g. Apex if you are scanning a \
         Salesforce org with only Visualforce pages)"
    )]
    NoParseableLanguages,

    #[error(
        "parser failed for language `{language}` with exit code {exit_code}; \
         stderr log: {stderr_log_path}\n--- tail ---\n{stderr_tail}"
    )]
    ParserFailed {
        language: String,
        exit_code: i32,
        stderr_log_path: PathBuf,
        stderr_tail: String,
    },

    #[error(
        "analyzer failed with exit code {exit_code}; \
         stderr log: {stderr_log_path}\n--- tail ---\n{stderr_tail}"
    )]
    AnalyzerFailed {
        exit_code: i32,
        stderr_log_path: PathBuf,
        stderr_tail: String,
    },

    #[error("scan cancelled")]
    Cancelled,

    #[error("could not deserialize report JSON: {0}")]
    ReportDeserialize(String),

    #[error(
        "{which} binary at {path} reports version `{actual}`, runner expected `{expected}`. \
         The CLI and its sidecar engine binaries must be built from the same workspace \
         (`scripts/install/build-cli-release.sh`) and installed together \
         (`scripts/install/install.sh`); a stale sidecar will silently produce wrong \
         results in shadow-mode scans. Re-run the install script or set the \
         per-binary override env var (e.g. `GE_ANALYZE_BIN`) to a fresh build."
    )]
    BinaryVersionMismatch {
        which: BinaryKind,
        expected: String,
        actual: String,
        path: PathBuf,
    },

    #[error(
        "could not read version from {which} binary at {path}: {detail}. \
         The runner depends on `{which} --version` returning the clap default \
         `name <version>` line on stdout to detect drift between the CLI and its \
         sidecars. If you are using a third-party drop-in binary, set the \
         override env var (e.g. `GE_ANALYZE_BIN`) and rebuild from this workspace."
    )]
    BinaryVersionUnreadable {
        which: BinaryKind,
        path: PathBuf,
        detail: String,
    },

    #[error(transparent)]
    Io(std::io::Error),
}

/// Which engine binary a [`RunError::BinaryMissing`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryKind {
    Parser,
    Analyzer,
}

impl std::fmt::Display for BinaryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinaryKind::Parser => f.write_str("graphengine-parsing"),
            BinaryKind::Analyzer => f.write_str("ge-analyze"),
        }
    }
}
