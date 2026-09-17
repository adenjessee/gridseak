//! Contract tests for the shipped LSP language configs.
//!
//! These guard two concrete reliability fixes from the LSP reliability
//! handoff so they cannot silently regress:
//!
//! 1. Python must launch `pyright-langserver` (the LSP server), NOT the
//!    `pyright` type-checking CLI, which does not speak LSP over
//!    `--stdio` and exits, forcing a heuristic fallback.
//! 2. Per-language request timeouts must not be set so low that a cold
//!    server's first `textDocument/definition` times out before it can
//!    answer, causing premature fallback. We pin a sane base floor.
//!
//! The first test goes through the real config loader (proving
//! resolution + parsing); the rest read the shipped YAML files directly
//! so the on-disk contract is checked independent of loader behavior.

use std::path::PathBuf;

use graphengine_parsing::infrastructure::config::load_language_descriptor;

/// The base (fast-policy) request-timeout floor every language config
/// must meet. Patient/exhaustive policies raise this further at runtime;
/// this is only the lower bound shipped in the YAML.
const MIN_REQUEST_TIMEOUT_MS: u64 = 5000;

fn configs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs")
}

fn read_config_yaml(language: &str) -> serde_yaml::Value {
    let path = configs_dir().join(format!("{language}.yaml"));
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_yaml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn python_config_loader_uses_pyright_langserver() {
    let descriptor =
        load_language_descriptor("python").expect("python.yaml should load via the real loader");
    assert_eq!(
        descriptor.lsp_command.as_deref(),
        Some("pyright-langserver"),
        "Python must use the LSP server `pyright-langserver`, not the `pyright` CLI"
    );
}

#[test]
fn python_config_passes_stdio_transport() {
    let cfg = read_config_yaml("python");
    let args: Vec<String> = cfg["lsp_args"]
        .as_sequence()
        .expect("python.yaml lsp_args should be a list")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        args.iter().any(|a| a == "--stdio"),
        "pyright-langserver must be launched with --stdio; got {args:?}"
    );
}

#[test]
fn lsp_request_timeouts_meet_base_floor() {
    // Languages whose timeouts were flagged as too low in the handoff,
    // plus the rest of the subprocess-LSP languages for consistency.
    // Apex is intentionally higher (jorje needs a longer budget) and is
    // covered by its own config; we only assert the floor here.
    for language in [
        "python",
        "go",
        "typescript",
        "javascript",
        "java",
        "rust",
        "csharp",
        "apex",
    ] {
        let cfg = read_config_yaml(language);
        let timeout = cfg["lsp_request_timeout_ms"]
            .as_u64()
            .unwrap_or_else(|| panic!("{language}.yaml missing numeric lsp_request_timeout_ms"));
        assert!(
            timeout >= MIN_REQUEST_TIMEOUT_MS,
            "{language}.yaml lsp_request_timeout_ms={timeout} is below the base floor \
             of {MIN_REQUEST_TIMEOUT_MS}ms; a cold server's first definition request \
             can exceed that and force premature heuristic fallback"
        );
    }
}

#[test]
fn declared_lsp_commands_match_expected_servers() {
    // Pins the canonical server name per language so a typo (or a repeat
    // of the pyright vs pyright-langserver bug) is caught immediately.
    let expected = [
        ("python", "pyright-langserver"),
        ("typescript", "typescript-language-server"),
        ("javascript", "typescript-language-server"),
        ("go", "gopls"),
        ("java", "jdtls"),
        ("rust", "rust-analyzer"),
        ("csharp", "omnisharp"),
        ("apex", "java"),
    ];
    for (language, server) in expected {
        let cfg = read_config_yaml(language);
        assert_eq!(
            cfg["lsp_command"].as_str(),
            Some(server),
            "{language}.yaml lsp_command should be `{server}`"
        );
    }
}
