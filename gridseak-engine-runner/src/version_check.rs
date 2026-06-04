//! Sidecar version drift detection.
//!
//! # Why this exists
//!
//! `gridseak` (the CLI) and its sidecar binaries (`graphengine-parsing`,
//! `ge-analyze`) are three independently linked executables built from
//! the same workspace. Installation copies all three into the
//! shadow-mode prefix (`~/.gridseak/share/<timestamp>/`), and `gridseak`
//! execs the other two during every scan. If a user upgrades the CLI
//! without rebuilding the sidecars (e.g. dropped a new `gridseak`
//! binary into `~/.gridseak/bin` from a manual `cargo build`), the
//! runner will still happily exec the *old* sidecar — and silently
//! produce wrong results when the new CLI relies on flags or contracts
//! the old sidecar does not understand.
//!
//! This module detects that drift early by spawning each sidecar with
//! `--version` and matching the reported version against the runner's
//! own [`env!("CARGO_PKG_VERSION")`]. The runner is in the same
//! workspace as the sidecars, all three crates inherit
//! `workspace.package.version` from the root `Cargo.toml`, so equality
//! is the right invariant: any disagreement is a build artefact, not a
//! supported configuration.
//!
//! # Failure shape
//!
//! Drift returns [`RunError::BinaryVersionMismatch`]; a binary that
//! refuses to print a version (older builds of `ge-analyze` literally
//! did not accept `--version` — that was the originating bug here)
//! returns [`RunError::BinaryVersionUnreadable`]. Both variants carry
//! the binary path so the CLI surface can suggest exactly which
//! binary to refresh.
//!
//! # What this module does **not** do
//!
//! It does not enforce a SemVer relationship between versions ("major
//! must match, minor may differ"), because the workspace version is a
//! single value — drift is binary equality, not range matching.
//! Should the engine versions ever diverge from `gridseak`'s version
//! (e.g. an analyzer-only point release), this module is the place to
//! generalise to `is_compatible(runner, sidecar)`.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::error::{BinaryKind, RunError};

/// The version every sidecar must report to be considered consistent
/// with this runner. Derived from the workspace `package.version` via
/// `env!("CARGO_PKG_VERSION")` at compile time.
pub const EXPECTED_SIDECAR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Spawn `bin --version`, parse the output, and confirm it equals
/// [`EXPECTED_SIDECAR_VERSION`].
///
/// On any non-zero exit, missing-version-flag error, or parse failure,
/// returns [`RunError::BinaryVersionUnreadable`]. On a successful read
/// that disagrees with the expected value, returns
/// [`RunError::BinaryVersionMismatch`].
pub async fn check_binary_version(which: BinaryKind, bin: &Path) -> Result<(), RunError> {
    let actual = read_version(which, bin).await?;
    if actual != EXPECTED_SIDECAR_VERSION {
        return Err(RunError::BinaryVersionMismatch {
            which,
            expected: EXPECTED_SIDECAR_VERSION.to_string(),
            actual,
            path: bin.to_path_buf(),
        });
    }
    Ok(())
}

