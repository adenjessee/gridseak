//! # GraphEngine Parsing
//!
//! A production-ready, language-agnostic code parsing system that extracts semantic graphs from source code repositories.
//! Built with Rust for performance and reliability, following clean architecture principles.
//!
//! ## Overview
//!
//! GraphEngine Parsing implements the "1024 plan" - a comprehensive refactoring that creates a modular, extensible system for parsing code into semantic graphs. The system uses a two-phase approach:
//!
//! 1. **FAST**: Tree-sitter based syntactic extraction for quick symbol discovery
//! 2. **SEM**: LSP-based semantic resolution for accurate relationship mapping
//!
//! ## Features
//!
//! - **Multi-language Support**: Rust, JavaScript/TypeScript, Python (extensible)
//! - **Clean Architecture**: Domain, Application, and Infrastructure layers
//! - **Production Ready**: Comprehensive error handling, logging, and security features
//! - **CLI Interface**: Command-line tool for parsing and querying
//! - **Persistent Storage**: SQLite-based graph storage with querying capabilities
//! - **Security**: LSP subprocess resource limits and sandboxing
//! - **Testing**: E2E tests, fuzzing, benchmarks, and failure simulation
//!
//! ## Architecture
//!
//! The system follows clean architecture principles with clear separation of concerns:
//!
//! ### Domain Layer
//! - Pure models: [`Node`], [`Edge`], [`Graph`], [`Provenance`], [`Confidence`]
//! - Business invariants and validation
//! - Language-agnostic abstractions
//!
//! ### Application Layer
//! - Use cases: [`ParseRepositoryUseCase`]
//! - Ports (traits): [`SyntaxExtractor`], [`SemanticResolver`], [`GraphRepository`]
//! - Orchestration and business logic
//!
//! ### Infrastructure Layer
//! - **Config**: YAML-based language configurations
//! - **Syntax**: Tree-sitter based extractors
//! - **Semantic**: LSP-based resolvers
//! - **Storage**: SQLite repositories
//! - **Security**: Process limits and monitoring
//!
//! ## Quick Start
//!
//! ```rust
//! use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
//! use graphengine_parsing::domain::Confidence;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create use case with SQLite storage
//!     let use_case = ParseRepositoryUseCase::with_sqlite_storage(
//!         "rust".to_string(),
//!         Confidence::Medium,
//!         "database.db",
//!     ).await?;
//!
//!     // Parse repository
//!     let graph = use_case.parse(
//!         std::path::PathBuf::from("/path/to/repo"),
//!         "rust".to_string(),
//!     ).await?;
//!
//!     println!("Parsed {} nodes and {} edges",
//!              graph.graph().node_count(),
//!              graph.graph().edge_count());
//!
//!     Ok(())
//! }
//! ```
//!
//! ## CLI Usage
//!
//! The crate also provides a command-line interface:
//!
//! ```bash
//! # Parse a repository
//! graphengine-parsing parse --root /path/to/repo --lang rust --db database.db
//!
//! # Query the database
//! graphengine-parsing query --db database.db --query-type list-nodes
//!
//! # List supported languages
//! graphengine-parsing languages
//! ```
//!
//! ## Configuration
//!
//! Language configurations are stored in YAML files under `configs/`. Each configuration defines:
//!
//! - File extensions
//! - LSP server commands
//! - Tree-sitter queries for symbol extraction
//! - Kind mappings to universal domain models
//!
//! ## Security Features
//!
//! - **Process Limits**: LSP subprocesses are limited in CPU, memory, and file usage
//! - **Sandboxing**: Isolated execution environments for language servers
//! - **Monitoring**: Health checks and automatic restart capabilities
//! - **Resource Protection**: Prevents resource exhaustion attacks
//!
//! ## Performance
//!
//! - **Parallel Processing**: Multi-threaded file parsing using Rayon
//! - **Incremental Parsing**: Skip unchanged files using content hashing
//! - **Batched Operations**: Efficient database operations with transactions
//! - **Memory Efficient**: Streaming processing for large repositories

pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod module_resolution;
pub mod symbol_index;
pub mod syntax;

