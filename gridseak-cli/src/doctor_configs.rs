//! `gridseak doctor` — language-configs consistency check.
//!
//! # What this answers
//!
//! "Will scans load the language configs, and are they the ones shipped
//! with *this* build of the parser sidecar?"
//!
//! The LSP reliability work proved that fixing a config (e.g. pointing
//! Python at `pyright-langserver`) is worthless if the installed
//! binaries load a *different*, stale `configs/` directory. The parser
//! resolves configs via `GRAPHENGINE_CONFIGS_DIR` or an ancestor probe
//! relative to its own executable; either can point at a directory that
//! does not match the sidecar that will actually run.
//!
//! This check surfaces three deterministic facts:
//!
//! 1. Did a configs directory resolve at all, and how?
//! 2. Does it contain the core LSP-bearing language configs?
//! 3. Is it co-located with the resolved `graphengine-parsing` sidecar
//!    (the strongest "same build" signal we can compute without
//!    embedding a build fingerprint)?
//!
//! A content fingerprint is intentionally *not* computed here: without
//! a baseline captured at build time it would be informational only.
//! That can be layered on later; the actionable drift signals above do
//! not need it.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// LSP-bearing languages that a complete configs directory must ship.
/// Absence of any of these is the signature of a stale or partial dir.
const CORE_LANGUAGES: &[&str] = &[
    "python",
    "typescript",
    "javascript",
    "go",
    "java",
    "rust",
    "apex",
];

/// Result of the configs-dir check.
///
/// `status` is the single field CI / scripts branch on:
/// - `ok` — a configs dir resolved and contains every core language.
/// - `incomplete` — a configs dir resolved but is missing core languages
///   (likely a stale or partial directory).
/// - `not_found` — no configs dir could be resolved; scans cannot run.
#[derive(Debug, Serialize)]
pub struct ConfigsCheck {
    pub status: String,
    /// Resolved configs directory, if any.
    pub configs_dir: Option<String>,
    /// How it was resolved: `env` (`GRAPHENGINE_CONFIGS_DIR`) or `probe`
    /// (ancestor-of-exe / workspace fallback), or `unresolved`.
    pub source: String,
    /// `Some(true)` when the configs dir and the parser sidecar share a
    /// parent directory (canonicalized), `Some(false)` when they don't,
    /// `None` when the sidecar path was unavailable to compare against.
    pub colocated_with_sidecar: Option<bool>,
    /// Core languages present in the resolved dir.
    pub present_languages: Vec<String>,
    /// Core languages expected but absent.
    pub missing_languages: Vec<String>,
    pub detail: Option<String>,
}

/// Run the configs check. `configs_dir` is the directory the CLI
/// resolved with the same rules `scan` uses (pass `None` when nothing
/// resolved). `parser_bin` is the resolved `graphengine-parsing` path,
/// used only for the co-location signal.
pub fn check_configs(configs_dir: Option<PathBuf>, parser_bin: Option<&Path>) -> ConfigsCheck {
    let source = configs_source(configs_dir.as_deref());

    let Some(dir) = configs_dir else {
        return ConfigsCheck {
            status: "not_found".to_string(),
            configs_dir: None,
            source,
            colocated_with_sidecar: None,
            present_languages: Vec::new(),
            missing_languages: CORE_LANGUAGES.iter().map(|s| s.to_string()).collect(),
            detail: Some(
                "no configs directory resolved. Set GRAPHENGINE_CONFIGS_DIR or reinstall \
                 with scripts/install/install.sh so configs ship next to the sidecar."
                    .to_string(),
            ),
        };
    };

    if !dir.is_dir() {
        return ConfigsCheck {
            status: "not_found".to_string(),
            configs_dir: Some(dir.display().to_string()),
            source,
            colocated_with_sidecar: None,
            present_languages: Vec::new(),
            missing_languages: CORE_LANGUAGES.iter().map(|s| s.to_string()).collect(),
            detail: Some(format!(
                "resolved configs path is not a directory: {}",
                dir.display()
            )),
        };
    }

    let (present, missing) = core_language_presence(&dir);
    let colocated = colocated_with_sidecar(&dir, parser_bin);

    let mut detail = None;
    let status = if !missing.is_empty() {
        detail = Some(format!(
            "configs dir is missing core language configs: {}. This usually means a \
             stale or partial configs/ directory.",
            missing.join(", ")
        ));
        "incomplete".to_string()
    } else {
        if colocated == Some(false) {
            detail = Some(
                "configs dir resolved but is NOT co-located with the graphengine-parsing \
                 sidecar. Scans may load configs from a different build than the binary that \
                 runs. Set GRAPHENGINE_CONFIGS_DIR to the sidecar's configs/ or reinstall."
                    .to_string(),
            );
        }
        "ok".to_string()
    };

    ConfigsCheck {
        status,
        configs_dir: Some(dir.display().to_string()),
        source,
        colocated_with_sidecar: colocated,
        present_languages: present,
        missing_languages: missing,
        detail,
    }
}