/// Spawn `bin --version` and return just the version token. Public so
/// `gridseak doctor` can render the actual / expected pair without
/// going through the runner's pass/fail boolean.
pub async fn read_version(which: BinaryKind, bin: &Path) -> Result<String, RunError> {
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .await
        .map_err(|err| RunError::BinaryVersionUnreadable {
            which,
            path: bin.to_path_buf(),
            detail: format!("spawn failed: {err}"),
        })?;
    if !output.status.success() {
        return Err(RunError::BinaryVersionUnreadable {
            which,
            path: bin.to_path_buf(),
            detail: format!(
                "`{} --version` exited {} (likely a pre-0.1 sidecar that does not accept `--version`); stderr: {}",
                bin.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    parse_version_line(&String::from_utf8_lossy(&output.stdout)).map_err(|detail| {
        RunError::BinaryVersionUnreadable {
            which,
            path: bin.to_path_buf(),
            detail,
        }
    })
}

/// Parse the clap-default `--version` output, which is a single line
/// of the form `<name> <version>` (e.g. `ge-analyze 0.1.0`). Returns
/// the version token only.
///
/// Tolerates trailing whitespace and an optional newline, since both
/// `Output::stdout` and a piped shell command can produce either.
/// Anything else returns a descriptive parse error so the caller's
/// `BinaryVersionUnreadable` says *why* the line was rejected.
fn parse_version_line(stdout: &str) -> Result<String, String> {
    let line = stdout.lines().next().ok_or_else(|| {
        format!("`--version` produced empty stdout; expected `<name> <version>`, got {stdout:?}")
    })?;
    let mut parts = line.split_whitespace();
    let _name = parts
        .next()
        .ok_or_else(|| format!("could not extract binary name from `--version` line: {line:?}"))?;
    let version = parts.next().ok_or_else(|| {
        format!(
            "could not extract version token from `--version` line: {line:?} (expected `<name> <version>`)"
        )
    })?;
    Ok(version.to_string())
}

/// Per-binary version probe result for `gridseak doctor`. Captures
/// either the version string actually reported by the binary, or the
/// reason the probe failed — kept on one type so the doctor UI can
/// render a single table row per binary without branching on
/// `Result` arms.
#[derive(Debug, Clone)]
pub struct VersionProbe {
    pub which: BinaryKind,
    pub path: PathBuf,
    pub expected: String,
    pub outcome: VersionProbeOutcome,
}

#[derive(Debug, Clone)]
pub enum VersionProbeOutcome {
    /// Version was read and matches `EXPECTED_SIDECAR_VERSION`.
    Match { actual: String },
    /// Version was read but disagrees with `EXPECTED_SIDECAR_VERSION`.
    Mismatch { actual: String },
    /// The binary could not be probed (missing file, no `--version`
    /// support, garbled output, etc.). `detail` is the underlying
    /// reason, suitable for displaying to the user.
    Unreadable { detail: String },
}

impl VersionProbe {
    /// True when the binary's version matches the expected version.
    /// Both `Mismatch` and `Unreadable` count as failure: a sidecar
    /// that refuses to identify itself is just as much a deployment
    /// hazard as one that identifies as the wrong version.
    pub fn is_ok(&self) -> bool {
        matches!(self.outcome, VersionProbeOutcome::Match { .. })
    }
}

/// Probe both engine sidecars and return one [`VersionProbe`] per
/// binary. Does *not* short-circuit on the first failure, because
/// `gridseak doctor` wants to show the user the state of every
/// binary in one pass.
pub async fn probe_engine_binaries(parser_bin: &Path, analyzer_bin: &Path) -> Vec<VersionProbe> {
    vec![
        probe_one(BinaryKind::Parser, parser_bin).await,
        probe_one(BinaryKind::Analyzer, analyzer_bin).await,
    ]
}

async fn probe_one(which: BinaryKind, bin: &Path) -> VersionProbe {
    let expected = EXPECTED_SIDECAR_VERSION.to_string();
    if !bin.exists() {
        return VersionProbe {
            which,
            path: bin.to_path_buf(),
            expected,
            outcome: VersionProbeOutcome::Unreadable {
                detail: format!("binary not found at {}", bin.display()),
            },
        };
    }
    match read_version(which, bin).await {
        Ok(actual) if actual == EXPECTED_SIDECAR_VERSION => VersionProbe {
            which,
            path: bin.to_path_buf(),
            expected,
            outcome: VersionProbeOutcome::Match { actual },
        },
        Ok(actual) => VersionProbe {
            which,
            path: bin.to_path_buf(),
            expected,
            outcome: VersionProbeOutcome::Mismatch { actual },
        },
        Err(RunError::BinaryVersionUnreadable { detail, .. }) => VersionProbe {
            which,
            path: bin.to_path_buf(),
            expected,
            outcome: VersionProbeOutcome::Unreadable { detail },
        },
        // `read_version` only ever produces `BinaryVersionUnreadable`
        // on the error path. Any other variant here means the error
        // surface drifted underneath us; flag it loudly instead of
        // silently treating it as a generic failure.
        Err(other) => VersionProbe {
            which,
            path: bin.to_path_buf(),
            expected,
            outcome: VersionProbeOutcome::Unreadable {
                detail: format!("unexpected RunError shape from read_version: {other}"),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_line_handles_clap_default_shape() {
        assert_eq!(parse_version_line("ge-analyze 0.1.0\n").unwrap(), "0.1.0");
    }

    #[test]
    fn parse_version_line_handles_no_trailing_newline() {
        assert_eq!(parse_version_line("gridseak 0.1.0").unwrap(), "0.1.0");
    }

    #[test]
    fn parse_version_line_rejects_empty_stdout() {
        let err = parse_version_line("").unwrap_err();
        assert!(err.contains("empty stdout"), "got: {err}");
    }

    #[test]
    fn parse_version_line_rejects_missing_version_token() {
        let err = parse_version_line("ge-analyze\n").unwrap_err();
        assert!(
            err.contains("could not extract version token"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_version_line_takes_first_line_only() {
        // Some binaries print `<name> <version>` followed by extra
        // lines (build metadata, git commit). We only need the first
        // line; the parser must not choke on the rest.
        assert_eq!(
            parse_version_line("ge-analyze 0.1.0\nbuild: abc123\n").unwrap(),
            "0.1.0"
        );
    }
}