// Re-export main types for convenience.
//
// `pub use application::mocks as application_mocks;` and
// `pub use infrastructure::mock_extractor;` were removed in R2
// (v0.1.0-rc1 follow-up). Tests and benches that previously reached
// for `graphengine_parsing::application_mocks::*` or
// `graphengine_parsing::mock_extractor::*` now consume the same
// types from the dev-only `graphengine-parsing-test-support` crate.
pub use application::errors as application_errors;
pub use application::ports;
pub use application::use_cases::parse_repo::*;
pub use domain::*;
pub use infrastructure::config;
pub use infrastructure::lsp;
pub use infrastructure::storage;
pub use infrastructure::utils;
pub use module_resolution::*;
pub use symbol_index::*;
pub use syntax::*;

#[cfg(test)]
mod tests {
    use super::domain::*;

    #[test]
    fn test_dangling_edge_error_display() {
        let error = ValidationError::DanglingEdge {
            from_id: "node_a".to_string(),
            to_id: "node_b".to_string(),
        };

        assert_eq!(error.to_string(), "Dangling edge: from node_a to node_b");
    }

    #[test]
    fn test_low_confidence_error_display() {
        let error = ValidationError::LowConfidence(5);

        assert_eq!(
            error.to_string(),
            "Low confidence: 5 elements below threshold"
        );
    }

    #[test]
    fn test_invalid_kind_error_display() {
        let error = ValidationError::InvalidKind("UnknownType".to_string());

        assert_eq!(error.to_string(), "Invalid kind: UnknownType");
    }

    #[test]
    fn test_error_equality() {
        let error1 = ValidationError::DanglingEdge {
            from_id: "a".to_string(),
            to_id: "b".to_string(),
        };
        let error2 = ValidationError::DanglingEdge {
            from_id: "a".to_string(),
            to_id: "b".to_string(),
        };
        let error3 = ValidationError::DanglingEdge {
            from_id: "x".to_string(),
            to_id: "y".to_string(),
        };

        assert_eq!(error1, error2);
        assert_ne!(error1, error3);
    }

    #[test]
    fn test_error_debug() {
        let error = ValidationError::LowConfidence(3);
        let debug_str = format!("{:?}", error);

        assert!(debug_str.contains("LowConfidence"));
        assert!(debug_str.contains("3"));
    }

    #[test]
    fn test_invalid_provenance_error_display() {
        let error =
            ValidationError::InvalidProvenance("Lsp source with Low confidence".to_string());

        assert_eq!(
            error.to_string(),
            "Invalid provenance: Lsp source with Low confidence"
        );
    }

    // Provenance tests
    #[test]
    fn test_provenance_creation() {
        let prov = Provenance::new(ProvenanceSource::TreeSitter, Confidence::High);
        assert_eq!(prov.source, ProvenanceSource::TreeSitter);
        assert_eq!(prov.confidence, Confidence::High);
    }

    #[test]
    fn test_provenance_convenience_methods() {
        let tree_sitter = Provenance::tree_sitter();
        assert_eq!(tree_sitter.source, ProvenanceSource::TreeSitter);
        assert_eq!(tree_sitter.confidence, Confidence::High);

        let lsp = Provenance::lsp();
        assert_eq!(lsp.source, ProvenanceSource::Lsp);
        assert_eq!(lsp.confidence, Confidence::High);

        let heuristic = Provenance::heuristic();
        assert_eq!(heuristic.source, ProvenanceSource::Heuristic);
        assert_eq!(heuristic.confidence, Confidence::Low);
    }

    #[test]
    fn test_confidence_ordering() {
        assert!(Confidence::High > Confidence::Medium);
        assert!(Confidence::Medium > Confidence::Low);
        assert!(Confidence::High > Confidence::Low);
        assert_eq!(Confidence::High, Confidence::High);
    }

    #[test]
    fn test_provenance_equality() {
        let prov1 = Provenance::new(ProvenanceSource::Lsp, Confidence::Medium);
        let prov2 = Provenance::new(ProvenanceSource::Lsp, Confidence::Medium);
        let prov3 = Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium);