fn configs_source(configs_dir: Option<&Path>) -> String {
    if configs_dir.is_none() {
        return "unresolved".to_string();
    }
    match std::env::var_os("GRAPHENGINE_CONFIGS_DIR") {
        Some(_) => "env".to_string(),
        None => "probe".to_string(),
    }
}

/// Split the core language set into (present, missing) by checking for
/// `{lang}.yaml` files in the resolved dir.
fn core_language_presence(dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for &lang in CORE_LANGUAGES {
        if dir.join(format!("{lang}.yaml")).is_file() {
            present.push(lang.to_string());
        } else {
            missing.push(lang.to_string());
        }
    }
    (present, missing)
}

/// Whether the configs dir and the parser sidecar share a parent
/// directory. Both paths are canonicalized first so the installer's
/// `~/.gridseak/configs -> share/<ts>/configs` symlink resolves to the
/// same `share/<ts>` parent as `share/<ts>/graphengine-parsing`.
fn colocated_with_sidecar(configs_dir: &Path, parser_bin: Option<&Path>) -> Option<bool> {
    let parser = parser_bin?;
    let configs_parent = canonical_parent(configs_dir)?;
    let parser_parent = canonical_parent(parser)?;
    Some(configs_parent == parser_parent)
}

fn canonical_parent(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_lang(dir: &Path, lang: &str) {
        std::fs::write(dir.join(format!("{lang}.yaml")), b"language: x\n").unwrap();
    }

    #[test]
    fn not_found_when_dir_is_none() {
        let check = check_configs(None, None);
        assert_eq!(check.status, "not_found");
        assert_eq!(check.source, "unresolved");
        assert_eq!(check.missing_languages.len(), CORE_LANGUAGES.len());
    }

    #[test]
    fn incomplete_when_core_language_missing() {
        let dir = TempDir::new().unwrap();
        // Everything except python.
        for &lang in CORE_LANGUAGES.iter().filter(|l| **l != "python") {
            write_lang(dir.path(), lang);
        }
        let check = check_configs(Some(dir.path().to_path_buf()), None);
        assert_eq!(check.status, "incomplete", "detail: {:?}", check.detail);
        assert!(check.missing_languages.contains(&"python".to_string()));
    }

    #[test]
    fn ok_when_all_core_languages_present() {
        let dir = TempDir::new().unwrap();
        for &lang in CORE_LANGUAGES {
            write_lang(dir.path(), lang);
        }
        let check = check_configs(Some(dir.path().to_path_buf()), None);
        assert_eq!(check.status, "ok", "detail: {:?}", check.detail);
        assert!(check.missing_languages.is_empty());
        // No sidecar passed -> co-location is unknown, not a failure.
        assert_eq!(check.colocated_with_sidecar, None);
    }

    #[test]
    fn detects_non_colocated_sidecar() {
        let configs = TempDir::new().unwrap();
        for &lang in CORE_LANGUAGES {
            write_lang(configs.path(), lang);
        }
        // Sidecar lives in an unrelated directory.
        let sidecar_dir = TempDir::new().unwrap();
        let sidecar = sidecar_dir.path().join("graphengine-parsing");
        std::fs::write(&sidecar, b"#!/bin/sh\n").unwrap();

        let check = check_configs(Some(configs.path().to_path_buf()), Some(&sidecar));
        assert_eq!(check.status, "ok");
        assert_eq!(check.colocated_with_sidecar, Some(false));
        assert!(check.detail.is_some());
    }
}
