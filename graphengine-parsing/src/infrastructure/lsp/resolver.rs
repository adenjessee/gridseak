//! LSP-based semantic resolver implementation
//!
//! Provides semantic resolution using Language Server Protocol (LSP) to resolve
//! function calls, imports, types, and containment relationships. This is the
//! "SEM" part of the FAST-SEM pipeline, providing high-confidence semantic
//! relationships that complement the fast syntactic extraction.

use crate::application::ports::{
    ResolutionStatsSummary, ResolvedEdges, SemanticResolver, SyntaxResults, UnresolvedReference,
};
use crate::domain::{Edge, EdgeKind, Range};
use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::call_resolver::CallResolver;
use crate::infrastructure::lsp::definition_provider::{
    session_provider, session_provider_with_options, DefinitionProvider, SessionOptions,
};
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::receiver_detector::ReceiverTypeDetector;
use crate::infrastructure::lsp::resolvers::{
    call_resolver_lsp::LspCallResolver, import_resolver::ImportResolver,
    module_dependency_resolver::ModuleDependencyResolver, type_resolver::TypeResolver,
};
use crate::infrastructure::lsp::stats::{LspMiss, ResolutionAggregator};
use crate::module_resolution::ModuleResolver;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, info, warn};
use url::Url;

/// LSP-based semantic resolver
pub struct LspResolver {
    /// Language configuration
    config: Arc<LanguageConfig>,
    /// Pluggable definition provider (LSP-backed in production)
    definition_provider: Arc<dyn DefinitionProvider>,
    /// Receiver type detector for trait object detection
    /// Wrapped in Arc to enable sharing with CallResolver
    receiver_detector: Option<Arc<ReceiverTypeDetector>>,
}

impl LspResolver {
    /// Create a new LSP resolver
    pub fn new(config: LanguageConfig, workspace_root: Option<Url>) -> Self {
        let config_arc = Arc::new(config.clone());
        let provider = session_provider(config, workspace_root);
        let receiver_detector = Some(Arc::new(ReceiverTypeDetector::new(
            Arc::clone(&provider),
            config_arc.clone(),
        )));
        Self {
            config: config_arc,
            definition_provider: provider,
            receiver_detector,
        }
    }

    /// F.2: like [`LspResolver::new`] but with explicit session
    /// options (workspace folders, readiness strategy). Used by the
    /// Apex path in the factory so jorje sees every SFDX package and
    /// we wait for indexing before sending `textDocument/definition`.
    pub fn with_options(
        config: LanguageConfig,
        workspace_root: Option<Url>,
        options: SessionOptions,
    ) -> Self {
        let config_arc = Arc::new(config.clone());
        let provider = session_provider_with_options(config, workspace_root, options);
        let receiver_detector = Some(Arc::new(ReceiverTypeDetector::new(
            Arc::clone(&provider),
            config_arc.clone(),
        )));
        Self {
            config: config_arc,
            definition_provider: provider,
            receiver_detector,
        }
    }

    /// Construct a resolver with a custom definition provider (used in tests).
    pub fn with_provider(
        config: Arc<LanguageConfig>,
        definition_provider: Arc<dyn DefinitionProvider>,
    ) -> Self {
        let receiver_detector = Some(Arc::new(ReceiverTypeDetector::new(
            Arc::clone(&definition_provider),
            config.clone(),
        )));
        Self {
            config,
            definition_provider,
            receiver_detector,
        }
    }

