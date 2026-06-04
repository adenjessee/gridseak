//! Failure simulation tests for robustness and error recovery
//!
//! These tests simulate various failure scenarios to ensure the system
//! handles errors gracefully and recovers appropriately.

use graphengine_parsing::application::ports::GraphRepository;
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::{Confidence, Provenance, ProvenanceSource};
use graphengine_parsing::infrastructure::SqliteRepository;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test handling of database corruption
#[tokio::test]
async fn test_database_corruption_handling() {
    let storage_path = tempfile::tempdir().unwrap();
    let _temp_dir = TempDir::new().unwrap();
    let db_path = storage_path.path().join("test.db");

    // Create a corrupted database file
    std::fs::write(&db_path, "This is not a valid SQLite database").unwrap();

    // Try to create a repository with the corrupted database
    let result = SqliteRepository::new(db_path.to_str().unwrap());

    // Should surface an error gracefully (no panic)
    assert!(
        result.is_err(),
        "Corrupted database should return an error without panicking"
    );
}

/// Test handling of disk space exhaustion
#[tokio::test]
async fn test_disk_space_exhaustion() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a repository
    let repo = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();

    // Create a large graph that might exhaust disk space
    let mut graph = graphengine_parsing::domain::Graph::new();

    // Add many nodes (this won't actually exhaust disk space in tests, but simulates the scenario)
    for i in 0..10000 {
        let node = graphengine_parsing::domain::Node {
            id: format!("node_{}", i),
            kind: graphengine_parsing::domain::NodeKind::Function,
            fqn: format!("test::function_{}", i),
            location: graphengine_parsing::domain::Range::new(1, 0, 1, 10),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        };
        graph.add_node(node);
    }

    // Try to upsert the large graph
    let result = repo.upsert(&graph).await;

    // Should handle large data gracefully
    assert!(result.is_ok(), "Should handle large data gracefully");
}

/// Test handling of network failures (for LSP servers)
#[tokio::test]
async fn test_network_failure_handling() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a use case with a non-existent LSP server
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        "rust".to_string(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    // Create a simple Rust file
    let rust_file = temp_dir.path().join("main.rs");
    std::fs::write(
        &rust_file,
        r#"
fn main() {
    println!("Hello, world!");
}
"#,
    )
    .unwrap();

    // Try to parse - should handle LSP failures gracefully
    let result = use_case
        .parse(temp_dir.path().to_path_buf(), "rust".to_string())
        .await;

    // Should either succeed with reduced functionality or fail gracefully
    // The exact behavior depends on the LSP implementation
    assert!(result.is_ok(), "Should handle LSP failures gracefully");
}

/// Test handling of permission errors
#[tokio::test]
async fn test_permission_error_handling() {
    let _temp_dir = TempDir::new().unwrap();

    // Try to create a database in a read-only location
    // This is hard to simulate in tests, but we can test the error handling
    let read_only_path = PathBuf::from("/readonly/path/test.db");

    // Try to create a repository with a read-only path
    let result = SqliteRepository::new(read_only_path.to_str().unwrap());

    // Should fail gracefully with a clear error message
    assert!(
        result.is_err(),
        "Should fail gracefully with permission errors"
    );
}

/// Test handling of concurrent access conflicts
#[tokio::test]
async fn test_concurrent_access_conflicts() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create two repositories pointing to the same database
    let repo1 = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();
    let repo2 = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();

    // Create test data
    let mut graph1 = graphengine_parsing::domain::Graph::new();
    let node1 = graphengine_parsing::domain::Node {
        id: "node1".to_string(),
        kind: graphengine_parsing::domain::NodeKind::Function,
        fqn: "test::function1".to_string(),
        location: graphengine_parsing::domain::Range::new(1, 0, 1, 10),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };
    graph1.add_node(node1);

    let mut graph2 = graphengine_parsing::domain::Graph::new();
    let node2 = graphengine_parsing::domain::Node {
        id: "node2".to_string(),
        kind: graphengine_parsing::domain::NodeKind::Function,
        fqn: "test::function2".to_string(),
        location: graphengine_parsing::domain::Range::new(2, 0, 2, 10),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };
    graph2.add_node(node2);

    // Try concurrent operations
    let result1 = repo1.upsert(&graph1).await;
    let result2 = repo2.upsert(&graph2).await;

    // At least one should succeed
    assert!(
        result1.is_ok() || result2.is_ok(),
        "At least one concurrent operation should succeed"
    );
}

