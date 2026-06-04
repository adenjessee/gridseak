#![cfg(feature = "lsp-tests")]
//! Integration tests for LSP with real language servers.
//!
//! These tests require the appropriate LSP servers to be installed:
//! - Rust: `rust-analyzer`
//! - Python: `pyright` (or `pylsp`)
//! - JavaScript/TypeScript: `typescript-language-server`
//!
//! Tests are skipped if servers are not available.

use graphengine_parsing::application::use_cases::parse_repo::ParseRepositoryUseCase;
use graphengine_parsing::domain::{Confidence, EdgeKind, ProvenanceSource, Range};
use graphengine_parsing::infrastructure::config::{load_config, LanguageConfig};
use graphengine_parsing::infrastructure::lsp::errors::LspError;
use graphengine_parsing::infrastructure::lsp::session::SessionSupervisor;
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use url::Url;
use which::which;

/// Check if an LSP server command is available
fn is_lsp_server_available(command: &str) -> bool {
    which(command).is_ok()
}

/// Test LSP initialization for Rust
#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_rust_lsp_initialization() {
    if !is_lsp_server_available("rust-analyzer") {
        eprintln!("Skipping test: rust-analyzer not found in PATH");
        return;
    }

    let config = load_config("rust").expect("Should load Rust config");
    let temp_dir = tempfile::tempdir().expect("Should create temp dir");
    let workspace_root = Url::from_directory_path(temp_dir.path()).unwrap();

    let session = Arc::new(SessionSupervisor::new(config, Some(workspace_root)));

    timeout(Duration::from_secs(30), session.initialize())
        .await
        .expect("Initialization should complete within 30s")
        .expect("Should initialize successfully");

    // Check that session is ready
    assert_eq!(
        session.get_state(),
        graphengine_parsing::infrastructure::lsp::session::SessionState::Ready
    );
}

/// Test document synchronization for Rust
#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_rust_lsp_document_sync() {
    if !is_lsp_server_available("rust-analyzer") {
        eprintln!("Skipping test: rust-analyzer not found in PATH");
        return;
    }

    let config = load_config("rust").expect("Should load Rust config");
    let session = Arc::new(SessionSupervisor::new(config, None));

    timeout(Duration::from_secs(30), session.initialize())
        .await
        .expect("Initialization should complete")
        .expect("Should initialize successfully");

    // Create a test file
    let temp_dir = tempfile::tempdir().expect("Should create temp dir");
    let test_file = temp_dir.path().join("test.rs");
    let content = r#"
pub fn hello() -> String {
    "Hello, world!".to_string()
}

pub fn main() {
    let msg = hello();
    println!("{}", msg);
}
"#;

    std::fs::write(&test_file, content).expect("Should write test file");

    // Open document
    let open_result = session
        .document_did_open(test_file.to_string_lossy().as_ref(), content.to_string())
        .await;
    assert!(open_result.is_ok(), "Should open document");

    // Try to find definition
    let location = Range::with_file(7, 0, 7, 10, test_file.to_string_lossy().to_string());
    let def_result = session.find_definition("hello", &location).await;

    assert!(def_result.is_ok(), "Should not error on definition lookup");
    if let Some(def_range) = def_result.unwrap() {
        assert_eq!(
            def_range.file,
            test_file.to_string_lossy(),
            "Definition should be in same file"
        );
    } else {
        eprintln!("rust-analyzer returned no definition for 'hello'; continuing");
    }

    // Close document
    let close_result = session
        .document_did_close(test_file.to_string_lossy().as_ref())
        .await;
    assert!(close_result.is_ok(), "Should close document");
}

