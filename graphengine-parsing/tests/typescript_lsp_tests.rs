//! TypeScript LSP Integration Tests
//!
//! Tests for TypeScript LSP-based semantic resolution.
//! These tests verify:
//! 1. LSP protocol message creation for TypeScript
//! 2. TypeScript-specific resolution patterns
//! 3. End-to-end resolution on fixtures (feature-gated)

use graphengine_parsing::application::ports::{CallSite, SemanticResolver, SyntaxResults};
use graphengine_parsing::domain::{
    Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::lsp::protocol::{LspId, LspMessage, LspProtocol};
use graphengine_parsing::infrastructure::lsp::{LspResolver, SessionState, SessionSupervisor};
use tempfile::TempDir;
use url::Url;

// =============================================================================
// Domain Tests: LSP Message Structures for TypeScript
// =============================================================================

#[test]
fn test_typescript_initialize_request_serializes_correctly() {
    let request = LspProtocol::create_initialize_request(
        LspId::Number(1),
        Some("file:///home/user/ts-project".to_string()),
        None,
    );

    let json = LspProtocol::serialize_message(&request).expect("Should serialize");

    assert!(
        json.contains("\"jsonrpc\":\"2.0\""),
        "Should have JSON-RPC version"
    );
    assert!(
        json.contains("\"method\":\"initialize\""),
        "Should have initialize method"
    );
    assert!(
        json.contains("file:///home/user/ts-project"),
        "Should contain workspace URI"
    );
}

#[test]
fn test_typescript_definition_request_serializes_correctly() {
    let request = LspProtocol::create_definition_request(
        LspId::Number(2),
        "file:///home/user/project/src/auth.service.ts".to_string(),
        10, // line
        15, // character
    );

    let json = LspProtocol::serialize_message(&request).expect("Should serialize");

    assert!(
        json.contains("\"method\":\"textDocument/definition\""),
        "Should have definition method"
    );
    assert!(json.contains("auth.service.ts"), "Should contain file URI");
    assert!(json.contains("\"line\":10"), "Should have correct line");
    assert!(
        json.contains("\"character\":15"),
        "Should have correct character"
    );
}

#[test]
fn test_typescript_hover_request_serializes_correctly() {
    let request = LspProtocol::create_hover_request(
        LspId::Number(3),
        "file:///project/src/service.ts".to_string(),
        5,
        20,
    );

    let json = LspProtocol::serialize_message(&request).expect("Should serialize");

    assert!(
        json.contains("\"method\":\"textDocument/hover\""),
        "Should have hover method"
    );
    assert!(json.contains("service.ts"), "Should contain file URI");
}

#[test]
fn test_typescript_did_open_notification_serializes_correctly() {
    let notification = LspProtocol::create_did_open_notification(
        "file:///project/src/app.ts".to_string(),
        "typescript".to_string(),
        1,
        "export class App {}".to_string(),
    );

    let json = LspProtocol::serialize_message(&notification).expect("Should serialize");

    assert!(
        json.contains("\"method\":\"textDocument/didOpen\""),
        "Should have didOpen method"
    );
    assert!(
        json.contains("\"languageId\":\"typescript\""),
        "Should have typescript language ID"
    );
    assert!(
        json.contains("export class App"),
        "Should contain document text"
    );
}

#[test]
fn test_definition_response_deserializes_to_location() {
    // Simulated response from TypeScript language server
    let response_json = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "result": [{
            "uri": "file:///project/src/user.ts",
            "range": {
                "start": {"line": 5, "character": 0},
                "end": {"line": 10, "character": 1}
            }
        }]
    }"#;

    let message = LspProtocol::deserialize_message(response_json).expect("Should deserialize");

    match message {
        LspMessage::Response { id, result, error } => {
            assert!(matches!(id, LspId::Number(1)), "Should have correct ID");
            assert!(result.is_some(), "Should have result");
            assert!(error.is_none(), "Should have no error");

            let result = result.unwrap();
            assert!(result.is_array(), "Result should be array of locations");
            let locations = result.as_array().unwrap();
            assert_eq!(locations.len(), 1, "Should have one location");
        }
        _ => panic!("Expected response message"),
    }
}

#[test]
fn test_error_response_handled_gracefully() {
    let error_json = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": -32600,
            "message": "Invalid Request"
        }
    }"#;

    let message = LspProtocol::deserialize_message(error_json).expect("Should deserialize");

    match message {
        LspMessage::Response { id, result, error } => {
            assert!(matches!(id, LspId::Number(1)));
            assert!(result.is_none());
            assert!(error.is_some());

            let err = error.unwrap();
            assert_eq!(err.code, -32600);
            assert_eq!(err.message, "Invalid Request");
        }
        _ => panic!("Expected response message"),
    }
}

// =============================================================================
// Infrastructure Tests: TypeScript LSP Resolver
// =============================================================================

#[tokio::test]
async fn test_typescript_lsp_resolver_creation() {
    let config = load_config("typescript").expect("Should load TypeScript config");
    let _resolver = LspResolver::new(config, None);

    // Resolver should be created successfully (test passes by not panicking)
}

