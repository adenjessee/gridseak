//! Per-language LSP session options (readiness + workspace folders).
//!
//! Apex keeps its own builder in `syntax::language::apex::lsp_session`.
//! All other subprocess-LSP languages route through
//! [`build_session_options`] so `patient` / `exhaustive` policy tiers
//! can opt into a `ProgressAndProbe` readiness barrier without
//! scattering language checks across the factory.

use std::path::Path;
use std::time::Duration;
use tracing::{debug, info};
use url::Url;
use walkdir::WalkDir;

use crate::infrastructure::lsp::definition_provider::SessionOptions;
use crate::infrastructure::lsp::policy::runtime_policy;
use crate::infrastructure::lsp::session::ReadinessStrategy;

/// Directories skipped when hunting for a readiness canary file.
const SKIP_DIR_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "venv",
    ".venv",
    "__pycache__",
    "vendor",
];

/// Build session options for a subprocess-LSP language parse.
///
/// Under `fast` policy this returns defaults (`Immediate` readiness).
/// Under `patient` / `exhaustive`, languages with a known readiness
/// model get `ProgressAndProbe` with a real source-file canary and
/// policy-scaled deadlines.
pub fn build_session_options(language: &str, workspace_root: Option<&Url>) -> SessionOptions {
    build_session_options_with_policy(language, workspace_root, runtime_policy())
}

/// Like [`build_session_options`] but accepts an explicit policy (used in tests).
pub fn build_session_options_with_policy(
    language: &str,
    workspace_root: Option<&Url>,
    policy: &crate::infrastructure::lsp::policy::LspRuntimePolicy,
) -> SessionOptions {
    if !policy.tier.uses_patient_defaults() {
        return SessionOptions::default();
    }

    let readiness = match language {
        "python" => readiness_for_extensions(
            workspace_root,
            &["py"],
            policy.index_wait,
            policy.document_settle,
            None,
        ),
        "typescript" | "javascript" => readiness_for_extensions(
            workspace_root,
            &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
            policy.index_wait,
            policy.document_settle,
            None,
        ),
        "java" => readiness_for_extensions(
            workspace_root,
            &["java"],
            policy.index_wait.max(Duration::from_secs(90)),
            policy.document_settle,
            Some(Duration::from_secs(5)),
        ),
        "go" => readiness_for_extensions(
            workspace_root,
            &["go"],
            policy.index_wait.max(Duration::from_secs(45)),
            policy.document_settle,
            None,
        ),
        "rust" => readiness_for_extensions(
            workspace_root,
            &["rs"],
            policy.index_wait,
            policy.document_settle,
            None,
        ),
        _ => ReadinessStrategy::Immediate,
    };

    if !matches!(readiness, ReadinessStrategy::Immediate) {
        info!(
            language,
            tier = policy.tier.as_str(),
            "LSP session options: patient-tier readiness engaged"
        );
    }

    SessionOptions {
        workspace_folders: Vec::new(),
        readiness,
    }
}

fn readiness_for_extensions(
    workspace_root: Option<&Url>,
    extensions: &[&str],
    deadline: Duration,
    document_settle: Duration,
    quiet_override: Option<Duration>,
) -> ReadinessStrategy {
    let canary = workspace_root
        .and_then(|url| url.to_file_path().ok())
        .and_then(|root| pick_canary_file(&root, extensions));

    let quiet_period = quiet_override.unwrap_or_else(|| {
        if document_settle.is_zero() {
            Duration::from_secs(3)
        } else {
            document_settle
        }
    });

    debug!(
        canary = ?canary,
        deadline_ms = deadline.as_millis() as u64,
        quiet_ms = quiet_period.as_millis() as u64,
        "ProgressAndProbe readiness configured"
    );

    ReadinessStrategy::ProgressAndProbe {
        canary_file: canary,
        deadline,
        quiet_period,
    }
}

/// First source file under `root` matching one of `extensions` (without dot).
fn pick_canary_file(root: &Path, extensions: &[&str]) -> Option<String> {
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !SKIP_DIR_NAMES.iter().any(|skip| name == *skip)
        })
    {
        let entry = entry.ok()?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if extensions.iter().any(|want| ext.eq_ignore_ascii_case(want)) {
                return path_to_file_uri(path);
            }
        }
    }
    None
}