/// Ensure the real LSP resolver produces high-confidence cross-file call edges
#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_rust_lsp_cross_file_call_edges() {
    if !is_lsp_server_available("rust-analyzer") {
        eprintln!("Skipping test: rust-analyzer not found in PATH");
        return;
    }

    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("function-relationship-test");
    let workspace_url = Url::from_directory_path(&fixture_root).expect("workspace URL");

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db_path = temp_dir.path().join("graph.db");

    let use_case = ParseRepositoryUseCase::with_real_components(
        "rust".to_string(),
        Confidence::Medium,
        db_path.to_string_lossy().as_ref(),
        Some(workspace_url),
    )
    .await
    .expect("create use case");

    let resolved = use_case
        .parse(fixture_root.clone(), "rust".to_string())
        .await
        .expect("parse fixture with LSP");

    let stats = resolved.stats();
    assert!(
        stats.lsp_edges >= 6,
        "Expected at least 6 LSP-backed call edges, got {} \
         (heuristic_call_fallbacks={}, total edges={}). \
         Known issue: SessionSupervisor::wait_until_ready returns the moment \
         `initialize` resolves but does NOT block on rust-analyzer's rocksdb \
         indexing. Definition queries fire too early and all return None, \
         falling back to heuristic. Fix is to consume $/progress notifications \
         (rustAnalyzer/cachePriming token); tracked in Sprint D LSP robustness.",
        stats.lsp_edges,
        stats.heuristic_call_fallbacks,
        resolved.graph().edges.len(),
    );

    let graph = resolved.graph();
    let mut fqn_by_id = std::collections::HashMap::new();
    for node in &graph.nodes {
        fqn_by_id.insert(node.id.clone(), node.fqn.clone());
    }

    let mut lsp_edges = Vec::new();
    for edge in &graph.edges {
        if edge.kind == EdgeKind::Call && edge.provenance.source == ProvenanceSource::Lsp {
            if let (Some(from), Some(to)) =
                (fqn_by_id.get(&edge.from_id), fqn_by_id.get(&edge.to_id))
            {
                lsp_edges.push((from.clone(), to.clone()));
            }
        }
    }

    assert!(
        lsp_edges
            .iter()
            .any(|(from, to)| from.ends_with("::a4_cross_b1") && to.ends_with("::b1_base")),
        "Expected LSP to resolve cross-file call a4_cross_b1 -> b1_base, edges: {:?}",
        lsp_edges
    );
    assert!(
        lsp_edges
            .iter()
            .any(|(from, to)| from.ends_with("::c4_cross_a2") && to.ends_with("::a2_uses_a1")),
        "Expected LSP to resolve cross-file call c4_cross_a2 -> a2_uses_a1, edges: {:?}",
        lsp_edges
    );
}

/// Test Python LSP initialization (if pyright is available)
#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_python_lsp_initialization() {
    if !is_lsp_server_available("pyright") {
        eprintln!("Skipping test: pyright not found in PATH");
        return;
    }

    let config = load_config("python").expect("Should load Python config");
    let temp_dir = tempfile::tempdir().expect("Should create temp dir");
    let workspace_root = Url::from_directory_path(temp_dir.path()).unwrap();

    let session = Arc::new(SessionSupervisor::new(config, Some(workspace_root)));

    timeout(Duration::from_secs(30), session.initialize())
        .await
        .expect("Initialization should complete")
        .expect("Should initialize successfully");
    assert_eq!(
        session.get_state(),
        graphengine_parsing::infrastructure::lsp::session::SessionState::Ready
    );
}

/// Test JavaScript/TypeScript LSP initialization
#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_javascript_lsp_initialization() {
    if !is_lsp_server_available("typescript-language-server") {
        eprintln!("Skipping test: typescript-language-server not found in PATH");
        return;
    }

    let config = load_config("javascript").expect("Should load JavaScript config");
    let temp_dir = tempfile::tempdir().expect("Should create temp dir");
    let workspace_root = Url::from_directory_path(temp_dir.path()).unwrap();

    let session = Arc::new(SessionSupervisor::new(config, Some(workspace_root)));

    timeout(Duration::from_secs(30), session.initialize())
        .await
        .expect("Initialization should complete")
        .expect("Should initialize successfully");
    assert_eq!(
        session.get_state(),
        graphengine_parsing::infrastructure::lsp::session::SessionState::Ready
    );
}

/// Test that LSP gracefully handles unavailable servers
#[tokio::test]
async fn test_lsp_unavailable_server() {
    let config = LanguageConfig {
        language: "nonexistent".to_string(),
        file_extensions: vec![".test".to_string()],
        queries: std::collections::HashMap::new(),
        kind_mappings: std::collections::HashMap::new(),
        grammar_path: None,
        lsp_command: Some("nonexistent-lsp-server-xyz".to_string()),
        lsp_args: Some(vec!["--stdio".to_string()]),
        version: "1.0".to_string(),
        receiver_type_detection: None,
        lsp_request_timeout_ms: None,
        lsp_max_concurrent_requests: None,
        lsp_initialization_options: None,
    };

    let session = Arc::new(SessionSupervisor::new(config, None));

    let err = timeout(Duration::from_secs(5), session.initialize())
        .await
        .expect("Should complete")
        .expect_err("Should fail to initialize nonexistent server");

    assert!(
        err.is_server_unavailable() || matches!(err, LspError::InvalidConfig(_)),
        "Unexpected error variant when LSP server is missing"
    );
}

// TODO: Test document symbols request once SessionSupervisor exposes this functionality
// #[tokio::test]
// #[ignore]
// async fn test_rust_lsp_document_symbols() {
//     // This test requires document_symbols to be exposed via SessionSupervisor
// }
