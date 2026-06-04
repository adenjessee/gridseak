#![cfg(feature = "e2e-tests")]

//! End-to-end integration tests for the complete parsing pipeline
//!
//! These tests verify the entire system works together from CLI input
//! to database output, including error handling and edge cases.

use graphengine_parsing::application::ports::GraphRepository;
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::Confidence;
use graphengine_parsing::infrastructure::SqliteRepository;
use tempfile::TempDir;

/// Test the complete parsing pipeline with a simple Rust crate
#[tokio::test]
async fn test_e2e_rust_parsing() {
    use graphengine_parsing::domain::EdgeKind;

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create a minimal crate with an explicit cross-function call so heuristics can resolve it.
    let rust_file = temp_dir.path().join("main").with_extension("rs");
    std::fs::write(
        &rust_file,
        r#"
fn helper() -> i32 {
    1
}

fn main() {
    let value = helper();
    println!("{}", value);
}
"#,
    )
    .unwrap();

    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .expect("create use case");

    let resolved = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await
        .expect("parse repository");

    let graph = resolved.graph();
    assert!(
        graph.node_count() >= 2,
        "expected functions to be discovered"
    );
    assert!(
        graph.edges.iter().any(|e| e.kind == EdgeKind::Call),
        "expected at least one call edge"
    );

    let stats = resolved.stats();
    assert!(
        stats.total_call_edges() >= 1,
        "expected call edges recorded in stats"
    );
}

/// Test error handling when parsing unsupported files
#[tokio::test]
async fn test_e2e_unsupported_files() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create an unsupported file
    let unsupported_file = temp_dir.path().join("test").with_extension("txt");
    std::fs::write(&unsupported_file, "This is not code").unwrap();

    // Parse the repository
    let language = String::from("rust");
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        language.clone(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), language.clone())
        .await;

    // Should succeed but with no nodes (unsupported files are skipped)
    assert!(result.is_ok(), "Should handle unsupported files gracefully");
    let _resolved_graph = result.unwrap();
    assert!(
        _resolved_graph.graph().nodes.is_empty(),
        "Should have no nodes for unsupported files"
    );
}

/// Test parsing with high confidence requirement
#[tokio::test]
async fn test_e2e_high_confidence_requirement() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create a simple Rust file
    let rust_file = temp_dir.path().join("main").with_extension("rs");
    std::fs::write(
        &rust_file,
        "fn main() {\n    println!(\"Hello, world!\");\n}\n",
    )
    .unwrap();

    // Parse with high confidence requirement
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::High,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await;

    assert!(result.is_ok(), "Should handle high confidence requirement ");
    let resolved_graph = result.unwrap();
    assert!(
        !resolved_graph.graph().nodes.is_empty(),
        "Even with high confidence requirement, nodes should be produced"
    );
}

/// Test database persistence and retrieval
#[tokio::test]
async fn test_e2e_database_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create a simple Rust file
    let rust_file = temp_dir.path().join("main").with_extension("rs");
    std::fs::write(
        &rust_file,
        r#"
fn main() {
    println!("Hello, world!");
}

fn helper() -> i32 {
    42
}
"#,
    )
    .unwrap();

    // Parse and store
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await;
    assert!(result.is_ok(), "Should parse successfully ");

    // Verify data was stored in database
    let repo = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();
    let node_ids = repo.list().await.unwrap();
    assert!(!node_ids.is_empty(), "Should have nodes in database ");

    // Try to retrieve a specific node
    if let Some(node_id) = node_ids.first() {
        let retrieved_graph = repo.get(node_id).await.unwrap();
        assert!(
            retrieved_graph.is_some(),
            "Should be able to retrieve stored graph "
        );
    }
}

/// Test parsing multiple files
#[tokio::test]
async fn test_e2e_multiple_files() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create multiple Rust files
    let main_file = temp_dir.path().join("main").with_extension("rs");
    std::fs::write(
        &main_file,
        r#"
mod utils;

fn main() {
    let result = utils::add(1, 2);
    println!("Result: {}", result);
}
"#,
    )
    .unwrap();

    let utils_file = temp_dir.path().join("utils").with_extension("rs");
    std::fs::write(
        &utils_file,
        r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#,
    )
    .unwrap();

    // Parse the repository
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await;
    assert!(result.is_ok(), "Should parse multiple files successfully ");

    let _resolved_graph = result.unwrap();
    let graph = _resolved_graph.graph();

    // Should have nodes from both files
    assert!(
        graph.nodes.len() >= 3,
        "Should have nodes from multiple files "
    );

    // Should have edges between files (calls from main to utils)
    assert!(!graph.edges.is_empty(), "Should have edges between files ");
}

/// Test error handling with invalid configuration
#[tokio::test]
async fn test_e2e_invalid_configuration() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Try to parse with unsupported language
    let result = ParseRepositoryUseCase::with_sqlite_storage(
        "unsupported_language".to_string(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await;

    assert!(result.is_err(), "Should fail with unsupported language ");
}

/// Test parsing with empty directory
#[tokio::test]
async fn test_e2e_empty_directory() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Parse empty directory
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await;
    assert!(result.is_ok(), "Should handle empty directory gracefully ");

    let _resolved_graph = result.unwrap();
    assert!(
        _resolved_graph.graph().nodes.is_empty(),
        "Should have no nodes in empty directory "
    );
}

/// Test parsing with mixed file types
#[tokio::test]
async fn test_e2e_mixed_file_types() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create mixed file types
    let rust_file = temp_dir.path().join("main").with_extension("rs");
    std::fs::write(
        &rust_file,
        r#"
fn main() {
    println!("Hello, world!");
}
"#,
    )
    .unwrap();

    let text_file = temp_dir.path().join("README").with_extension("txt");
    std::fs::write(&text_file, "This is a README file ").unwrap();

    let json_file = temp_dir.path().join("config").with_extension("json");
    std::fs::write(&json_file, r#"{"name": "test"}"#).unwrap();

    // Parse the repository
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await;
    assert!(result.is_ok(), "Should handle mixed file types gracefully ");

    let _resolved_graph = result.unwrap();
    // Should only have nodes from the Rust file
    assert!(
        !_resolved_graph.graph().nodes.is_empty(),
        "Should have nodes from Rust file "
    );
}

/// Test parsing with nested directories
#[tokio::test]
async fn test_e2e_nested_directories() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test").with_extension("db");

    // Create nested directory structure
    let nested_dir = temp_dir.path().join("src").join("lib");
    std::fs::create_dir_all(&nested_dir).unwrap();

    let main_file = temp_dir.path().join("main").with_extension("rs");
    std::fs::write(
        &main_file,
        "mod lib;\n\nfn main() {\n    lib::hello();\n}\n",
    )
    .unwrap();

    let lib_file = nested_dir.join("mod").with_extension("rs");
    std::fs::write(
        &lib_file,
        "pub fn hello() {\n    println!(\"Hello from lib!\");\n}\n",
    )
    .unwrap();

    // Parse the repository
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        String::from("rust"),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), String::from("rust"))
        .await;
    assert!(result.is_ok());

    let resolved_graph = result.unwrap();
    assert!(!resolved_graph.graph().nodes.is_empty());
}
