//! Integration tests for the application layer
//!
//! Tests the complete parsing pipeline with mock adapters.
//! Covers happy path, error scenarios, and edge cases.

use graphengine_parsing::application::*;
use graphengine_parsing::domain::*;
use graphengine_parsing::infrastructure::lsp::LspResolver;
use graphengine_parsing::infrastructure::load_config;
use graphengine_parsing::infrastructure::SqliteRepository;
use std::path::PathBuf;
use std::collections::HashMap;
use tempfile::TempDir;

/// Mock syntax extractor that can be configured for different scenarios
pub struct ConfigurableMockSyntaxExtractor {
    pub language: String,
    pub supported_extensions: Vec<String>,
    pub should_fail: bool,
    pub return_empty: bool,
    pub symbols: Vec<Node>,
    pub call_sites: Vec<CallSite>,
    pub imports: Vec<Range>,
    pub type_refs: Vec<Range>,
}

impl ConfigurableMockSyntaxExtractor {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
            supported_extensions: vec![language.to_string()],
            should_fail: false,
            return_empty: false,
            symbols: Vec::new(),
            call_sites: Vec::new(),
            imports: Vec::new(),
            type_refs: Vec::new(),
        }
    }

    pub fn with_failure(mut self, should_fail: bool) -> Self {
        self.should_fail = should_fail;
        self
    }

    pub fn with_empty_results(mut self, return_empty: bool) -> Self {
        self.return_empty = return_empty;
        self
    }

    pub fn add_symbol(mut self, symbol: Node) -> Self {
        self.symbols.push(symbol);
        self
    }

    pub fn add_call_site<S: Into<String>>(mut self, location: Range, function_name: S) -> Self {
        self.call_sites.push(CallSite {
            location,
            function_name: function_name.into(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
            edge_kind_hint: None,
        });
        self
    }

    pub fn add_import(mut self, location: Range) -> Self {
        self.imports.push(location);
        self
    }

    pub fn add_type_ref(mut self, location: Range) -> Self {
        self.type_refs.push(location);
        self
    }
}

#[async_trait::async_trait]
impl SyntaxExtractor for ConfigurableMockSyntaxExtractor {
    async fn extract(&self, _files: &[PathBuf]) -> anyhow::Result<SyntaxResults> {
        if self.should_fail {
            return Err(anyhow::anyhow!("Mock extraction failure"));
        }

        if self.return_empty {
            return Ok(SyntaxResults::new());
        }

        let mut results = SyntaxResults::new();
        results.symbols = self.symbols.clone();
        results.call_sites = self.call_sites.clone();
        results.imports = self.imports.clone();
        results.type_refs = self.type_refs.clone();
        Ok(results)
    }

    fn supported_language(&self) -> &str {
        &self.language
    }

    fn supports_extension(&self, ext: &str) -> bool {
        self.supported_extensions.contains(&ext.to_string())
    }
}

/// Mock semantic resolver that can be configured for different scenarios
pub struct ConfigurableMockSemanticResolver {
    pub language: String,
    pub available: bool,
    pub should_fail: bool,
    pub return_empty: bool,
    pub call_edges: Vec<Edge>,
    pub import_edges: Vec<Edge>,
    pub type_edges: Vec<Edge>,
    pub containment_edges: Vec<Edge>,
}

impl ConfigurableMockSemanticResolver {
    pub fn new(language: &str, available: bool) -> Self {
        Self {
            language: language.to_string(),
            available,
            should_fail: false,
            return_empty: false,
            call_edges: Vec::new(),
            import_edges: Vec::new(),
            type_edges: Vec::new(),
            containment_edges: Vec::new(),
        }
    }

    pub fn with_failure(mut self, should_fail: bool) -> Self {
        self.should_fail = should_fail;
        self
    }

    pub fn with_empty_results(mut self, return_empty: bool) -> Self {
        self.return_empty = return_empty;
        self
    }

    pub fn add_call_edge(mut self, edge: Edge) -> Self {
        self.call_edges.push(edge);
        self
    }

    pub fn add_import_edge(mut self, edge: Edge) -> Self {
        self.import_edges.push(edge);
        self
    }

    pub fn add_type_edge(mut self, edge: Edge) -> Self {
        self.type_edges.push(edge);
        self
    }

    pub fn add_containment_edge(mut self, edge: Edge) -> Self {
        self.containment_edges.push(edge);
        self
    }
}

#[async_trait::async_trait]
impl SemanticResolver for ConfigurableMockSemanticResolver {
    async fn resolve(&self, _hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        if self.should_fail {
            return Err(anyhow::anyhow!("Mock resolution failure"));
        }

        if self.return_empty {
            return Ok(ResolvedEdges::new());
        }

        let mut edges = ResolvedEdges::new();
        edges.call_edges = self.call_edges.clone();
        edges.import_edges = self.import_edges.clone();
        edges.type_edges = self.type_edges.clone();
        edges.containment_edges = self.containment_edges.clone();
        Ok(edges)
    }