    /// Resolve function calls using LSP semantic analysis
    async fn resolve_calls(
        &self,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
    ) -> Result<(Vec<Edge>, ResolutionStatsSummary), LspError> {
        let mut aggregator = ResolutionAggregator::default();

        let mut unresolved_calls: Vec<UnresolvedReference> = syntax_results.references.clone();

        let t_lsp_calls = std::time::Instant::now();
        match self.definition_provider.ensure_ready().await {
            Ok(_) => match self.resolve_calls_with_lsp(syntax_results, misses).await {
                Ok(outcome) => {
                    aggregator.add_edges(outcome.edges);
                    unresolved_calls = outcome.unresolved_calls;
                }
                Err(err) => {
                    warn!(
                        "LSP call resolution failed ({}), falling back to heuristics",
                        err
                    );
                    aggregator.record_lsp_failure(err.to_string());
                }
            },
            Err(err) => {
                warn!(
                    "Unable to prepare definition provider: {}. Falling back to heuristics.",
                    err
                );
                aggregator.record_lsp_failure(err.to_string());
            }
        }
        info!("[TIMING] LSP call resolution: {:?}", t_lsp_calls.elapsed());

        let t_heuristic_calls = std::time::Instant::now();
        if !unresolved_calls.is_empty() {
            match self.resolve_calls_with_heuristics(syntax_results, &unresolved_calls) {
                Ok(edges) => aggregator.add_edges(edges),
                Err(err) => {
                    warn!("Heuristic call resolution failed: {}", err);
                    aggregator.record_heuristic_failure(err.to_string());
                }
            }
        } else {
            debug!("All call sites resolved via LSP; skipping heuristic fallback");
        }
        info!(
            "[TIMING] Heuristic call fallback ({} unresolved): {:?}",
            unresolved_calls.len(),
            t_heuristic_calls.elapsed()
        );

        let (edges, stats) = aggregator.into_parts();
        Ok((edges, stats))
    }

    /// Resolve calls using LSP semantic analysis
    async fn resolve_calls_with_lsp(
        &self,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
    ) -> Result<crate::infrastructure::lsp::stats::LspResolutionOutcome, LspError> {
        LspCallResolver::resolve_with_lsp(
            self.definition_provider.as_ref(),
            self.receiver_detector.as_ref(),
            syntax_results,
            misses,
        )
        .await
    }

    /// Resolve calls using heuristic analysis (fallback when LSP is not available).
    ///
    /// Accepts the full syntax results (for building indexes) and the specific
    /// call sites to resolve (avoids cloning the entire SyntaxResults).
    fn resolve_calls_with_heuristics(
        &self,
        syntax_results: &SyntaxResults,
        unresolved_calls: &[UnresolvedReference],
    ) -> Result<Vec<Edge>, LspError> {
        let mut call_resolver = match &self.receiver_detector {
            Some(detector) => CallResolver::with_receiver_detector(Arc::clone(detector)),
            None => CallResolver::new(),
        };

        call_resolver.prepare(syntax_results);
        let module_resolver = ModuleResolver::from_syntax(syntax_results);

        match call_resolver.resolve_references(unresolved_calls, &module_resolver) {
            Ok(edges) => Ok(edges),
            Err(e) => {
                warn!("Heuristic call resolution failed: {}", e);
                Err(LspError::request_failed(e.to_string()))
            }
        }
    }

    /// Resolve imports using LSP (internal, with progress tracking)
    async fn resolve_imports_internal(
        &self,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
        total_work: usize,
        completed_work: usize,
    ) -> Result<(Vec<Edge>, Vec<Range>), LspError> {
        ImportResolver::resolve_with_lsp(
            self.definition_provider.as_ref(),
            syntax_results,
            misses,
            total_work,
            completed_work,
        )
        .await
    }

    /// Resolve type relationships using LSP
    async fn resolve_types(
        &self,
        syntax_results: &SyntaxResults,
        misses: &mut Vec<LspMiss>,
    ) -> Result<(Vec<Edge>, Vec<Range>), LspError> {
        TypeResolver::resolve_with_lsp(self.definition_provider.as_ref(), syntax_results, misses)
            .await
    }

    /// Resolve imports using heuristics (fallback when LSP fails)
    fn resolve_imports_with_heuristics(
        &self,
        syntax_results: &SyntaxResults,
        unresolved_imports: &[Range],
    ) -> Result<Vec<Edge>, LspError> {
        ImportResolver::resolve_with_heuristics(syntax_results, unresolved_imports)
    }