fn path_to_file_uri(path: &Path) -> Option<String> {
    Url::from_file_path(path).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::lsp::policy::{LspPolicy, LspRuntimePolicy};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn patient_policy(language: &str) -> LspRuntimePolicy {
        LspRuntimePolicy::resolve(Some(LspPolicy::Patient), &sample_config(language))
    }

    fn fast_policy(language: &str) -> LspRuntimePolicy {
        LspRuntimePolicy::resolve(Some(LspPolicy::Fast), &sample_config(language))
    }

    fn sample_config(language: &str) -> crate::infrastructure::config::LanguageConfig {
        let mut queries = HashMap::new();
        queries.insert("functions".into(), "dummy".into());
        queries.insert("structs".into(), "dummy".into());
        queries.insert("modules".into(), "dummy".into());
        queries.insert("call_sites".into(), "dummy".into());
        crate::infrastructure::config::LanguageConfig {
            language: language.into(),
            file_extensions: vec![".py".into()],
            queries,
            kind_mappings: HashMap::new(),
            grammar_path: None,
            lsp_command: Some("pyright-langserver".into()),
            lsp_args: Some(vec!["--stdio".into()]),
            version: "1.0".into(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: Some(5_000),
            lsp_max_concurrent_requests: Some(32),
            lsp_initialization_options: None,
        }
    }

    fn install_patient_policy(language: &str) -> LspRuntimePolicy {
        patient_policy(language)
    }

    #[test]
    fn fast_policy_yields_immediate_readiness() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_session_options_with_policy("python", Some(&root), &fast_policy("python"));
        assert!(matches!(opts.readiness, ReadinessStrategy::Immediate));
    }

    #[test]
    fn patient_python_picks_py_canary() {
        let policy = install_patient_policy("python");

        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("main.py"), "def main(): pass\n").unwrap();
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_session_options_with_policy("python", Some(&root), &policy);
        match opts.readiness {
            ReadinessStrategy::ProgressAndProbe { canary_file, .. } => {
                let c = canary_file.expect("expected python canary");
                assert!(c.ends_with("main.py"), "canary was {c}");
            }
            other => panic!("expected ProgressAndProbe, got {other:?}"),
        }
    }

    #[test]
    fn patient_typescript_picks_ts_canary() {
        let policy = install_patient_policy("typescript");

        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("index.ts"), "export const x = 1;\n").unwrap();
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_session_options_with_policy("typescript", Some(&root), &policy);
        match opts.readiness {
            ReadinessStrategy::ProgressAndProbe { canary_file, .. } => {
                let c = canary_file.expect("expected typescript canary");
                assert!(c.ends_with("index.ts"), "canary was {c}");
            }
            other => panic!("expected ProgressAndProbe, got {other:?}"),
        }
    }

    #[test]
    fn patient_java_uses_longer_deadline() {
        let policy = install_patient_policy("java");

        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("Main.java"), "class Main {}\n").unwrap();
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_session_options_with_policy("java", Some(&root), &policy);
        match opts.readiness {
            ReadinessStrategy::ProgressAndProbe { deadline, .. } => {
                assert!(
                    deadline >= Duration::from_secs(90),
                    "java readiness deadline should be >= 90s, got {deadline:?}"
                );
            }
            other => panic!("expected ProgressAndProbe, got {other:?}"),
        }
    }

    #[test]
    fn patient_go_picks_go_canary() {
        let policy = install_patient_policy("go");

        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("main.go"), "package main\nfunc main() {}\n").unwrap();
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_session_options_with_policy("go", Some(&root), &policy);
        match opts.readiness {
            ReadinessStrategy::ProgressAndProbe {
                canary_file,
                deadline,
                ..
            } => {
                let c = canary_file.expect("expected go canary");
                assert!(c.ends_with("main.go"), "canary was {c}");
                assert!(
                    deadline >= Duration::from_secs(45),
                    "go readiness deadline should be >= 45s, got {deadline:?}"
                );
            }
            other => panic!("expected ProgressAndProbe, got {other:?}"),
        }
    }

    #[test]
    fn pick_canary_skips_node_modules() {
        let tmp = tempdir().unwrap();
        let nm = tmp.path().join("node_modules/pkg");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("index.js"), "module.exports = {}").unwrap();
        std::fs::write(tmp.path().join("app.ts"), "export {}").unwrap();

        let canary = pick_canary_file(tmp.path(), &["ts", "js"])
            .expect("should find app.ts, not node_modules");
        assert!(canary.ends_with("app.ts"));
    }
}