    fn supported_language(&self) -> &str {
        &self.language
    }

    async fn is_available(&self) -> bool {
        self.available
    }
}

/// Mock graph repository that tracks operations
pub struct TrackingMockGraphRepository {
    pub graphs: HashMap<String, Graph>,
    pub should_fail: bool,
    pub upsert_calls: usize,
    pub get_calls: usize,
    pub list_calls: usize,
    pub delete_calls: usize,
}

impl TrackingMockGraphRepository {
    pub fn new() -> Self {
        Self {
            graphs: HashMap::new(),
            should_fail: false,
            upsert_calls: 0,
            get_calls: 0,
            list_calls: 0,
            delete_calls: 0,
        }
    }

    pub fn with_failure(mut self, should_fail: bool) -> Self {
        self.should_fail = should_fail;
        self
    }
}

#[async_trait::async_trait]
impl GraphRepository for TrackingMockGraphRepository {
    async fn upsert(&self, _graph: &Graph) -> anyhow::Result<()> {
        if self.should_fail {
            return Err(anyhow::anyhow!("Mock repository failure"));
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<Graph>> {
        if self.should_fail {
            return Err(anyhow::anyhow!("Mock repository failure"));
        }
        Ok(self.graphs.get(id).cloned())
    }

    async fn list(&self) -> anyhow::Result<Vec<String>> {
        if self.should_fail {
            return Err(anyhow::anyhow!("Mock repository failure"));
        }
        Ok(self.graphs.keys().cloned().collect())
    }

    async fn delete(&self, _id: &str) -> anyhow::Result<()> {
        if self.should_fail {
            return Err(anyhow::anyhow!("Mock repository failure"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_happy_path_parsing() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10)))
            .add_call_site(Range::test(3, 5, 3, 15), "println!".to_string())
    );
    
    let semantic_resolver = Box::new(
        ConfigurableMockSemanticResolver::new("rust", true)
            .add_call_edge(Edge::call("main".to_string(), "println!".to_string(), Provenance::lsp()))
    );
    
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("happy_path_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() { println!(\"hello\"); }").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());
    
    let resolved_graph = result.unwrap();
    assert!(!resolved_graph.is_empty());
    assert!(resolved_graph.node_count() > 0);
    assert!(resolved_graph.edge_count() > 0);
}

#[tokio::test]
async fn test_empty_repository() {
    let syntax_extractor = Box::new(ConfigurableMockSyntaxExtractor::new("rust"));
    let semantic_resolver = Box::new(ConfigurableMockSemanticResolver::new("rust", true));
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("empty_repo_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());
    
    let resolved_graph = result.unwrap();
    assert!(resolved_graph.is_empty());
}

#[tokio::test]
async fn test_syntax_extraction_failure() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .with_failure(true)
    );
    let semantic_resolver = Box::new(ConfigurableMockSemanticResolver::new("rust", true));
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("syntax_failure_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_err());
    
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Extraction failed"));
}

#[tokio::test]
async fn test_semantic_resolution_failure() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10)))
    );
    let semantic_resolver = Box::new(
        ConfigurableMockSemanticResolver::new("rust", true)
            .with_failure(true)
    );
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("resolution_failure_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_err());
    
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Resolution failed"));
}

#[tokio::test]
async fn test_repository_persistence_failure() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10)))
    );
    let semantic_resolver = Box::new(ConfigurableMockSemanticResolver::new("rust", true));
    let graph_repo = Box::new(
        TrackingMockGraphRepository::new()
            .with_failure(true)
    );
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("repo_failure_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_err());
    
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Repository error"));
}

#[tokio::test]
async fn test_unavailable_semantic_resolver() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10)))
    );
    let semantic_resolver = Box::new(ConfigurableMockSemanticResolver::new("rust", false));
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("unavailable_resolver_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());
    
    let resolved_graph = result.unwrap();
    assert!(resolved_graph.node_count() > 0);
    assert_eq!(resolved_graph.edge_count(), 0); // No edges from unavailable resolver
}

