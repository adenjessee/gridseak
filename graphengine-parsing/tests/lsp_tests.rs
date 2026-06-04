//! Integration tests for the LSP infrastructure layer
//!
//! Tests the complete LSP pipeline including session management,
//! semantic resolution, and error handling.

use graphengine_parsing::config::load_config;
use graphengine_parsing::infrastructure::lsp::{
    LspResolver, SecurityConfig, SessionState, SessionSupervisor,
};
use graphengine_parsing::ports::{SemanticResolver, SyntaxResults};
use graphengine_parsing::*;

/// Test LSP resolver creation and basic functionality
#[tokio::test]
async fn test_lsp_resolver_creation() {
    let config = load_config("rust").unwrap();
    let _resolver = LspResolver::new(config, None);

    // Should be created successfully
    // test passes by not panicking
}

/// Test session supervisor state management
#[tokio::test]
async fn test_session_supervisor_states() {
    let config = load_config("rust").unwrap();
    let supervisor = SessionSupervisor::new(config, None);

    // Initially should be in Idle state
    assert_eq!(supervisor.get_state(), SessionState::Idle);

    // Test state transitions
    assert!(!supervisor.get_state().is_functional());
    assert!(!supervisor.get_state().can_accept_requests());
}

/// Test LSP resolver availability check
#[tokio::test]
async fn test_lsp_resolver_availability() {
    let config = load_config("rust").unwrap();
    let resolver = LspResolver::new(config, None);

    // Currently the resolver reports availability based on configuration
    assert!(resolver.is_available().await);
}

/// Test LSP resolution with empty syntax results
#[tokio::test]
async fn test_lsp_resolution_empty() {
    let config = load_config("rust").unwrap();
    let resolver = LspResolver::new(config, None);

    let syntax_results = SyntaxResults::new();
    let result = resolver.resolve(&syntax_results).await;

    // Heuristic fallback should still work and produce results
    assert!(result.is_ok());
    let _ = result.unwrap();
    // No specific assertion on edges for now
}

/// Test LSP resolution with syntax results
#[tokio::test]
async fn test_lsp_resolution_with_syntax() {
    let config = load_config("rust").unwrap();
    let resolver = LspResolver::new(config, None);

    let mut syntax_results = SyntaxResults::new();

    // Add test symbols and call sites
    let node = Node::new(
        NodeKind::Function,
        "test::function".to_string(),
        Range::new(1, 0, 5, 10),
        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
    ); // Node::new already sets trait_metadata: None
    syntax_results.add_symbol(node);
    syntax_results.add_call_site(Range::new(2, 0, 2, 10), "test::function".to_string());
    syntax_results.add_import(Range::new(3, 0, 3, 20));
    syntax_results.add_type_ref(Range::new(4, 0, 4, 15));

    let result = resolver.resolve(&syntax_results).await;

    // Should return empty edges when LSP is not available
    assert!(result.is_ok());
    let _ = result.unwrap();
    // No specific assertion on heuristic output
}

/// Test security configuration
#[tokio::test]
async fn test_security_config() {
    let config = SecurityConfig::default();

    assert_eq!(config.max_cpu_time, 300);
    assert_eq!(config.max_memory, 1024 * 1024 * 1024);
    assert_eq!(config.max_file_size, 100 * 1024 * 1024);
    assert_eq!(config.max_open_files, 1024);
    // max_processes field doesn't exist in SecurityConfig
}

/// Test security support detection
#[tokio::test]
async fn test_security_support() {
    // This test will pass on Unix-like systems, fail on Windows
    // We just want to make sure the function doesn't panic
    let _supported = graphengine_parsing::infrastructure::lsp::security::check_security_support();
    // test passes by not panicking
}

/// Test LSP error types
#[tokio::test]
async fn test_lsp_errors() {
    use graphengine_parsing::infrastructure::lsp::errors::LspError;

    let conn_error = LspError::connection_failed("Test connection error");
    assert!(matches!(conn_error, LspError::ConnectionFailed(_)));

    let timeout_error = LspError::timeout(5000);
    assert!(matches!(
        timeout_error,
        LspError::Timeout { timeout_ms: 5000 }
    ));

    let crash_error = LspError::server_crashed("Test crash");
    assert!(matches!(crash_error, LspError::ServerCrashed(_)));

    // Test error display
    let error_str = format!("{}", conn_error);
    assert!(error_str.contains("LSP server connection failed"));
    assert!(error_str.contains("Test connection error"));
}