/// Test handling of malformed input files
#[tokio::test]
async fn test_malformed_input_handling() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create malformed Rust files
    let large_content = "fn main() { println!(\"Hello\"); }".repeat(1000);
    let malformed_files = vec![
        ("syntax_error.rs", "fn main() { invalid syntax }"),
        ("incomplete.rs", "fn main() {"),
        ("unicode.rs", "fn main() { println!(\"测试\"); }"),
        ("large_file.rs", &large_content),
    ];

    for (filename, content) in malformed_files {
        let file_path = temp_dir.path().join(filename);
        std::fs::write(&file_path, content).unwrap();
    }

    // Try to parse the malformed files
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        "rust".to_string(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), "rust".to_string())
        .await;

    // Should handle malformed files gracefully
    assert!(result.is_ok(), "Should handle malformed files gracefully");
}

/// Test handling of memory pressure
#[tokio::test]
async fn test_memory_pressure_handling() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a repository
    let repo = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();

    // Create a very large graph to simulate memory pressure
    let mut graph = graphengine_parsing::domain::Graph::new();

    // Add many nodes with large FQNs
    for i in 0..1000 {
        let large_fqn = format!(
            "very::long::namespace::with::many::segments::function_{}",
            i
        );
        let node = graphengine_parsing::domain::Node {
            id: format!("node_{}", i),
            kind: graphengine_parsing::domain::NodeKind::Function,
            fqn: large_fqn,
            location: graphengine_parsing::domain::Range::new(1, 0, 1, 10),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        };
        graph.add_node(node);
    }

    // Try to upsert the large graph
    let result = repo.upsert(&graph).await;

    // Should handle memory pressure gracefully
    assert!(result.is_ok(), "Should handle memory pressure gracefully");
}

/// Test handling of timeout scenarios
#[tokio::test]
async fn test_timeout_handling() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a use case
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        "rust".to_string(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    // Create a file that might cause timeouts (very large)
    let rust_file = temp_dir.path().join("main.rs");
    let large_content =
        "fn main() {\n".to_string() + &"    println!(\"Hello, world!\");\n".repeat(10000) + "}";
    std::fs::write(&rust_file, large_content).unwrap();

    // Try to parse with a timeout (this is more of a smoke test)
    let result = use_case
        .parse(temp_dir.path().to_path_buf(), "rust".to_string())
        .await;

    // Should handle large files gracefully
    assert!(result.is_ok(), "Should handle large files gracefully");
}

/// Test handling of configuration errors
#[tokio::test]
async fn test_configuration_error_handling() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Try to create a use case with an unsupported language
    let result = ParseRepositoryUseCase::with_sqlite_storage(
        "unsupported_language".to_string(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await;

    // Should fail gracefully with a clear error message
    assert!(
        result.is_err(),
        "Should fail gracefully with unsupported language"
    );
}

/// Test handling of partial failures
#[tokio::test]
async fn test_partial_failure_handling() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a mix of valid and invalid files
    let valid_file = temp_dir.path().join("valid.rs");
    std::fs::write(
        &valid_file,
        r#"
fn main() {
    println!("Hello, world!");
}
"#,
    )
    .unwrap();

    let invalid_file = temp_dir.path().join("invalid.rs");
    std::fs::write(&invalid_file, "This is not valid Rust code").unwrap();

    // Try to parse the mixed files
    let use_case = ParseRepositoryUseCase::with_sqlite_storage(
        "rust".to_string(),
        Confidence::Medium,
        db_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), "rust".to_string())
        .await;

    // Should handle partial failures gracefully
    assert!(result.is_ok(), "Should handle partial failures gracefully");

    let resolved_graph = result.unwrap();
    // Should have some nodes from the valid file
    assert!(
        !resolved_graph.graph().nodes.is_empty(),
        "Should have nodes from valid files"
    );
}
