use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::Confidence;
use graphengine_parsing::infrastructure::{load_config, SqliteRepository};
use graphengine_parsing_test_support::{MockLspResolver, MockSyntaxExtractorWithConfig};
use std::fs;
use std::path::PathBuf;

#[tokio::test]
async fn test_cross_file_function_call_resolves() {
    // Create temp repo with two files: a.rs defines foo, b.rs calls foo
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let root = temp_dir.path();

    let a = root.join("a.rs");
    let b = root.join("b.rs");

    fs::write(&a, r#"
pub fn foo() {}
"#).expect("write a.rs");

    fs::write(&b, r#"
fn bar() { foo(); }
"#).expect("write b.rs");

    // Inlined what was `ParseRepositoryUseCase::with_in_memory_storage("rust", Confidence::Low)`.
    // R2 (v0.1.0-rc1 follow-up) deleted that factory method along with
    // `with_infrastructure` because both unconditionally instantiated
    // mock components from production source. Mocks now live in the
    // dev-only `graphengine-parsing-test-support` crate.
    let config = load_config("rust").expect("load rust config");
    let syntax_extractor = Box::new(MockSyntaxExtractorWithConfig::new(config.clone()));
    let semantic_resolver = Box::new(MockLspResolver::new(config, None));
    let graph_repo = Box::new(
        SqliteRepository::new_in_memory().expect("create in-memory SQLite repo"),
    );
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::Low,
    );

    let resolved = use_case
        .parse(PathBuf::from(root), "rust".to_string())
        .await
        .expect("parse ok");
    let graph = resolved.graph();

    // Assert nodes include foo and bar
    let mut has_foo = false;
    let mut has_bar = false;
    for node in graph.nodes.values() {
        if node.fqname.contains("foo") { has_foo = true; }
        if node.fqname.contains("bar") { has_bar = true; }
    }
    assert!(has_foo && has_bar, "expected foo and bar nodes to exist");

    // Assert at least one Call edge between bar -> foo (LSP or fallback)
    let mut has_call = false;
    for edge in &graph.edges {
        if let graphengine_parsing::domain::EdgeKind::Call = edge.r#type { has_call = true; break; }
    }
    assert!(has_call, "expected at least one Call edge across files (bar -> foo)");

    let stats = resolved.stats();
    assert!(stats.total_call_edges() > 0, "expected call edges in stats");
}