#[tokio::test]
async fn test_cross_language_python_heuristic_resolution() {
    use url::Url;

    let temp_dir = TempDir::new().unwrap();
    let workspace_root = Url::from_directory_path(temp_dir.path()).ok();

    let python_file = temp_dir.path().join("module.py");
    std::fs::write(
        &python_file,
        r#"
def helper():
    return 1

def main():
    value = helper()
    return value
"#,
    )
    .unwrap();

    let file_str = python_file.to_string_lossy().to_string();

    let helper_node = Node::function(
        "python::helper".to_string(),
        Range::with_file(1, 0, 3, 0, file_str.clone()),
    );
    let main_node = Node::function(
        "python::main".to_string(),
        Range::with_file(5, 0, 8, 0, file_str.clone()),
    );

    let mut extractor = ConfigurableMockSyntaxExtractor::new("python")
        .add_symbol(helper_node)
        .add_symbol(main_node)
        .add_call_site(
            Range::with_file(6, 4, 6, 14, file_str.clone()),
            "python::helper",
        );
    extractor.supported_extensions = vec![".py".to_string()];

    let config = load_config("python").expect("load python config");
    let semantic_resolver = LspResolver::new(config, workspace_root);
    let graph_repo = SqliteRepository::new_in_memory().expect("create in-memory repo");

    let use_case = ParseRepositoryUseCase::new(
        Box::new(extractor),
        Box::new(semantic_resolver),
        Box::new(graph_repo),
        Confidence::Medium,
    );

    let result = use_case
        .parse(temp_dir.path().to_path_buf(), "python".to_string())
        .await
        .expect("parse python project");

    let graph = result.graph();
    assert!(graph.node_count() >= 2, "expected python functions captured");
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Call),
        "expected heuristic call edge for python"
    );

    let stats = result.stats();
    assert!(stats.total_call_edges() >= 1, "stats should record call edges");
    assert!(
        stats.heuristic_edges >= 1,
        "heuristic edges should be recorded when LSP unavailable"
    );
}

#[tokio::test]
async fn test_validation_failure() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10)))
    );
    let semantic_resolver = Box::new(
        ConfigurableMockSemanticResolver::new("rust", true)
            .add_call_edge(Edge::call("main".to_string(), "nonexistent".to_string(), Provenance::lsp()))
    );
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("validation_failure_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_err());
    
    let error = result.unwrap_err();
    assert!(error.to_string().contains("Validation failed"));
}

#[tokio::test]
async fn test_different_confidence_levels() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10))));
        Box::new(ConfigurableMockSemanticResolver::new("rust", true)),
        Box::new(TrackingMockGraphRepository::new()),
        Confidence::Low,
    );
    
    let temp_dir = std::env::temp_dir().join("confidence_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();
    
    let result = use_case_low.parse(temp_dir.clone(), "rust".to_string()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_multiple_file_types() {
    let syntax_extractor = Box::new(
        ConfigurableMockSyntaxExtractor::new("rust")
            .add_symbol(Node::function("main".to_string(), Range::test(1, 0, 5, 10)))
            .add_symbol(Node::struct_("MyStruct".to_string(), Range::test(7, 0, 10, 10)))
    );
    let semantic_resolver = Box::new(ConfigurableMockSemanticResolver::new("rust", true));
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("multiple_files_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file1 = temp_dir.join("main.rs");
    let test_file2 = temp_dir.join("lib.rs");
    std::fs::write(&test_file1, "fn main() {}").unwrap();
    std::fs::write(&test_file2, "struct MyStruct {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());
    
    let resolved_graph = result.unwrap();
    assert!(resolved_graph.node_count() >= 2);
}

#[tokio::test]
async fn test_unsupported_file_extension() {
    let syntax_extractor = Box::new(ConfigurableMockSyntaxExtractor::new("rust"));
    let semantic_resolver = Box::new(ConfigurableMockSemanticResolver::new("rust", true));
    let graph_repo = Box::new(TrackingMockGraphRepository::new());
    
    let use_case = ParseRepositoryUseCase::new(
        syntax_extractor,
        semantic_resolver,
        graph_repo,
        Confidence::High,
    );
    
    let temp_dir = std::env::temp_dir().join("unsupported_files_test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    
    let test_file = temp_dir.join("main.js"); // JavaScript file, not Rust
    std::fs::write(&test_file, "function main() {}").unwrap();
    
    let result = use_case.parse(temp_dir, "rust".to_string()).await;
    assert!(result.is_ok());
    
    let resolved_graph = result.unwrap();
    assert!(resolved_graph.is_empty()); // No Rust files found
}

#[tokio::test]
async fn test_resolved_graph_wrapper_functionality() {
    let mut graph = Graph::new();
    let node1 = Node::function("func1".to_string(), Range::test(1, 0, 5, 10));
    let node2 = Node::function("func2".to_string(), Range::test(6, 0, 10, 10));
    graph.add_node(node1);
    graph.add_node(node2);
    graph.add_edge(Edge::call("func1".to_string(), "func2".to_string(), Provenance::lsp()));
    
    let resolved_graph = ResolvedGraph::new(graph, ResolutionStatsSummary::new());
    
    assert_eq!(resolved_graph.node_count(), 2);
    assert_eq!(resolved_graph.edge_count(), 1);
    assert!(!resolved_graph.is_empty());
    
    let underlying_graph = resolved_graph.graph();
    assert_eq!(underlying_graph.node_count(), 2);
    assert_eq!(underlying_graph.edge_count(), 1);
}
