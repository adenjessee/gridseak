//! Smoke tests for `ParseRepositoryUseCase` exercised with the
//! lightweight `MockSyntaxExtractor` / `MockSemanticResolver` /
//! `MockGraphRepository` fakes from `graphengine-parsing-test-support`.
//!
//! Originally lived as `#[cfg(test)] mod tests` inside
//! `src/application/use_cases/parse_repo/use_case.rs`. R2
//! (v0.1.0-rc1 follow-up) relocated the block so the mocks would no
//! longer be present anywhere inside `graphengine-parsing/src/`.
//! Behavioural assertions are unchanged.

use graphengine_parsing::application::ports::ResolutionStatsSummary;
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::application::use_cases::ResolvedGraph;
use graphengine_parsing::domain::{Confidence, Graph, Node, ProvenanceSource, Range};
use graphengine_parsing_test_support::{
    MockGraphRepository, MockSemanticResolver, MockSyntaxExtractor,
};

fn create_test_use_case() -> ParseRepositoryUseCase {
    let syntax_extractor = Box::new(MockSyntaxExtractor::new("rust"));
    let semantic_resolver = Box::new(MockSemanticResolver::new("rust", true));
    let graph_repo = Box::new(MockGraphRepository::new());

    ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    )
}

#[tokio::test]
async fn test_parse_empty_directory() {
    let use_case = create_test_use_case();
    let temp_dir = std::env::temp_dir().join("empty_test");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());

    let resolved_graph = result.unwrap();
    assert!(resolved_graph.is_empty());
}

#[tokio::test]
async fn test_parse_with_mock_data() {
    let syntax_extractor = Box::new(MockSyntaxExtractor::new("rust"));
    let semantic_resolver = Box::new(MockSemanticResolver::new("rust", false));
    let graph_repo = Box::new(MockGraphRepository::new());

    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );

    let temp_dir = std::env::temp_dir().join("mock_test");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let test_file = temp_dir.join("test.rs");
    std::fs::write(&test_file, "fn main() { println!(\"hello\"); }").unwrap();

    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    if let Err(e) = &result {
        println!("Error: {}", e);
    }
    assert!(result.is_ok());

    let resolved_graph = result.unwrap();
    assert!(!resolved_graph.is_empty());
    assert!(resolved_graph.node_count() > 0);
}

#[tokio::test]
async fn test_resolved_graph_wrapper() {
    let mut graph = Graph::new();
    let node = Node::function(
        "test::func".to_string(),
        Range::with_file(1, 0, 5, 10, "test.rs".to_string()),
    );
    graph.add_node(node);

    let resolved_graph = ResolvedGraph::new(graph, ResolutionStatsSummary::new());
    assert_eq!(resolved_graph.node_count(), 1);
    assert_eq!(resolved_graph.edge_count(), 0);
    assert!(!resolved_graph.is_empty());
}

#[tokio::test]
async fn test_use_case_creation() {
    let _use_case = create_test_use_case();
}

#[tokio::test]
async fn test_parse_with_unavailable_resolver() {
    let syntax_extractor = Box::new(MockSyntaxExtractor::new("rust"));
    let semantic_resolver = Box::new(MockSemanticResolver::new("rust", false));
    let graph_repo = Box::new(MockGraphRepository::new());

    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );

    let temp_dir = std::env::temp_dir().join("unavailable_test");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let test_file = temp_dir.join("test.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();

    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());

    let resolved_graph = result.unwrap();
    assert!(resolved_graph.node_count() > 0);
    assert!(
        resolved_graph
            .graph()
            .edges
            .iter()
            .all(|e| e.provenance.source != ProvenanceSource::Lsp),
        "Expected no LSP-derived edges when resolver is unavailable"
    );
}

#[tokio::test]
async fn test_parse_with_low_confidence_requirement() {
    let syntax_extractor = Box::new(MockSyntaxExtractor::new("rust"));
    let semantic_resolver = Box::new(MockSemanticResolver::new("rust", false));
    let graph_repo = Box::new(MockGraphRepository::new());

    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::Low,
    );

    let temp_dir = std::env::temp_dir().join("low_conf_test");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let test_file = temp_dir.join("test.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();

    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());
}
