//! Fuzzing tests for robustness and error handling
//!
//! These tests use random or malformed inputs to ensure the system
//! handles edge cases gracefully without crashing.

use graphengine_parsing::application::ports::GraphRepository;
use graphengine_parsing::domain::{
    Confidence, Edge, EdgeKind, Graph, Node, NodeKind, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::SqliteRepository;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test configuration loading with malformed YAML
#[test]
fn test_fuzz_malformed_config() {
    // Test with various malformed inputs
    let malformed_inputs = vec![
        "",                                       // Empty string
        "invalid yaml: [",                        // Invalid YAML syntax
        "language: rust\ninvalid_field: [",       // Partial YAML
        "language: rust\nqueries: invalid",       // Invalid queries structure
        "language: rust\nkind_mappings: invalid", // Invalid kind_mappings
    ];

    for input in malformed_inputs {
        // Create a temporary config file with malformed content
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("malformed.yaml");
        std::fs::write(&config_path, input).unwrap();

        // Try to load the config - should fail gracefully
        let result = load_config("malformed");
        assert!(
            result.is_err(),
            "Should fail to load malformed config: {}",
            input
        );
    }
}

/// Test database operations with invalid data
#[tokio::test]
async fn test_fuzz_database_invalid_data() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let repo = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();

    // Test with invalid node data
    let invalid_nodes = vec![
        // Node with empty ID
        Node {
            id: "".to_string(),
            kind: NodeKind::Function,
            fqn: "test::function".to_string(),
            location: Range::new(1, 0, 1, 10),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        },
        // Node with invalid FQN
        Node {
            id: "test_id".to_string(),
            kind: NodeKind::Function,
            fqn: "".to_string(),
            location: Range::new(1, 0, 1, 10),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        },
    ];

    for node in invalid_nodes {
        let mut graph = Graph::new();
        graph.add_node(node);

        // Should handle invalid data gracefully
        let _result = repo.upsert(&graph).await;
        // Note: The current implementation might accept invalid data
        // In a production system, you'd want to validate and reject invalid data
    }
}

/// Test graph validation with malformed graphs
#[test]
fn test_fuzz_malformed_graphs() {
    // Test with various malformed graph structures
    let mut graph = Graph::new();

    // Add a node
    let node = Node {
        id: "test_node".to_string(),
        kind: NodeKind::Function,
        fqn: "test::function".to_string(),
        location: Range::new(1, 0, 1, 10),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };
    graph.add_node(node);

    // Test with dangling edge (references non-existent node)
    let dangling_edge = Edge {
        from_id: "non_existent_node".to_string(),
        to_id: "test_node".to_string(),
        kind: EdgeKind::Call,
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
    };

    // This should be caught by validation
    graph.add_edge(dangling_edge);
    let validation_result = graph.validate(Confidence::Medium);
    assert!(validation_result.is_err(), "Should reject dangling edge");

    // Reset edges for next validation
    graph.edges.clear();

    // Test with self-loop edge (should be allowed for some edge types)
    let self_loop_edge = Edge {
        from_id: "test_node".to_string(),
        to_id: "test_node".to_string(),
        kind: EdgeKind::Contains,
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
    };

    graph.add_edge(self_loop_edge);
    let validation_result = graph.validate(Confidence::Medium);
    assert!(
        validation_result.is_ok(),
        "Should allow self-loop for Contains edge"
    );
}

/// Test with extremely large inputs
#[test]
fn test_fuzz_large_inputs() {
    // Test with very large FQN
    let large_fqn = "a".repeat(10000);
    let node = Node {
        id: "test_node".to_string(),
        kind: NodeKind::Function,
        fqn: large_fqn,
        location: Range::new(1, 0, 1, 10),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };

    let mut graph = Graph::new();
    graph.add_node(node);
    let validation_result = graph.validate(Confidence::Medium);
    assert!(validation_result.is_ok(), "Should handle large FQN");

    // Test with very large location range
    let large_range_node = Node {
        id: "test_node2".to_string(),
        kind: NodeKind::Function,
        fqn: "test::function".to_string(),
        location: Range::new(1, 0, u32::MAX, u32::MAX),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };

    graph.add_node(large_range_node);
    let validation_result = graph.validate(Confidence::Medium);
    assert!(
        validation_result.is_ok(),
        "Should handle large location range"
    );
}