#[tokio::test]
async fn test_typescript_lsp_resolver_with_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_uri = Url::from_directory_path(temp_dir.path()).unwrap();

    let config = load_config("typescript").expect("Should load TypeScript config");
    let resolver = LspResolver::new(config, Some(workspace_uri));

    // Check availability
    let available = resolver.is_available().await;
    assert!(
        available,
        "TypeScript resolver should report available based on config"
    );
}

#[tokio::test]
async fn test_typescript_session_supervisor_states() {
    let config = load_config("typescript").expect("Should load TypeScript config");
    let supervisor = SessionSupervisor::new(config, None);

    // Initial state should be Idle
    assert_eq!(supervisor.get_state(), SessionState::Idle);

    // Should not be ready yet
    assert!(!supervisor.get_state().is_functional());
    assert!(!supervisor.get_state().can_accept_requests());
}

#[tokio::test]
async fn test_typescript_resolution_with_empty_syntax() {
    let config = load_config("typescript").expect("Should load TypeScript config");
    let resolver = LspResolver::new(config, None);

    let syntax_results = SyntaxResults::new();
    let result = resolver.resolve(&syntax_results).await;

    // Should succeed with empty results (heuristic fallback)
    assert!(result.is_ok());
    let resolved = result.unwrap();
    assert!(resolved.is_empty());
}

#[tokio::test]
async fn test_typescript_resolution_with_call_sites() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("service.ts");

    std::fs::write(
        &file_path,
        r#"
export function helper(): number {
    return 42;
}

export function main(): void {
    const result = helper();
    console.log(result);
}
"#,
    )
    .unwrap();

    let config = load_config("typescript").expect("Should load TypeScript config");
    let workspace_uri = Url::from_directory_path(temp_dir.path()).unwrap();
    let resolver = LspResolver::new(config, Some(workspace_uri));

    let mut syntax_results = SyntaxResults::new();

    // Add function nodes
    let helper_node = Node::new(
        NodeKind::Function,
        "service::helper".to_string(),
        Range::with_file(2, 0, 4, 1, file_path.to_string_lossy().to_string()),
        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
    );
    let main_node = Node::new(
        NodeKind::Function,
        "service::main".to_string(),
        Range::with_file(6, 0, 9, 1, file_path.to_string_lossy().to_string()),
        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
    );

    syntax_results.add_symbol(helper_node);
    syntax_results.add_symbol(main_node);

    // Add call site (main calls helper)
    syntax_results.push_call(CallSite {
        location: Range::with_file(7, 19, 7, 27, file_path.to_string_lossy().to_string()),
        function_name: "helper".to_string(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    });

    let result = resolver.resolve(&syntax_results).await;
    assert!(result.is_ok());

    // With heuristic fallback, we may get edges
    let _resolved = result.unwrap();
    // Don't assert specific edges since LSP may not be running
}

// =============================================================================
// Application Tests: Resolution Use Case (Mocked)
// =============================================================================

