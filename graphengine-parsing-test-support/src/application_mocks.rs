//! Application-layer port mocks (lightweight in-memory fixtures for
//! `SyntaxExtractor`, `SemanticResolver`, `GraphRepository`).
//!
//! Relocated from `graphengine-parsing/src/application/mocks.rs` by R2
//! (v0.1.0-rc1 follow-up). These mocks satisfy the trait contracts with
//! minimal behaviour — enough to exercise the application-layer wiring
//! in tests without spinning up Tree-sitter / LSP / SQLite.
//!
//! For a richer fake that simulates parsing (line-based pattern
//! matching producing realistic-looking `SyntaxResults`), see
//! [`crate::mock_extractor::MockSyntaxExtractor`]. The simpler mock
//! here is exposed via the crate root re-export
//! [`crate::MockSyntaxExtractor`]; the config-aware variant is
//! re-exported as [`crate::MockSyntaxExtractorWithConfig`].

use async_trait::async_trait;
use graphengine_parsing::application::ports::{
    GraphRepository, ResolvedEdges, SemanticResolver, SyntaxExtractor, SyntaxResults,
};
use graphengine_parsing::domain::{Edge, Graph, Node, Provenance, Range};

/// Mock syntax extractor for testing
pub struct MockSyntaxExtractor {
    pub language: String,
    pub supported_extensions: Vec<String>,
}

impl MockSyntaxExtractor {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
            supported_extensions: vec![language.to_string(), format!(".{}", language)],
        }
    }
}

#[async_trait]
impl SyntaxExtractor for MockSyntaxExtractor {
    async fn extract(&self, _files: &[std::path::PathBuf]) -> anyhow::Result<SyntaxResults> {
        let mut results = SyntaxResults::new();
        let node = Node::function(
            "mock::func".to_string(),
            Range::with_file(1, 0, 5, 10, "mock.rs".to_string()),
        );
        results.add_symbol(node);
        Ok(results)
    }

    fn supported_language(&self) -> &str {
        &self.language
    }

    fn supports_extension(&self, ext: &str) -> bool {
        let normalized = if let Some(stripped) = ext.strip_prefix('.') {
            stripped
        } else {
            ext
        };

        self.supported_extensions
            .iter()
            .any(|e| e == ext || e == normalized)
            || (self.language == "rust" && (normalized == "rs" || ext == ".rs"))
    }
}

/// Mock semantic resolver for testing
pub struct MockSemanticResolver {
    pub language: String,
    pub available: bool,
}

impl MockSemanticResolver {
    pub fn new(language: &str, available: bool) -> Self {
        Self {
            language: language.to_string(),
            available,
        }
    }
}

#[async_trait]
impl SemanticResolver for MockSemanticResolver {
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        let mut edges = ResolvedEdges::new();
        if !hints.symbols.is_empty() {
            let edge = Edge::contains(
                "mock::func".to_string(),
                "mock::func".to_string(),
                Provenance::lsp(),
            );
            edges.add_containment_edge(edge);
        }
        Ok(edges)
    }

    fn supported_language(&self) -> &str {
        &self.language
    }

    async fn is_available(&self) -> bool {
        self.available
    }
}

/// Mock graph repository for testing
pub struct MockGraphRepository {
    pub graphs: std::collections::HashMap<String, Graph>,
}

impl Default for MockGraphRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl MockGraphRepository {
    pub fn new() -> Self {
        Self {
            graphs: std::collections::HashMap::new(),
        }
    }
}

#[async_trait]
impl GraphRepository for MockGraphRepository {
    async fn upsert(&self, _graph: &Graph) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<Graph>> {
        Ok(self.graphs.get(id).cloned())
    }

    async fn list(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.graphs.keys().cloned().collect())
    }

    async fn delete(&self, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_parsing::domain::Node;

    #[tokio::test]
    async fn test_mock_syntax_extractor() {
        let extractor = MockSyntaxExtractor::new("rust");
        assert_eq!(extractor.supported_language(), "rust");
        assert!(extractor.supports_extension("rust"));
        assert!(!extractor.supports_extension("js"));

        let files = vec![std::path::PathBuf::from("test.rs")];
        let results = extractor.extract(&files).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results.symbols.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_semantic_resolver() {
        let resolver = MockSemanticResolver::new("rust", true);
        assert_eq!(resolver.supported_language(), "rust");
        assert!(resolver.is_available().await);

        let mut hints = SyntaxResults::new();
        let node = Node::function(
            "mock::func".to_string(),
            Range::with_file(1, 0, 5, 10, "mock.rs".to_string()),
        );
        hints.add_symbol(node);
        let edges = resolver.resolve(&hints).await.unwrap();
        assert!(!edges.is_empty());
        assert_eq!(edges.containment_edges.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_graph_repository() {
        let repo = MockGraphRepository::new();
        let graph = Graph::new();

        repo.upsert(&graph).await.unwrap();
        let graphs = repo.list().await.unwrap();
        assert_eq!(graphs.len(), 0);

        repo.delete("test").await.unwrap();
    }
}