    /// Resolve types using heuristics (fallback when LSP fails)
    fn resolve_types_with_heuristics(
        &self,
        syntax_results: &SyntaxResults,
        unresolved_types: &[Range],
    ) -> Result<Vec<Edge>, LspError> {
        TypeResolver::resolve_with_heuristics(syntax_results, unresolved_types)
    }

    async fn resolve_containment(
        &self,
        _syntax_results: &SyntaxResults,
    ) -> Result<Vec<Edge>, LspError> {
        Ok(Vec::new())
    }

    /// Core resolution logic, kept outside `async_trait` so the compiler
    /// can fully infer the lifetimes of `buffer_unordered` closures inside
    /// the individual resolver phases.
    async fn resolve_inner(
        &self,
        syntax_results: &SyntaxResults,
    ) -> Result<ResolvedEdges, anyhow::Error> {
        let is_available = self.definition_provider.is_available().await;
        if !is_available {
            warn!("LSP server not available; continuing with heuristic resolution only");
        }

        let resolution_start = std::time::Instant::now();

        let total_calls = syntax_results.references.len();
        let total_imports = syntax_results.import_specs.len();
        let total_types = syntax_results.type_refs.len();
        let total_work = total_calls + total_imports + total_types;

        info!(
            "[PARALLEL] Starting resolution: {} calls, {} imports, {} types ({} total items)",
            total_calls, total_imports, total_types, total_work
        );

        // Run phases 1-3 in parallel (or sequentially for profiling).
        // Set GE_SEQUENTIAL_RESOLVE=1 to force sequential execution for
        // measuring whether the phases truly overlap in the join!.
        let mut call_misses = Vec::new();
        let mut import_misses = Vec::new();
        let mut type_misses = Vec::new();

        let sequential = std::env::var("GE_SEQUENTIAL_RESOLVE").is_ok_and(|v| v == "1");

        let t_lsp = std::time::Instant::now();
        let (call_result, import_result, type_result) = if sequential {
            info!("[PARALLEL] Running resolution phases SEQUENTIALLY (GE_SEQUENTIAL_RESOLVE=1)");
            let cr = self.resolve_calls(syntax_results, &mut call_misses).await;
            let ir = self
                .resolve_imports_internal(syntax_results, &mut import_misses, total_work, 0)
                .await;
            let tr = self.resolve_types(syntax_results, &mut type_misses).await;
            (cr, ir, tr)
        } else {
            tokio::join!(
                self.resolve_calls(syntax_results, &mut call_misses),
                self.resolve_imports_internal(syntax_results, &mut import_misses, total_work, 0),
                self.resolve_types(syntax_results, &mut type_misses),
            )
        };
        info!(
            "[TIMING] LSP resolution phases ({}): {:?}",
            if sequential { "sequential" } else { "join!" },
            t_lsp.elapsed()
        );

        let mut aggregator = ResolutionAggregator::default();
        let mut heuristic_call_fallbacks = 0;
        let mut heuristic_import_fallbacks = 0;
        let mut heuristic_type_fallbacks = 0;

        match call_result {
            Ok((call_edges, call_stats)) => {
                let count = call_edges.len();
                aggregator.add_edges(call_edges);
                heuristic_call_fallbacks = call_stats.heuristic_edges;
                info!(
                    "[1/5] Calls resolved: {} edges from {} call sites",
                    count, total_calls
                );
            }
            Err(e) => {
                warn!("Call resolution failed: {}", e);
                aggregator.record_lsp_failure(format!("Call resolution failed: {}", e));
            }
        }

        let unresolved_imports = match import_result {
            Ok((edges, unresolved)) => {
                let count = edges.len();
                aggregator.add_edges(edges);
                info!(
                    "[2/5] Imports resolved: {} edges, {} unresolved",
                    count,
                    unresolved.len()
                );
                unresolved
            }
            Err(e) => {
                warn!("Import resolution failed: {}", e);
                aggregator.record_lsp_failure(format!("Import resolution failed: {}", e));
                syntax_results
                    .import_specs
                    .iter()
                    .map(|s| s.range.clone())
                    .collect()
            }
        };

        let t_heuristic_imports = std::time::Instant::now();
        if !unresolved_imports.is_empty() {
            match self.resolve_imports_with_heuristics(syntax_results, &unresolved_imports) {
                Ok(fallback_edges) => {
                    heuristic_import_fallbacks = fallback_edges.len();
                    aggregator.add_edges(fallback_edges);
                }
                Err(e) => {
                    warn!("Heuristic import resolution failed: {}", e);
                    aggregator.record_heuristic_failure(format!("Import fallback failed: {}", e));
                }
            }
        }
        info!(
            "[TIMING] Heuristic import fallback ({} unresolved): {:?}",
            unresolved_imports.len(),
            t_heuristic_imports.elapsed()
        );

        let module_dep_edges = ModuleDependencyResolver::resolve_relative_module_imports(
            syntax_results,
            &self.config.file_extensions,
        );
        if !module_dep_edges.is_empty() {
            aggregator.add_edges(module_dep_edges);
        }

        let unresolved_types = match type_result {
            Ok((edges, unresolved)) => {
                let count = edges.len();
                aggregator.add_edges(edges);
                info!(
                    "[3/5] Types resolved: {} edges, {} unresolved",
                    count,
                    unresolved.len()
                );
                unresolved
            }
            Err(e) => {
                warn!("Type resolution failed: {}", e);
                aggregator.record_lsp_failure(format!("Type resolution failed: {}", e));
                syntax_results.type_refs.clone()
            }
        };

        let t_heuristic_types = std::time::Instant::now();
        if !unresolved_types.is_empty() {
            match self.resolve_types_with_heuristics(syntax_results, &unresolved_types) {
                Ok(fallback_edges) => {
                    heuristic_type_fallbacks = fallback_edges.len();
                    aggregator.add_edges(fallback_edges);
                }
                Err(e) => {
                    warn!("Heuristic type resolution failed: {}", e);
                    aggregator.record_heuristic_failure(format!("Type fallback failed: {}", e));
                }
            }
        }
        info!(
            "[TIMING] Heuristic type fallback ({} unresolved): {:?}",
            unresolved_types.len(),
            t_heuristic_types.elapsed()
        );

        let containment_edges = match self.resolve_containment(syntax_results).await {
            Ok(edges) => {
                aggregator.add_edges(edges.clone());
                edges
            }
            Err(e) => {
                warn!("Containment resolution failed: {}", e);
                aggregator.record_lsp_failure(format!("Containment resolution failed: {}", e));
                Vec::new()
            }
        };

        aggregator.set_heuristic_fallbacks(
            heuristic_call_fallbacks,
            heuristic_import_fallbacks,
            heuristic_type_fallbacks,
        );

        let (all_edges, stats) = aggregator.into_parts();

        let lsp_edges = stats.lsp_edges;
        let heuristic_edges = stats.heuristic_edges;

        let mut call_edges = Vec::new();
        let mut resolved_import_edges = Vec::new();
        let mut resolved_type_edges = Vec::new();

        for edge in all_edges {
            match edge.kind {
                // Call-like family routes into the call-edges bucket so
                // ResolvedEdges keeps its "invoked-at-runtime" grouping.
                // Universal-fidelity T1 added Framework(_) / Declarative(_)
                // as typed variants of that same family; they share the
                // bucket because downstream consumers already pivot on
                // `EdgeKind` at the metric layer where granularity matters.
                EdgeKind::Call | EdgeKind::Framework(_) | EdgeKind::Declarative(_) => {
                    call_edges.push(edge)
                }
                EdgeKind::Import => resolved_import_edges.push(edge),
                // `Type`, `Uses`, `Extends`, `Implements` are all type-flavored
                // relationships and share the `type_edges` bucket. The edge
                // kind itself preserves the semantic distinction for
                // downstream consumers.
                EdgeKind::Type | EdgeKind::Uses | EdgeKind::Extends | EdgeKind::Implements => {
                    resolved_type_edges.push(edge)
                }
                EdgeKind::Contains => {}
            }
        }

        let resolved_edges = ResolvedEdges {
            call_edges,
            import_edges: resolved_import_edges,
            type_edges: resolved_type_edges,
            containment_edges,
            stats,
            // Subprocess-LSP path does not need the "already
            // semantically resolved" dedupe set because it emits
            // exactly one target per call site and the existing
            // `(caller_id, callee_id)` HashSet in `FallbackEdgeBuilder`
            // handles sibling-dedup in practice. See T6 Gate 1.2
            // commentary in `ports.rs::ResolvedEdges::resolved_call_sites`.
            resolved_call_sites: std::collections::HashSet::new(),
        };

        let total_resolution_time = resolution_start.elapsed();
        let total_edges = resolved_edges.call_edges.len()
            + resolved_edges.import_edges.len()
            + resolved_edges.type_edges.len()
            + resolved_edges.containment_edges.len();

        info!(
            "[5/5] Complete: {} edges (LSP={}, Heuristic={}) in {:?}",
            total_edges, lsp_edges, heuristic_edges, total_resolution_time
        );

        Ok(resolved_edges)
    }
}

