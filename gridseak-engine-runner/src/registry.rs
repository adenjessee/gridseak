//! Language registry sourced from `graphengine-parsing languages --json`.
//!
//! The parser binary is the source of truth for "what languages exist
//! and what is `discovery_only`." We invoke it as a subprocess rather
//! than depending on `graphengine-parsing`'s Rust types because (a) the
//! registry is genuinely an external IPC contract — desktop and CLI
//! both consume it the same way — and (b) keeping the runner free of
//! parser internals means changes to the parser's config-loader
//! representation don't ripple into this crate.
//!
//! This mirrors the in-shape data structure of
//! `desktop/src-tauri/src/language.rs::SupportedLanguage`. When
//! Stage 0 work moves to Phase B, that desktop file should switch to
//! re-exporting / re-using these types instead of declaring a parallel
//! struct.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// One language as reported by `graphengine-parsing languages --json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SupportedLanguage {
    pub language: String,
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub lsp_command: Option<String>,
    /// `true` means the engine recognizes this language for file-type
    /// purposes but cannot parse it as a standalone `--lang` (e.g.
    /// Visualforce: discovery only; absorbed by the Apex parser).
    #[serde(default)]
    pub discovery_only: bool,
}

/// In-memory snapshot of the parser's language registry.
#[derive(Debug, Clone)]
pub struct LanguageRegistry {
    entries: Vec<SupportedLanguage>,
}

impl LanguageRegistry {
    /// Spawn `parser_bin --configs-dir <configs_dir> languages --json`
    /// and parse the output.
    pub async fn load_from(parser_bin: &Path, configs_dir: &Path) -> anyhow::Result<Self> {
        let output = Command::new(parser_bin)
            .arg("--configs-dir")
            .arg(configs_dir)
            .arg("languages")
            .arg("--json")
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "`{} languages --json` exited {}: {}",
                parser_bin.display(),
                output.status,
                stderr.trim()
            );
        }

        let entries: Vec<SupportedLanguage> = serde_json::from_slice(&output.stdout)?;
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[SupportedLanguage] {
        &self.entries
    }

    /// Split a caller-supplied list of language names into
    /// `(parseable, skipped)` based on the registry's `discovery_only`
    /// flag. Languages not present in the registry at all are passed
    /// through to the `parseable` side — let the parser binary surface
    /// the "unknown language" error, since the registry might lag the
    /// installed binary in some rollout scenarios.
    pub fn partition_parseable(&self, requested: &[String]) -> (Vec<String>, Vec<String>) {
        let mut parseable = Vec::with_capacity(requested.len());
        let mut skipped = Vec::new();
        for lang in requested {
            let is_discovery_only = self
                .entries
                .iter()
                .find(|e| e.language.eq_ignore_ascii_case(lang))
                .map(|e| e.discovery_only)
                .unwrap_or(false);
            if is_discovery_only {
                skipped.push(lang.clone());
            } else {
                parseable.push(lang.clone());
            }
        }
        (parseable, skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(language: &str, discovery_only: bool) -> SupportedLanguage {
        SupportedLanguage {
            language: language.to_string(),
            file_extensions: vec![],
            lsp_command: None,
            discovery_only,
        }
    }

    fn registry_with(entries: Vec<SupportedLanguage>) -> LanguageRegistry {
        LanguageRegistry { entries }
    }

    #[test]
    fn partition_drops_discovery_only() {
        let reg = registry_with(vec![fixture("apex", false), fixture("visualforce", true)]);
        let (parse, skip) = reg.partition_parseable(&["apex".into(), "visualforce".into()]);
        assert_eq!(parse, vec!["apex"]);
        assert_eq!(skip, vec!["visualforce"]);
    }

    #[test]
    fn partition_passes_through_unknown_languages() {
        // Registry doesn't know "klingon" — pass through and let the
        // parser binary error out with its own message rather than
        // silently dropping the request.
        let reg = registry_with(vec![fixture("rust", false)]);
        let (parse, skip) = reg.partition_parseable(&["klingon".into()]);
        assert_eq!(parse, vec!["klingon"]);
        assert!(skip.is_empty());
    }

    #[test]
    fn partition_is_case_insensitive() {
        let reg = registry_with(vec![fixture("Apex", false)]);
        let (parse, _) = reg.partition_parseable(&["APEX".into()]);
        assert_eq!(parse, vec!["APEX"]);
    }
}