        assert_eq!(prov1, prov2);
        assert_ne!(prov1, prov3);
    }

    #[test]
    fn test_provenance_debug() {
        let prov = Provenance::heuristic();
        let debug_str = format!("{:?}", prov);

        assert!(debug_str.contains("Heuristic"));
        assert!(debug_str.contains("Low"));
    }

    #[test]
    fn test_provenance_validation() {
        // Valid provenance combinations
        let tree_sitter_high = Provenance::new(ProvenanceSource::TreeSitter, Confidence::High);
        assert!(tree_sitter_high.validate().is_ok());

        let lsp_high = Provenance::new(ProvenanceSource::Lsp, Confidence::High);
        assert!(lsp_high.validate().is_ok());

        let heuristic_low = Provenance::new(ProvenanceSource::Heuristic, Confidence::Low);
        assert!(heuristic_low.validate().is_ok());

        // Invalid provenance combination
        let lsp_low = Provenance::new(ProvenanceSource::Lsp, Confidence::Low);
        assert!(lsp_low.validate().is_err());
    }

    // Node tests
    #[test]
    fn test_node_creation() {
        let location = Range::with_file(10, 5, 15, 20, "test.rs".to_string());
        let provenance = Provenance::tree_sitter();
        let node = Node::new(
            NodeKind::Function,
            "test::func".to_string(),
            location.clone(),
            provenance,
        );

        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.fqn, "test::func");
        assert_eq!(node.location, location);
        assert_eq!(node.provenance, provenance);
    }

    #[test]
    fn test_node_id_stability() {
        let location = Range::with_file(10, 5, 15, 20, "test.rs".to_string());
        let provenance = Provenance::tree_sitter();

        let node1 = Node::new(
            NodeKind::Function,
            "test::func".to_string(),
            location.clone(),
            provenance,
        );
        let node2 = Node::new(
            NodeKind::Function,
            "test::func".to_string(),
            location,
            provenance,
        );

        // Same input should produce same ID
        assert_eq!(node1.id, node2.id);
    }

    #[test]
    fn test_node_id_uniqueness_by_fqn() {
        // T2 contract: two nodes with the same FQN and no body hash to the
        // same ID regardless of location, because location is not part of
        // identity under the content-based scheme. IDs are uniquely
        // differentiated by FQN (for container / no-body nodes) or by
        // FQN + normalized body (for real symbols via `Node::with_body`).
        let location1 = Range::with_file(10, 5, 15, 20, "test.rs".to_string());
        let location2 = Range::with_file(11, 5, 15, 20, "test.rs".to_string());
        let provenance = Provenance::tree_sitter();

        let node1 = Node::new(
            NodeKind::Function,
            "test::func_a".to_string(),
            location1.clone(),
            provenance,
        );
        let node2 = Node::new(
            NodeKind::Function,
            "test::func_b".to_string(),
            location2,
            provenance,
        );
        // Different FQNs must differentiate.
        assert_ne!(node1.id, node2.id);

        // Same FQN + no body: IDs collapse across locations (T2 intent).
        let same_a = Node::new(
            NodeKind::Function,
            "test::func_a".to_string(),
            location1,
            provenance,
        );
        assert_eq!(node1.id, same_a.id);
    }

    #[test]
    fn test_node_id_differentiated_by_body() {
        // T2: when body is supplied, two nodes with the same FQN but
        // different bodies must have different IDs. This guards against the
        // degenerate case where FQN alone would collide (e.g., an overload
        // whose FQN didn't encode its signature).
        let location = Range::with_file(1, 0, 10, 0, "test.rs".to_string());
        let prov = Provenance::tree_sitter();

        let a = Node::with_body(
            NodeKind::Function,
            "pkg::overloaded".to_string(),
            location.clone(),
            prov,
            "fn overloaded(x: i32) { x + 1 }",
            Some("rust"),
        );
        let b = Node::with_body(
            NodeKind::Function,
            "pkg::overloaded".to_string(),
            location,
            prov,
            "fn overloaded(x: i32, y: i32) { x + y }",
            Some("rust"),
        );
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn test_node_convenience_methods() {
        let location = Range::with_file(10, 5, 15, 20, "test.rs".to_string());

        let func_node = Node::function("test::func".to_string(), location.clone());
        assert_eq!(func_node.kind, NodeKind::Function);
        assert_eq!(func_node.provenance.source, ProvenanceSource::TreeSitter);

        let struct_node = Node::struct_("test::MyStruct".to_string(), location.clone());
        assert_eq!(struct_node.kind, NodeKind::Struct);

        let module_node = Node::module("test::module".to_string(), location);
        assert_eq!(module_node.kind, NodeKind::Module);
    }

    #[test]
    fn test_range_creation() {
        let range = Range::with_file(10, 5, 15, 20, "test.rs".to_string());
        assert_eq!(range.start_line, 10);
        assert_eq!(range.start_char, 5);
        assert_eq!(range.end_line, 15);
        assert_eq!(range.end_char, 20);
    }

    #[test]
    fn test_node_kind_equality() {
        assert_eq!(NodeKind::Function, NodeKind::Function);
        assert_ne!(NodeKind::Function, NodeKind::Struct);
    }

    #[test]
    fn test_unicode_fqn_handling() {
        // Test with Unicode characters in FQN
        let unicode_fqn = "测试::模块::函数".to_string();
        let location = Range::with_file(1, 0, 5, 10, "test.rs".to_string());
        let provenance = Provenance::tree_sitter();

        let node = Node::new(
            NodeKind::Function,
            unicode_fqn.clone(),
            location.clone(),
            provenance,
        );

        // Should handle Unicode correctly
        assert_eq!(node.fqn, unicode_fqn);
        assert!(!node.id.is_empty());

        // ID should be stable for same Unicode input
        let node2 = Node::new(NodeKind::Function, unicode_fqn, location, provenance);
        assert_eq!(node.id, node2.id);
    }

    #[test]
    fn test_unicode_edge_cases() {
        // Test with various Unicode edge cases
        let test_cases = vec![
            "αβγ::δεζ::ηθι",            // Greek
            "функция::модуль::класс",   // Cyrillic
            "関数::モジュール::クラス", // Japanese
            "🚀::📦::⚡",               // Emoji
            "café::naïve::résumé",      // Accented characters
        ];

        for fqn in test_cases {
            let location = Range::with_file(1, 0, 1, 10, "test.rs".to_string());
            let provenance = Provenance::tree_sitter();
            let node = Node::new(NodeKind::Function, fqn.to_string(), location, provenance);

            // Should handle all Unicode correctly
            assert_eq!(node.fqn, fqn);
            assert!(!node.id.is_empty());
        }
    }

    // Edge tests
    #[test]
    fn test_edge_creation() {
        let provenance = Provenance::tree_sitter();
        let edge = Edge::new(
            "node_a".to_string(),
            "node_b".to_string(),
            EdgeKind::Call,
            provenance,
        );

        assert_eq!(edge.from_id, "node_a");
        assert_eq!(edge.to_id, "node_b");
        assert_eq!(edge.kind, EdgeKind::Call);
        assert_eq!(edge.provenance, provenance);
    }

    #[test]
    fn test_edge_convenience_methods() {
        let provenance = Provenance::lsp();

        let call_edge = Edge::call("func1".to_string(), "func2".to_string(), provenance);
        assert_eq!(call_edge.kind, EdgeKind::Call);

        let contains_edge = Edge::contains("module".to_string(), "func".to_string(), provenance);
        assert_eq!(contains_edge.kind, EdgeKind::Contains);

        let import_edge = Edge::import("file1".to_string(), "file2".to_string(), provenance);
        assert_eq!(import_edge.kind, EdgeKind::Import);

        let type_edge = Edge::type_("struct".to_string(), "trait".to_string(), provenance);
        assert_eq!(type_edge.kind, EdgeKind::Type);

        let uses_edge = Edge::uses("func".to_string(), "var".to_string(), provenance);
        assert_eq!(uses_edge.kind, EdgeKind::Uses);
    }

    #[test]
    fn test_edge_kind_equality() {
        assert_eq!(EdgeKind::Call, EdgeKind::Call);
        assert_ne!(EdgeKind::Call, EdgeKind::Contains);
    }

    #[test]
    fn test_edge_validation_allows_contains_self_loop() {
        let provenance = Provenance::tree_sitter();
        // This should not panic
        let edge = Edge::contains("module".to_string(), "module".to_string(), provenance);
        assert_eq!(edge.from_id, edge.to_id);
    }

    #[test]
    #[should_panic(expected = "Invalid self-loop for edge kind")]
    fn test_edge_validation_prevents_call_self_loop() {
        let provenance = Provenance::tree_sitter();
        // This should panic
        Edge::call("func".to_string(), "func".to_string(), provenance);
    }

    #[test]
    #[should_panic(expected = "Invalid self-loop for edge kind")]
    fn test_edge_validation_prevents_import_self_loop() {
        let provenance = Provenance::tree_sitter();
        // This should panic
        Edge::import("file".to_string(), "file".to_string(), provenance);
    }

    // Graph tests
    #[test]
    fn test_graph_creation() {
        let graph = Graph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_graph_add_node() {
        let mut graph = Graph::new();
        let node = Node::function(
            "test::func".to_string(),
            Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
        );

        graph.add_node(node.clone());
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.get_node(&node.id), Some(&node));
    }

    #[test]
    fn test_graph_add_edge() {
        let mut graph = Graph::new();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
        );
        let node2 = Node::function(
            "test::func2".to_string(),
            Range::with_file(6, 0, 10, 10, "test.rs".to_string()),
        );

        graph.add_node(node1.clone());
        graph.add_node(node2.clone());

        let edge = Edge::call(
            node1.id.clone(),
            node2.id.clone(),
            Provenance::tree_sitter(),
        );
        graph.add_edge(edge.clone());

        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.get_edges_from(&node1.id), vec![&edge]);
        assert_eq!(graph.get_edges_to(&node2.id), vec![&edge]);
    }

    #[test]
    fn test_graph_validation_success() {
        let mut graph = Graph::new();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
        );
        let node2 = Node::function(
            "test::func2".to_string(),
            Range::with_file(6, 0, 10, 10, "test.rs".to_string()),
        );

        graph.add_node(node1.clone());
        graph.add_node(node2.clone());

        let edge = Edge::call(node1.id, node2.id, Provenance::tree_sitter());
        graph.add_edge(edge);

        // Should validate successfully with high confidence requirement
        assert!(graph.validate(Confidence::High).is_ok());
    }

    #[test]
    fn test_graph_validation_dangling_edge() {
        let mut graph = Graph::new();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
        );

        graph.add_node(node1.clone());

        // Add edge to non-existent node
        let edge = Edge::call(
            node1.id.clone(),
            "nonexistent".to_string(),
            Provenance::tree_sitter(),
        );
        graph.add_edge(edge);

        // Should fail validation
        let result = graph.validate(Confidence::High);
        assert!(result.is_err());
        if let Err(ValidationError::DanglingEdge { from_id, to_id }) = result {
            assert_eq!(from_id, node1.id);
            assert_eq!(to_id, "nonexistent");
        }
    }

    #[test]
    fn test_graph_validation_low_confidence() {
        let mut graph = Graph::new();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
        );
        let node2 = Node::function(
            "test::func2".to_string(),
            Range::with_file(6, 0, 10, 10, "test.rs".to_string()),
        );

        graph.add_node(node1.clone());
        graph.add_node(node2.clone());

        // Add edge with low confidence
        let edge = Edge::call(node1.id, node2.id, Provenance::heuristic());
        graph.add_edge(edge);

        // Should fail validation with high confidence requirement
        let result = graph.validate(Confidence::High);
        assert!(result.is_err());
        if let Err(ValidationError::LowConfidence(count)) = result {
            assert_eq!(count, 1);
        }
    }

    #[test]
    fn test_graph_bfs() {
        let mut graph = Graph::new();

        // Create a simple graph: A -> B -> C
        let node_a = Node::function(
            "A".to_string(),
            Range::with_file(1, 0, 1, 1, "test.rs".to_string()),
        );
        let node_b = Node::function(
            "B".to_string(),
            Range::with_file(2, 0, 2, 1, "test.rs".to_string()),
        );
        let node_c = Node::function(
            "C".to_string(),
            Range::with_file(3, 0, 3, 1, "test.rs".to_string()),
        );

        graph.add_node(node_a.clone());
        graph.add_node(node_b.clone());
        graph.add_node(node_c.clone());

        graph.add_edge(Edge::call(
            node_a.id.clone(),
            node_b.id.clone(),
            Provenance::tree_sitter(),
        ));
        graph.add_edge(Edge::call(
            node_b.id.clone(),
            node_c.id.clone(),
            Provenance::tree_sitter(),
        ));

        let bfs_result = graph.bfs(&node_a.id);
        assert_eq!(bfs_result, vec![node_a.id, node_b.id, node_c.id]);
    }

    #[test]
    fn test_graph_dfs() {
        let mut graph = Graph::new();

        // Create a simple graph: A -> B -> C
        let node_a = Node::function(
            "A".to_string(),
            Range::with_file(1, 0, 1, 1, "test.rs".to_string()),
        );
        let node_b = Node::function(
            "B".to_string(),
            Range::with_file(2, 0, 2, 1, "test.rs".to_string()),
        );
        let node_c = Node::function(
            "C".to_string(),
            Range::with_file(3, 0, 3, 1, "test.rs".to_string()),
        );

        graph.add_node(node_a.clone());
        graph.add_node(node_b.clone());
        graph.add_node(node_c.clone());

        graph.add_edge(Edge::call(
            node_a.id.clone(),
            node_b.id.clone(),
            Provenance::tree_sitter(),
        ));
        graph.add_edge(Edge::call(
            node_b.id.clone(),
            node_c.id.clone(),
            Provenance::tree_sitter(),
        ));

        let dfs_result = graph.dfs(&node_a.id);
        assert_eq!(dfs_result, vec![node_a.id, node_b.id, node_c.id]);
    }

    #[test]
    fn test_graph_bfs_nonexistent_node() {
        let graph = Graph::new();
        let result = graph.bfs("nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn test_graph_default() {
        let graph = Graph::default();
        assert!(graph.is_empty());
    }

    // Property tests using proptest
    #[cfg(test)]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn test_node_id_stability_property(
                fqn in "[a-zA-Z_][a-zA-Z0-9_]*::[a-zA-Z_][a-zA-Z0-9_]*",
                start_line in 1u32..1000,
                start_char in 0u32..100,
                end_line in 1u32..1000,
                end_char in 0u32..100
            ) {
                let location = Range::with_file(start_line, start_char, end_line, end_char, "test.rs".to_string());
                let provenance = Provenance::tree_sitter();

                let node1 = Node::new(NodeKind::Function, fqn.clone(), location.clone(), provenance);
                let node2 = Node::new(NodeKind::Function, fqn, location, provenance);

                // Same input should always produce same ID
                prop_assert_eq!(node1.id, node2.id);
            }

            #[test]
            fn test_graph_validation_property(
                node_count in 2usize..50,  // Start from 2 to avoid single-node self-loop issues
                edge_count in 0usize..50
            ) {
                let mut graph = Graph::new();

                // Create nodes
                for i in 0..node_count {
                    let node = Node::function(
                        format!("test::func_{}", i),
                        Range::with_file(i as u32, 0, (i + 1) as u32, 10, "test.rs".to_string()),
                    );
                    graph.add_node(node);
                }

                // Create edges (avoid self-loops by using different indices)
                for i in 0..edge_count {
                    let from_idx = i % node_count;
                    let to_idx = (i + 1) % node_count;

                    let from_id = format!("test::func_{}", from_idx);
                    let to_id = format!("test::func_{}", to_idx);
                    let edge = Edge::call(from_id, to_id, Provenance::tree_sitter());
                    graph.add_edge(edge);
                }

                // Validation should either pass or fail with specific error types
                let result = graph.validate(Confidence::High);
                match result {
                    Ok(_) => {
                        // If validation passes, all edges should reference existing nodes
                        prop_assert!(graph.edge_count() <= graph.node_count());
                    }
                    Err(ValidationError::DanglingEdge { .. }) => {
                        // Expected for invalid graphs
                    }
                    Err(ValidationError::LowConfidence(_)) => {
                        // Expected for low confidence edges
                    }
                    Err(_) => {
                        // Other errors are unexpected
                        prop_assert!(false, "Unexpected validation error");
                    }
                }
            }
        }
    }
}