#[async_trait]
impl SemanticResolver for LspResolver {
    async fn resolve(
        &self,
        syntax_results: &SyntaxResults,
    ) -> Result<ResolvedEdges, anyhow::Error> {
        self.resolve_inner(syntax_results).await
    }

    async fn is_available(&self) -> bool {
        let available = self.definition_provider.is_available().await;
        if !available {
            warn!(
                "Definition provider unavailable for '{}'; proceeding with heuristic-only resolution",
                self.config.language
            );
        }
        true
    }

    fn supported_language(&self) -> &str {
        &self.config.language
    }

    async fn session_metrics(&self) -> Option<crate::application::ports::SessionMetricsSnapshot> {
        self.definition_provider.session_metrics().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::create_default_rust_config;
    use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
    use crate::{
        application::ports::{
            ImportKind, ImportPath, ImportSpec, ImportVisibility, ModDecl, ModKind, PathRoot,
            SyntaxResults,
        },
        domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range},
        infrastructure::lsp::errors::LspError,
    };
    use async_trait::async_trait;
    use std::time::Duration;
    use std::{collections::HashMap, fs, sync::Mutex};
    use tempfile::tempdir;

    #[test]
    fn resolution_aggregator_counts_and_dedups() {
        let mut aggregator = ResolutionAggregator::default();

        let lsp_edge = Edge::new(
            "a".into(),
            "b".into(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Lsp, Confidence::High),
        );
        let heuristic_edge_same = Edge::new(
            "a".into(),
            "b".into(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
        );
        let heuristic_edge_unique = Edge::new(
            "c".into(),
            "d".into(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
        );

        aggregator.add_edges(vec![lsp_edge, heuristic_edge_same, heuristic_edge_unique]);

        let (edges, summary) = aggregator.into_parts();
        assert_eq!(edges.len(), 2, "duplicate edges should be collapsed");
        assert_eq!(summary.lsp_edges, 1);
        assert_eq!(summary.heuristic_edges, 1);
        assert_eq!(summary.total_call_edges(), 2);
    }

    #[test]
    fn resolution_aggregator_records_failures() {
        let mut aggregator = ResolutionAggregator::default();
        aggregator.record_lsp_failure("lsp down".to_string());
        aggregator.record_heuristic_failure("heuristic blew up".to_string());

        let (edges, summary) = aggregator.into_parts();
        assert!(edges.is_empty());
        assert_eq!(summary.lsp_failures.len(), 1);
        assert_eq!(summary.heuristic_failures.len(), 1);
    }

    #[tokio::test]
    async fn test_lsp_resolver_creation() {
        let config = create_default_rust_config();
        let resolver = LspResolver::new(config, None);

        assert_eq!(resolver.supported_language(), "rust");
    }

    #[tokio::test]
    async fn test_lsp_resolver_empty_syntax() {
        let config = create_default_rust_config();
        let resolver = LspResolver::new(config, None);

        let syntax_results = SyntaxResults::new();
        let result = resolver.resolve(&syntax_results).await;

        assert!(result.is_ok());
        let resolved_edges = result.unwrap();
        assert!(resolved_edges.call_edges.is_empty());
        assert!(resolved_edges.import_edges.is_empty());
        assert!(resolved_edges.type_edges.is_empty());
        assert!(resolved_edges.containment_edges.is_empty());
    }

    struct MockDefinitionProvider {
        definitions: HashMap<String, Range>,
        ready: Mutex<bool>,
    }

    impl MockDefinitionProvider {
        fn new(definitions: HashMap<String, Range>) -> Self {
            Self {
                definitions,
                ready: std::sync::Mutex::new(false),
            }
        }
    }

    #[async_trait]
    impl DefinitionProvider for MockDefinitionProvider {
        async fn is_available(&self) -> bool {
            true
        }

        async fn ensure_ready(&self) -> Result<(), LspError> {
            let mut guard = self.ready.lock().unwrap();
            *guard = true;
            Ok(())
        }

        async fn find_definition(
            &self,
            call_site: &crate::application::ports::CallSite,
        ) -> Result<Option<Range>, LspError> {
            Ok(self.definitions.get(&call_site.function_name).cloned())
        }

        async fn open_document(&self, _path: &str, _text: String) -> Result<(), LspError> {
            Ok(())
        }

        async fn close_document(&self, _path: &str) -> Result<(), LspError> {
            Ok(())
        }

        async fn wait_until_ready(&self, _timeout: Duration) -> Result<(), LspError> {
            Ok(())
        }

        async fn hover(&self, _location: &Range) -> Result<Option<String>, LspError> {
            // Mock implementation - return None for tests
            Ok(None)
        }
    }

    #[tokio::test]
    async fn resolve_calls_prefers_lsp_edges_when_available() {
        let config = Arc::new(create_default_rust_config());

        let mut definitions = HashMap::new();
        let callee_range = Range::with_file(10, 0, 20, 0, "src/foo.rs".to_string());
        definitions.insert("crate::foo::bar".to_string(), callee_range.clone());
        let provider = Arc::new(MockDefinitionProvider::new(definitions));

        let resolver = LspResolver::with_provider(config.clone(), provider);

        let mut syntax = SyntaxResults::new();

        let caller_node = Node {
            id: "caller".into(),
            kind: NodeKind::Function,
            fqn: "crate::caller".into(),
            location: Range::with_file(1, 0, 50, 0, "src/lib.rs".to_string()),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        };

        let callee_node = Node {
            id: "callee".into(),
            kind: NodeKind::Function,
            fqn: "crate::foo::bar".into(),
            location: callee_range,
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        };

        syntax.symbols.push(caller_node);
        syntax.symbols.push(callee_node);
        syntax.add_call_site(
            Range::with_file(5, 10, 5, 20, "src/lib.rs".to_string()),
            "crate::foo::bar".to_string(),
        );

        let result = resolver.resolve(&syntax).await.expect("resolve call");
        assert_eq!(result.call_edges.len(), 1);
        let edge = &result.call_edges[0];
        assert_eq!(edge.provenance.source, ProvenanceSource::Lsp);
        assert_eq!(edge.provenance.confidence, Confidence::High);
        assert_eq!(result.stats.lsp_edges, 1);
        assert!(result.stats.heuristic_edges <= 1);
    }

    #[tokio::test]
    async fn resolve_imports_produces_lsp_edges() {
        let config = Arc::new(create_default_rust_config());
        let temp = tempdir().expect("create temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();

        let lib_path = root.join("src/lib.rs");
        fs::write(
            &lib_path,
            "pub mod foo;\nuse crate::foo::target as alias_target;\n",
        )
        .unwrap();
        let foo_path = root.join("src/foo.rs");
        fs::write(&foo_path, "pub fn target() {}\n").unwrap();

        let lib_path_str = lib_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let foo_path_str = foo_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(Node::module(
            "crate".into(),
            Range::with_file(1, 0, 10, 0, lib_path_str.clone()),
        ));
        syntax.symbols.push(Node::function(
            "crate::foo::target".into(),
            Range::with_file(1, 0, 1, 15, foo_path_str.clone()),
        ));
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path_str.clone(),
            range: Range::with_file(1, 0, 1, 20, lib_path_str.clone()),
            kind: ModKind::External,
            resolved_file: Some(foo_path_str.clone()),
        });
        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(2, 0, 2, 40, lib_path_str.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["foo".into(), "target".into()]),
            alias: Some("alias_target".into()),
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: lib_path_str.clone(),
        });