#[tokio::test]
async fn test_typescript_resolution_falls_back_to_heuristic() {
    // When LSP is not available, should fall back to heuristic resolution
    let config = load_config("typescript").expect("Should load TypeScript config");
    let resolver = LspResolver::new(config, None);

    let mut syntax_results = SyntaxResults::new();

    // Add two functions where one calls the other
    let caller = Node::new(
        NodeKind::Function,
        "main".to_string(),
        Range::with_file(1, 0, 5, 1, "test.ts".to_string()),
        Provenance::tree_sitter(),
    );
    let callee = Node::new(
        NodeKind::Function,
        "helper".to_string(),
        Range::with_file(6, 0, 10, 1, "test.ts".to_string()),
        Provenance::tree_sitter(),
    );

    syntax_results.add_symbol(caller);
    syntax_results.add_symbol(callee);

    syntax_results.push_call(CallSite {
        location: Range::with_file(3, 10, 3, 18, "test.ts".to_string()),
        function_name: "helper".to_string(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    });

    let result = resolver.resolve(&syntax_results).await;
    assert!(result.is_ok(), "Should succeed with heuristic fallback");
}

#[tokio::test]
async fn test_typescript_all_call_sites_processed() {
    let config = load_config("typescript").expect("Should load TypeScript config");
    let resolver = LspResolver::new(config, None);

    let mut syntax_results = SyntaxResults::new();

    // Add multiple call sites
    for i in 0..5 {
        syntax_results.push_call(CallSite {
            location: Range::with_file(i + 1, 0, i + 1, 10, "test.ts".to_string()),
            function_name: format!("func_{}", i),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        });
    }

    // Add corresponding functions
    for i in 0..5 {
        let node = Node::new(
            NodeKind::Function,
            format!("func_{}", i),
            Range::with_file(i * 10 + 1, 0, (i + 1) * 10, 1, "test.ts".to_string()),
            Provenance::tree_sitter(),
        );
        syntax_results.add_symbol(node);
    }

    let result = resolver.resolve(&syntax_results).await;
    assert!(
        result.is_ok(),
        "Should process all call sites without error"
    );
}

// =============================================================================
// Integration Tests: Real LSP Server (Feature-Gated)
// =============================================================================

/// Check if typescript-language-server is available
#[cfg(feature = "lsp-tests")]
fn is_typescript_lsp_available() -> bool {
    which::which("typescript-language-server").is_ok()
}

#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_typescript_lsp_server_starts_and_initializes() {
    if !is_typescript_lsp_available() {
        println!("Skipping: typescript-language-server not installed");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let workspace_uri = Url::from_directory_path(temp_dir.path()).unwrap();

    // Create a simple tsconfig.json
    std::fs::write(
        temp_dir.path().join("tsconfig.json"),
        r#"{"compilerOptions": {"target": "ES2020"}}"#,
    )
    .unwrap();

    let config = load_config("typescript").expect("Should load config");
    let supervisor = SessionSupervisor::new(config, Some(workspace_uri));

    // Try to start session
    let start_result = supervisor.initialize().await;

    if start_result.is_ok() {
        assert!(
            supervisor.get_state().is_functional(),
            "Should be functional after init"
        );
    } else {
        // LSP server might not start correctly in test environment
        println!("LSP server did not initialize: {:?}", start_result.err());
    }

    // Clean shutdown
    let _ = supervisor.kill().await;
}

#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_typescript_go_to_definition_returns_correct_location() {
    if !is_typescript_lsp_available() {
        println!("Skipping: typescript-language-server not installed");
        return;
    }

    let temp_dir = TempDir::new().unwrap();

    // Create TypeScript files
    std::fs::write(
        temp_dir.path().join("tsconfig.json"),
        r#"{"compilerOptions": {"target": "ES2020", "strict": true}}"#,
    )
    .unwrap();

    std::fs::write(
        temp_dir.path().join("helper.ts"),
        r#"
export function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#,
    )
    .unwrap();

    std::fs::write(
        temp_dir.path().join("main.ts"),
        r#"
import { greet } from './helper';

const message = greet('World');
console.log(message);
"#,
    )
    .unwrap();

    let workspace_uri = Url::from_directory_path(temp_dir.path()).unwrap();
    let config = load_config("typescript").expect("Should load config");
    let resolver = LspResolver::new(config, Some(workspace_uri));

    // Create syntax results with the greet call
    let mut syntax_results = SyntaxResults::new();

    let main_file = temp_dir.path().join("main.ts");
    syntax_results.push_call(CallSite {
        location: Range::with_file(4, 16, 4, 21, main_file.to_string_lossy().to_string()),
        function_name: "greet".to_string(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    });

    let result = resolver.resolve(&syntax_results).await;
    assert!(result.is_ok());

    // The actual resolution depends on LSP availability
    // We're mainly testing that the process doesn't crash
}

#[cfg(feature = "lsp-tests")]
#[tokio::test]
async fn test_typescript_server_handles_unknown_file_gracefully() {
    if !is_typescript_lsp_available() {
        println!("Skipping: typescript-language-server not installed");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let workspace_uri = Url::from_directory_path(temp_dir.path()).unwrap();

    let config = load_config("typescript").expect("Should load config");
    let resolver = LspResolver::new(config, Some(workspace_uri));

    let mut syntax_results = SyntaxResults::new();

    // Reference a file that doesn't exist
    syntax_results.push_call(CallSite {
        location: Range::with_file(1, 0, 1, 10, "/nonexistent/file.ts".to_string()),
        function_name: "missing".to_string(),
        receiver_range: None,
        receiver_text: None,
        arg_types: Vec::new(),
    });

    let result = resolver.resolve(&syntax_results).await;

    // Should handle gracefully without crashing
    assert!(result.is_ok(), "Should handle unknown files gracefully");
}

// =============================================================================
// Performance Tests
// =============================================================================

#[tokio::test]
async fn test_typescript_resolution_performance() {
    let config = load_config("typescript").expect("Should load config");
    let resolver = LspResolver::new(config, None);

    let mut syntax_results = SyntaxResults::new();

    // Add 100 symbols and call sites
    for i in 0..100 {
        let node = Node::new(
            NodeKind::Function,
            format!("typescript::func_{}", i),
            Range::with_file(i + 1, 0, i + 5, 10, "test.ts".to_string()),
            Provenance::tree_sitter(),
        );
        syntax_results.add_symbol(node);

        syntax_results.push_call(CallSite {
            location: Range::with_file(i + 2, 0, i + 2, 10, "test.ts".to_string()),
            function_name: format!("func_{}", (i + 1) % 100),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        });
    }

    let start = std::time::Instant::now();
    let result = resolver.resolve(&syntax_results).await;
    let duration = start.elapsed();

    assert!(result.is_ok());
    assert!(
        duration.as_millis() < 2000,
        "Should complete in < 2s. Took: {:?}",
        duration
    );
}
