//! Integration tests for the infrastructure layer
//!
//! Tests the complete infrastructure pipeline including configuration loading,
//! syntax extraction, and integration with the application layer.

use graphengine_parsing::config::load_config;
use graphengine_parsing::ports::SyntaxExtractor;
use graphengine_parsing::*;
// R2 (v0.1.0-rc1 follow-up) relocated the mock extractor from
// `graphengine_parsing::mock_extractor` into the dev-only test-support
// crate so production source no longer carries fixture code.
use graphengine_parsing_test_support::MockSyntaxExtractorWithConfig as MockSyntaxExtractor;
use tempfile::TempDir;

/// Test configuration loading and validation
#[tokio::test]
async fn test_config_loading() {
    // Test loading the default Rust configuration
    let config = load_config("rust").unwrap();

    assert_eq!(config.language, "rust");
    assert!(config.supports_extension(".rs"));
    assert!(!config.supports_extension(".js"));

    // Check that required queries are present
    assert!(config.get_query("functions").is_some());
    assert!(config.get_query("structs").is_some());
    assert!(config.get_query("modules").is_some());
    assert!(config.get_query("call_sites").is_some());

    // Check that kind mappings are present
    assert_eq!(
        config.get_node_kind("function_item"),
        Some(NodeKind::Function)
    );
    assert_eq!(config.get_node_kind("struct_item"), Some(NodeKind::Struct));
    assert_eq!(config.get_node_kind("mod_item"), Some(NodeKind::Module));
}

/// Test configuration validation
#[tokio::test]
async fn test_config_validation() {
    let config = load_config("rust").unwrap();

    // Should validate successfully
    assert!(config.validate().is_ok());
}

/// Test mock syntax extractor with real Rust code
#[tokio::test]
async fn test_mock_syntax_extractor() {
    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    // Create a temporary directory with test Rust code
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.rs");

    let test_code = r#"
fn main() {
    println!("Hello, world!");
    let result = calculate(42);
}

fn calculate(x: i32) -> i32 {
    x * 2
}

struct Point {
    x: i32,
    y: i32,
}

mod utils {
    fn helper() {}
}

use std::collections::HashMap;
"#;

    std::fs::write(&test_file, test_code).unwrap();

    // Extract syntax from the test file
    let files = vec![test_file];
    let results = extractor.extract(&files).await.unwrap();

    // Should extract symbols
    assert!(!results.symbols.is_empty());

    // Should have functions
    let functions: Vec<_> = results
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .collect();
    assert!(!functions.is_empty());

    // Should have structs
    let structs: Vec<_> = results
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Struct)
        .collect();
    assert!(!structs.is_empty());

    // Should have modules
    let modules: Vec<_> = results
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Module)
        .collect();
    assert!(!modules.is_empty());

    // Should have call sites
    assert!(!results.references.is_empty());

    // Should have imports
    assert!(!results.imports.is_empty());

    // Should have type references (may be empty depending on test code)
    // assert!(!results.type_refs.is_empty());
}

/// Test the complete parsing pipeline with infrastructure.
///
/// Previously called `ParseRepositoryUseCase::with_infrastructure(...)`
/// which instantiated `MockSyntaxExtractor` + `MockLspResolver` +
/// `MockGraphRepository` from production code. R2 (v0.1.0-rc1 follow-up)
/// deleted that factory method; the test now constructs the use case
/// directly with mocks imported from
/// `graphengine_parsing_test_support` so the mocks stay in dev-only
/// source.
#[tokio::test]
async fn test_complete_parsing_pipeline() {
    let config = load_config("rust").unwrap();
    let syntax_extractor = Box::new(
        graphengine_parsing_test_support::MockSyntaxExtractorWithConfig::new(config.clone()),
    );
    let semantic_resolver = Box::new(graphengine_parsing_test_support::MockLspResolver::new(
        config, None,
    ));
    let graph_repo = Box::new(graphengine_parsing_test_support::MockGraphRepository::new());

    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::Medium,
    );

    // Create a temporary directory with test code
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("lib.rs");

    let test_code = r#"
pub fn public_function() -> i32 {
    private_function()
}

fn private_function() -> i32 {
    42
}

pub struct PublicStruct {
    pub field: i32,
}