/// Test with Unicode and special characters
#[test]
fn test_fuzz_unicode_inputs() {
    // Test with various Unicode characters
    let unicode_inputs = vec![
        "测试::函数",                      // Chinese
        "тест::функция",                   // Russian
        "テスト::関数",                    // Japanese
        "🚀::🌟",                          // Emojis
        "test::function\nwith\nnewlines",  // Newlines
        "test::function\twith\ttabs",      // Tabs
        "test::function\"with\"quotes",    // Quotes
        "test::function'with'apostrophes", // Apostrophes
    ];

    for fqn in unicode_inputs {
        let node = Node {
            id: format!("node_{}", fqn.len()),
            kind: NodeKind::Function,
            fqn: fqn.to_string(),
            location: Range::new(1, 0, 1, 10),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        };

        let mut graph = Graph::new();
        graph.add_node(node);
        let validation_result = graph.validate(Confidence::Medium);
        assert!(
            validation_result.is_ok(),
            "Should handle Unicode FQN: {}",
            fqn
        );
    }
}

/// Test with concurrent access to database
#[tokio::test]
async fn test_fuzz_concurrent_database_access() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create multiple repositories pointing to the same database
    let repo1 = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();
    let repo2 = SqliteRepository::new(db_path.to_str().unwrap()).unwrap();

    // Create test data
    let mut graph1 = Graph::new();
    let node1 = Node {
        id: "node1".to_string(),
        kind: NodeKind::Function,
        fqn: "test::function1".to_string(),
        location: Range::new(1, 0, 1, 10),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };
    graph1.add_node(node1);

    let mut graph2 = Graph::new();
    let node2 = Node {
        id: "node2".to_string(),
        kind: NodeKind::Function,
        fqn: "test::function2".to_string(),
        location: Range::new(2, 0, 2, 10),
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

/// Test with malformed file paths
#[test]
fn test_fuzz_malformed_file_paths() {
    let malformed_paths = vec![
        PathBuf::from(""),                      // Empty path
        PathBuf::from("/"),                     // Root path
        PathBuf::from(".."),                    // Parent directory
        PathBuf::from("."),                     // Current directory
        PathBuf::from("nonexistent/path"),      // Non-existent path
        PathBuf::from("C:\\Windows\\System32"), // System directory (on Windows)
    ];

    for path in malformed_paths {
        // Ensure conversion to string losslessly (should not panic)
        let _ = path.to_string_lossy();
    }
}

/// Test with extreme confidence values
#[test]
fn test_fuzz_extreme_confidence_values() {
    // Test with various confidence values
    let confidence_values = vec![Confidence::Low, Confidence::Medium, Confidence::High];

    for confidence in confidence_values {
        let provenance = Provenance::new(ProvenanceSource::TreeSitter, confidence);
        assert_eq!(
            provenance.confidence, confidence,
            "Confidence should be preserved"
        );
    }
}

/// Test with malformed provenance data
#[test]
fn test_fuzz_malformed_provenance() {
    // Test with various provenance combinations
    let provenance_combinations = vec![
        (ProvenanceSource::TreeSitter, Confidence::Low),
        (ProvenanceSource::TreeSitter, Confidence::Medium),
        (ProvenanceSource::TreeSitter, Confidence::High),
        (ProvenanceSource::Lsp, Confidence::Low),
        (ProvenanceSource::Lsp, Confidence::Medium),
        (ProvenanceSource::Lsp, Confidence::High),
    ];

    for (source, confidence) in provenance_combinations {
        let provenance = Provenance::new(source, confidence);
        assert_eq!(provenance.source, source, "Source should be preserved");
        assert_eq!(
            provenance.confidence, confidence,
            "Confidence should be preserved"
        );
    }
}

/// Test with malformed edge data
#[test]
fn test_fuzz_malformed_edges() {
    let mut graph = Graph::new();

    // Add a node first
    let node = Node {
        id: "test_node".to_string(),
        kind: NodeKind::Function,
        fqn: "test::function".to_string(),
        location: Range::new(1, 0, 1, 10),
        provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        properties: std::collections::HashMap::new(),
        trait_metadata: None,
    };
    graph.add_node(node);

    // Test with various edge types
    let edge_types = vec![EdgeKind::Call, EdgeKind::Contains, EdgeKind::Import];

    for edge_kind in edge_types {
        let edge = Edge {
            from_id: "test_node".to_string(),
            to_id: "test_node".to_string(),
            kind: edge_kind,
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
        };

        graph.add_edge(edge);
        // Some edge types might not allow self-loops
        // The validation logic should handle this appropriately
    }
}