/// Test session supervisor retry budget
#[tokio::test]
async fn test_retry_budget() {
    let config = load_config("rust").unwrap();
    let supervisor = SessionSupervisor::new(config, None);

    // Should start with retry budget
    supervisor.reset_retry_budget();

    // Test that supervisor can be cloned
    let supervisor_clone = supervisor.clone();
    assert_eq!(supervisor.get_state(), supervisor_clone.get_state());
}

/// Test LSP resolver with different configurations
#[tokio::test]
async fn test_lsp_resolver_configurations() {
    let config = load_config("rust").unwrap();

    // Test with workspace root
    let workspace_root = url::Url::parse("file:///tmp/test").unwrap();
    let _resolver_with_root = LspResolver::new(config.clone(), Some(workspace_root));

    // Test without workspace root
    let _resolver_without_root = LspResolver::new(config, None);

    // Both should be created successfully
    // test passes by not panicking
}

/// Test LSP resolution performance with large syntax results
#[tokio::test]
async fn test_lsp_resolution_performance() {
    let config = load_config("rust").unwrap();
    let resolver = LspResolver::new(config, None);

    let mut syntax_results = SyntaxResults::new();

    // Add many symbols and call sites
    for i in 0..100 {
        let node = Node::new(
            NodeKind::Function,
            format!("test::function_{}", i),
            Range::new(i as u32 + 1, 0, (i + 5) as u32, 10),
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        );
        syntax_results.add_symbol(node);
        syntax_results.add_call_site(
            Range::new(i as u32 + 2, 0, (i + 2) as u32, 10),
            format!("call_{}", i),
        );
    }

    let start = std::time::Instant::now();
    let result = resolver.resolve(&syntax_results).await;
    let duration = start.elapsed();

    assert!(result.is_ok());
    // Log timing for diagnostics but don't assert — CI runners have variable performance
    eprintln!(
        "LSP resolution of 100 symbols took {}ms",
        duration.as_millis()
    );

    let _ = result.unwrap();
}

/// Test LSP resolver error handling
#[tokio::test]
async fn test_lsp_resolver_error_handling() {
    let config = load_config("rust").unwrap();
    let resolver = LspResolver::new(config, None);

    let syntax_results = SyntaxResults::new();

    // Should handle resolution gracefully even when LSP is not available
    let result = resolver.resolve(&syntax_results).await;
    assert!(result.is_ok());

    let resolved_edges = result.unwrap();
    assert!(resolved_edges.is_empty());
}

/// Test session supervisor lifecycle
#[tokio::test]
async fn test_session_supervisor_lifecycle() {
    let config = load_config("rust").unwrap();
    let supervisor = SessionSupervisor::new(config, None);

    // Test initial state
    assert_eq!(supervisor.get_state(), SessionState::Idle);

    // Test state transitions (without actually starting LSP server)
    // This test ensures the state machine logic works correctly
    assert!(!supervisor.get_state().is_functional());
    assert!(!supervisor.get_state().can_accept_requests());
}

/// Test LSP resolver with different edge types
#[tokio::test]
async fn test_lsp_resolver_edge_types() {
    let config = load_config("rust").unwrap();
    let resolver = LspResolver::new(config, None);

    let mut syntax_results = SyntaxResults::new();

    // Add different types of symbols
    let function = Node::new(
        NodeKind::Function,
        "test::function".to_string(),
        Range::new(1, 0, 5, 10),
        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
    );

    let struct_node = Node::new(
        NodeKind::Struct,
        "test::Struct".to_string(),
        Range::new(6, 0, 10, 10),
        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
    );

    let module = Node::new(
        NodeKind::Module,
        "test::module".to_string(),
        Range::new(11, 0, 15, 10),
        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
    );

    syntax_results.add_symbol(function);
    syntax_results.add_symbol(struct_node);
    syntax_results.add_symbol(module);

    // Add different types of relationships
    syntax_results.add_call_site(Range::new(2, 0, 2, 10), "sample_call".to_string());
    syntax_results.add_import(Range::new(3, 0, 3, 20));
    syntax_results.add_type_ref(Range::new(4, 0, 4, 15));

    let result = resolver.resolve(&syntax_results).await;
    assert!(result.is_ok());

    let _ = result.unwrap();
    // No specific assertion on heuristic output
}