        let mut definitions = HashMap::new();
        definitions.insert(
            "alias_target".into(),
            Range::with_file(1, 0, 1, 15, foo_path_str.clone()),
        );
        let provider = Arc::new(MockDefinitionProvider::new(definitions));

        let resolver = LspResolver::with_provider(config.clone(), provider);

        let result = resolver
            .resolve(&syntax)
            .await
            .expect("resolve imports via LSP");
        assert_eq!(result.import_edges.len(), 1);
        let edge = &result.import_edges[0];
        assert_eq!(edge.provenance.source, ProvenanceSource::Lsp);
        assert_eq!(result.stats.heuristic_import_fallbacks, 0);
    }

    #[tokio::test]
    async fn resolve_imports_falls_back_when_lsp_fails() {
        let config = Arc::new(create_default_rust_config());
        let temp = tempdir().expect("create temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();

        let lib_path = root.join("src/lib.rs");
        fs::write(
            &lib_path,
            "pub mod foo;\nuse crate::foo::target as alias_target;\n",
        )
        .unwrap();
        let foo_path = root.join("src/foo.rs");
        fs::write(&foo_path, "pub fn target() {}\n").unwrap();

        let lib_path_str = lib_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let foo_path_str = foo_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(Node::module(
            "crate".into(),
            Range::with_file(1, 0, 10, 0, lib_path_str.clone()),
        ));
        syntax.symbols.push(Node::function(
            "crate::foo::target".into(),
            Range::with_file(1, 0, 1, 15, foo_path_str.clone()),
        ));
        syntax.add_mod_decl(ModDecl {
            name: "foo".into(),
            source_file: lib_path_str.clone(),
            range: Range::with_file(1, 0, 1, 20, lib_path_str.clone()),
            kind: ModKind::External,
            resolved_file: Some(foo_path_str.clone()),
        });
        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(2, 0, 2, 40, lib_path_str.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["foo".into(), "target".into()]),
            alias: Some("alias_target".into()),
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: lib_path_str.clone(),
        });

        let module_resolver = ModuleResolver::from_syntax(&syntax);
        let alias_candidates =
            module_resolver.resolve_name_in_context(&lib_path_str, "alias_target");
        assert!(
            !alias_candidates.is_empty(),
            "module resolver should resolve alias bindings"
        );

        let definitions = HashMap::new();
        let provider = Arc::new(MockDefinitionProvider::new(definitions));

        let resolver = LspResolver::with_provider(config.clone(), provider);

        let result = resolver
            .resolve(&syntax)
            .await
            .expect("resolve imports via fallback");
        assert_eq!(
            result.import_edges.len(),
            1,
            "heuristic import edge expected"
        );
        let edge = &result.import_edges[0];
        assert_eq!(edge.provenance.source, ProvenanceSource::Heuristic);
        assert_eq!(result.stats.heuristic_import_fallbacks, 1);
    }

    #[tokio::test]
    async fn resolve_types_produces_lsp_edges() {
        let config = Arc::new(create_default_rust_config());
        let temp = tempdir().expect("create temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();

        let lib_path = root.join("src/lib.rs");
        let content = "struct MyType;\nfn uses_type() {\n    let value: MyType = MyType;\n}\n";
        fs::write(&lib_path, content).unwrap();

        let lib_path_str = lib_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let lines: Vec<&str> = content.lines().collect();
        let type_line = 3u32;
        let type_start = lines[2].find("MyType").unwrap() as u32;
        let type_range = Range::with_file(
            type_line,
            type_start,
            type_line,
            type_start + "MyType".len() as u32,
            lib_path_str.clone(),
        );

        let mut syntax = SyntaxResults::new();
        let struct_range = Range::with_file(1, 0, 1, "MyType".len() as u32, lib_path_str.clone());
        syntax
            .symbols
            .push(Node::struct_("crate::MyType".into(), struct_range.clone()));
        syntax.symbols.push(Node::function(
            "crate::uses_type".into(),
            Range::with_file(2, 0, 4, 1, lib_path_str.clone()),
        ));
        syntax.type_refs.push(type_range.clone());

        let mut definitions = HashMap::new();
        definitions.insert("MyType".into(), struct_range.clone());
        let provider = Arc::new(MockDefinitionProvider::new(definitions));

        let resolver = LspResolver::with_provider(config.clone(), provider);

        let result = resolver
            .resolve(&syntax)
            .await
            .expect("resolve types via LSP");
        assert_eq!(result.type_edges.len(), 1);
        let edge = &result.type_edges[0];
        assert_eq!(edge.provenance.source, ProvenanceSource::Lsp);
        assert_eq!(result.stats.heuristic_type_fallbacks, 0);
    }

    #[tokio::test]
    async fn resolve_types_falls_back_when_lsp_fails() {
        let config = Arc::new(create_default_rust_config());
        let temp = tempdir().expect("create temp dir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();

        let lib_path = root.join("src/lib.rs");
        let content = "struct MyType;\nfn uses_type() {\n    let value: MyType = MyType;\n}\n";
        fs::write(&lib_path, content).unwrap();

        let lib_path_str = lib_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let lines: Vec<&str> = content.lines().collect();
        let type_line = 3u32;
        let type_start = lines[2].find("MyType").unwrap() as u32;
        let type_range = Range::with_file(
            type_line,
            type_start,
            type_line,
            type_start + "MyType".len() as u32,
            lib_path_str.clone(),
        );

        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(Node::struct_(
            "crate::MyType".into(),
            Range::with_file(1, 0, 4, 1, lib_path_str.clone()),
        ));
        syntax.symbols.push(Node::function(
            "crate::uses_type".into(),
            Range::with_file(2, 0, 4, 1, lib_path_str.clone()),
        ));
        syntax.type_refs.push(type_range.clone());

        let definitions = HashMap::new();
        let provider = Arc::new(MockDefinitionProvider::new(definitions));

        let resolver = LspResolver::with_provider(config.clone(), provider);

        let result = resolver
            .resolve(&syntax)
            .await
            .expect("resolve types via fallback");
        assert_eq!(result.type_edges.len(), 1, "heuristic type edge expected");
        let edge = &result.type_edges[0];
        assert_eq!(edge.provenance.source, ProvenanceSource::Heuristic);
        assert_eq!(result.stats.heuristic_type_fallbacks, 1);
    }
}