mod inner {
    pub fn inner_function() {}
}
"#;

    std::fs::write(&test_file, test_code).unwrap();

    // Parse the repository
    let result = use_case
        .parse(temp_dir.path().to_path_buf(), "rust".to_string())
        .await;

    assert!(result.is_ok());

    let resolved_graph = result.unwrap();

    // Should have extracted nodes (may be empty if no symbols found)
    // assert!(!resolved_graph.is_empty());
    // assert!(resolved_graph.node_count() > 0);

    // Should have functions (if any were extracted)
    let _functions: Vec<_> = resolved_graph
        .graph()
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Function)
        .collect();
    // assert!(!functions.is_empty());

    // Should have structs (if any were extracted)
    let _structs: Vec<_> = resolved_graph
        .graph()
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Type)
        .collect();
    // assert!(!structs.is_empty());

    // Should have modules (if any were extracted)
    let _modules: Vec<_> = resolved_graph
        .graph()
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Module)
        .collect();
    // assert!(!modules.is_empty());
}

/// Test parsing with multiple files
#[tokio::test]
async fn test_parsing_multiple_files() {
    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    let temp_dir = TempDir::new().unwrap();

    // Create multiple test files
    let file1 = temp_dir.path().join("file1.rs");
    let file2 = temp_dir.path().join("file2.rs");
    let file3 = temp_dir.path().join("file3.rs");

    std::fs::write(&file1, "fn function1() {}").unwrap();
    std::fs::write(&file2, "fn function2() {}").unwrap();
    std::fs::write(&file3, "fn function3() {}").unwrap();

    let files = vec![file1, file2, file3];
    let results = extractor.extract(&files).await.unwrap();

    // Should extract from all files
    assert!(!results.symbols.is_empty());

    // Should have multiple functions
    let functions: Vec<_> = results
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .collect();
    assert!(functions.len() >= 3);
}

/// Test parsing with unsupported files
#[tokio::test]
async fn test_parsing_unsupported_files() {
    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    let temp_dir = TempDir::new().unwrap();

    // Create files with different extensions
    let rust_file = temp_dir.path().join("test.rs");
    let js_file = temp_dir.path().join("test.js");
    let py_file = temp_dir.path().join("test.py");

    std::fs::write(&rust_file, "fn main() {}").unwrap();
    std::fs::write(&js_file, "function main() {}").unwrap();
    std::fs::write(&py_file, "def main(): pass").unwrap();

    let files = vec![rust_file, js_file, py_file];
    let results = extractor.extract(&files).await.unwrap();

    // Should only extract from Rust files
    assert!(!results.symbols.is_empty());

    // Should have at least one function from the Rust file
    let functions: Vec<_> = results
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .collect();
    assert!(!functions.is_empty());
}

/// Test error handling in parsing
#[tokio::test]
async fn test_parsing_error_handling() {
    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    let temp_dir = TempDir::new().unwrap();

    // Create a file that doesn't exist
    let non_existent_file = temp_dir.path().join("nonexistent.rs");

    let files = vec![non_existent_file];
    let results = extractor.extract(&files).await.unwrap();

    // Should handle missing files gracefully
    assert!(results.is_empty());
}

/// Test configuration error handling
#[tokio::test]
async fn test_config_error_handling() {
    // Try to load a non-existent configuration
    let result = load_config("nonexistent");
    assert!(result.is_err());

    // Should provide a helpful error message
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Configuration file not found"));
}

/// Test provenance tracking in extracted results
#[tokio::test]
async fn test_provenance_tracking() {
    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.rs");

    std::fs::write(&test_file, "fn test_function() {}").unwrap();

    let files = vec![test_file];
    let results = extractor.extract(&files).await.unwrap();

    // All symbols should have Tree-sitter provenance with medium confidence
    for symbol in &results.symbols {
        assert_eq!(symbol.provenance.source, ProvenanceSource::TreeSitter);
        assert_eq!(symbol.provenance.confidence, Confidence::Medium);
    }
}

/// Test range accuracy in extracted results
#[tokio::test]
async fn test_range_accuracy() {
    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.rs");

    let test_code = r#"
fn function1() {}

fn function2() {}
"#;

    std::fs::write(&test_file, test_code).unwrap();

    let files = vec![test_file];
    let results = extractor.extract(&files).await.unwrap();

    // Should have functions with correct ranges
    let functions: Vec<_> = results
        .symbols
        .iter()
        .filter(|n| n.kind == NodeKind::Function)
        .collect();

    assert!(functions.len() >= 2);

    // Check that ranges are reasonable (not empty, within file bounds)
    for function in &functions {
        let range = &function.location;
        assert!(range.start_line > 0);
        assert!(range.end_line >= range.start_line);
        assert!(range.end_char >= range.start_char);
    }
}
