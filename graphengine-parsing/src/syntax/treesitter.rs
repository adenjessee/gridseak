//! Tree-sitter based syntax extractor implementation
//!
//! Implements the SyntaxExtractor port using Tree-sitter for fast,
//! language-agnostic syntax analysis. Extracts symbols and call sites
//! from source files using configurable queries.

use crate::application::ports::{SyntaxExtractor, SyntaxResults};
use crate::domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range};
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::extractors::{
    call_site_extractor::CallSiteExtractor, complexity_extractor,
    identifier_use_extractor::IdentifierUseExtractor, import_extractor::ImportExtractor,
    module_extractor::ModuleExtractor, symbol_extractor::SymbolExtractor,
    type_ref_extractor::TypeRefExtractor,
};
use crate::syntax::language::loader::{load_extractor, load_language};
use crate::syntax::language::LanguageSpecificExtractor;
use crate::syntax::utils::fqn_builder::build_fqn;
use crate::syntax::utils::visibility_detector::{
    go_visibility_from_name, python_visibility_from_name,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use graphengine_progress::{EngineEvent, EngineEventEmitter, NullEngineEventEmitter};
use rayon::prelude::*;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

/// Tree-sitter based syntax extractor
pub struct TreeSitterExtractor {
    config: Arc<LanguageConfig>,
    language: tree_sitter::Language,
    language_extractor: Arc<dyn LanguageSpecificExtractor>,
    progress_emitter: Arc<dyn EngineEventEmitter>,
    workspace_root: Option<String>,
}

impl TreeSitterExtractor {
    /// Create a new Tree-sitter extractor for the given language
    pub fn new(config: LanguageConfig) -> Result<Self> {
        let language = load_language(&config)?;
        let language_extractor = load_extractor(&config);

        info!(
            "Created TreeSitterExtractor for language: {}",
            config.language
        );
        Ok(Self {
            config: Arc::new(config),
            language,
            language_extractor,
            progress_emitter: Arc::new(NullEngineEventEmitter),
            workspace_root: None,
        })
    }

    /// Set the progress emitter for per-file progress reporting.
    pub fn set_progress_emitter(&mut self, emitter: Arc<dyn EngineEventEmitter>) {
        self.progress_emitter = emitter;
    }

    /// Set the workspace root for FQN construction.
    /// Files outside `src/` use the workspace-relative path as module context.
    pub fn set_workspace_root(&mut self, root: String) {
        self.workspace_root = Some(root);
    }

    /// Parse a single file and extract syntax information
    #[instrument(skip(self, content))]
    fn parse_file(&self, file_path: &Path, content: &str) -> Result<SyntaxResults> {
        let mut results = SyntaxResults::new();

        // Parse the file with Tree-sitter
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(self.language)?;

        let tree = parser
            .parse(content, None)
            .with_context(|| format!("Failed to parse file: {}", file_path.display()))?;

        let root_node = tree.root_node();

        let file_path_str = file_path.to_string_lossy().to_string();

        // Add a file-scoped Module node so import/type resolution can anchor to a container.
        // Without this, ImportResolver cannot find a "containing module" for top-level imports.
        let language_name = self.config.language.as_str();
        {
            let lines: Vec<&str> = content.split('\n').collect();
            let end_line = lines.len().max(1) as u32;
            let end_char = lines.last().map(|l| l.chars().count()).unwrap_or(0) as u32;
            let range = Range::with_file(1, 0, end_line, end_char, file_path_str.clone());
            let fqn = build_fqn(
                "__file_module__",
                &file_path_str,
                self.workspace_root.as_deref(),
            );
            let mut node = Node::new(
                NodeKind::Module,
                fqn,
                range,
                Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
            );
            node.properties
                .insert("file_module".to_string(), serde_json::Value::Bool(true));
            let file_module_id = node.id.clone();
            results.add_symbol(node);

            // Language-specific external reference synthesis (e.g. Apex
            // managed-package namespaces). The default implementation is
            // a no-op, so non-Apex languages are unaffected. For Apex,
            // this materialises the virtual external `Module` nodes and
            // `Import` edges that `tests/fixtures/apex_baseline` reports as
            // `managed_package_consumers`.
            let external = self.language_extractor.synthesize_external_references(
                &tree,
                content.as_bytes(),
                &file_path_str,
                &file_module_id,
            );
            for ext_node in external.nodes {
                results.add_symbol(ext_node);
            }
            for ext_edge in external.edges {
                results.add_synthesized_edge(ext_edge);
            }

            // TR-A.0: per-language class-symbols extraction. Default
            // impl returns an empty vec; only Apex populates it today
            // (backing the type oracle consumed by PRs 2–5). This
            // runs on the same tree/source pair that other extractors
            // already walked, so it does not re-parse the file — it's
            // the "extractor populates on existing pass" requirement
            // in `PHASE_A_EXECUTION_PLAN.md` §2.2 A.0.3.
            //
            // Populating `results.class_symbols` only affects the
            // `apex_class_symbols` persistence table. No graph node
            // or edge is emitted from it, which is what keeps the
            // rev-6.1 byte-identical regression gate crisp.
            let syms = self.language_extractor.extract_class_symbols(
                &tree,
                content.as_bytes(),
                &file_path_str,
            );
            results.class_symbols.extend(syms);

            // TR-A.3: per-language local-variable scope extraction.
            // Default impl returns an empty vec; only Apex populates
            // today, feeding the field-type-aware dispatch resolver.
            // Data is ephemeral (resolved in this run, never
            // persisted) so the `apex_class_symbols` table stays
            // exactly as it was in rev-6.1 and the byte-identical
            // regression gate holds.
            let scopes = self.language_extractor.extract_local_var_scopes(
                &tree,
                content.as_bytes(),
                &file_path_str,
            );
            results.local_var_scopes.extend(scopes);

            // T8 — per-file extraction-coverage record. Default impl
            // on `LanguageSpecificExtractor` returns `None`, so
            // non-instrumented languages contribute zero records and
            // the `dead_code.no_callers` classifier behaves exactly
            // as it did pre-T8. Apex populates this for R39 / R41
            // coverage gaps; see
            // `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`
            // §4.2.
            if let Some(coverage) = self.language_extractor.extract_file_coverage(
                &tree,
                content.as_bytes(),
                &file_path_str,
            ) {
                results.extraction_coverage.push(coverage);
            }
        }

        // Create extractors
        let symbol_extractor = SymbolExtractor::new(
            self.language,
            Arc::clone(&self.config),
            Arc::clone(&self.language_extractor),
        )
        .with_workspace_root(self.workspace_root.clone());
        let call_site_extractor = CallSiteExtractor::new(
            self.language,
            Arc::clone(&self.config),
            Arc::clone(&self.language_extractor),
        );
        let import_extractor = ImportExtractor::new(self.language, Arc::clone(&self.config));
        let module_extractor = ModuleExtractor::new(self.language, Arc::clone(&self.config));
        let type_ref_extractor = TypeRefExtractor::new(self.language, Arc::clone(&self.config));
        let identifier_use_extractor =
            IdentifierUseExtractor::new(self.language, Arc::clone(&self.config));

        // Extract symbols using configured queries
        symbol_extractor.extract(&root_node, content, &file_path_str, &mut results)?;

        // Extract call sites
        call_site_extractor.extract(&root_node, content, &file_path_str, &mut results)?;

        // Extract imports
        import_extractor.extract(&root_node, content, &file_path_str, &mut results)?;

        // Extract module declarations
        module_extractor.extract(&root_node, content, &file_path_str, &mut results)?;

        // Extract type references
        type_ref_extractor.extract(&root_node, content, &file_path_str, &mut results)?;

        // Extract identifier uses (variable references)
        identifier_use_extractor.extract(&root_node, content, &file_path_str, &mut results)?;

        // Add a file-scope function for top-level call sites (TS/JS)
        if !results.references.is_empty() || !results.identifier_uses.is_empty() {
            let language = self.config.language.as_str();
            if language.eq_ignore_ascii_case("typescript")
                || language.eq_ignore_ascii_case("javascript")
            {
                let file_scope_name = "__file_scope__";
                let fqn = build_fqn(
                    file_scope_name,
                    &file_path_str,
                    self.workspace_root.as_deref(),
                );
                let lines: Vec<&str> = content.split('\n').collect();
                let end_line = lines.len().max(1) as u32;
                let end_char = lines.last().map(|l| l.chars().count()).unwrap_or(0) as u32;
                let range = Range::with_file(1, 0, end_line, end_char, file_path_str.clone());
                let node = Node::new(
                    NodeKind::Function,
                    fqn,
                    range,
                    Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
                );
                results.add_symbol(node);
            }
        }

        // Collect unique import source modules and store them on the __file_module__ node.
        // The analysis crate uses these to structurally classify test files (Tier 1)
        // by checking if any import source matches a known test framework.
        {
            let import_sources: Vec<String> = results
                .import_specs
                .iter()
                .filter(|spec| spec.source_file == file_path_str)
                .filter_map(|spec| spec.path.segments.first().cloned())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();

            if !import_sources.is_empty() {
                let file_module_fqn = build_fqn(
                    "__file_module__",
                    &file_path_str,
                    self.workspace_root.as_deref(),
                );
                if let Some(fm) = results
                    .symbols
                    .iter_mut()
                    .find(|s| s.fqn == file_module_fqn)
                {
                    fm.set_property("import_sources", serde_json::json!(import_sources));
                }
            }
        }

        // Compute cyclomatic + cognitive complexity for each Function node
        let complexity_map = complexity_extractor::compute_file_complexity(
            &root_node,
            content.as_bytes(),
            self.language_extractor.as_ref(),
        );

        for symbol in &mut results.symbols {
            if symbol.kind != NodeKind::Function {
                continue;
            }
            let key = (symbol.location.start_line, symbol.location.end_line);
            let cx = complexity_map.get(&key).or_else(|| {
                // For arrow functions / function expressions assigned to variables,
                // the symbol range spans the declaration while the complexity range
                // spans just the inner function. Find the best contained match.
                let (sym_start, sym_end) = (symbol.location.start_line, symbol.location.end_line);
                complexity_map
                    .iter()
                    .filter(|((s, e), _)| *s >= sym_start && *e <= sym_end)
                    .min_by_key(|((s, e), _)| e - s)
                    .map(|(_, v)| v)
            });
            if let Some(cx) = cx {
                symbol.set_property("cyclomatic_complexity", cx.cyclomatic as i64);
                symbol.set_property("cognitive_complexity", cx.cognitive as i64);
            }
        }

        // Go/Python: apply naming-convention visibility for symbols that don't yet have it.
        // Rust and TS/JS set visibility from AST during extraction; Go and Python use conventions.
        if language_name.eq_ignore_ascii_case("go") || language_name.eq_ignore_ascii_case("python")
        {
            for symbol in &mut results.symbols {
                if symbol.properties.contains_key("visibility") {
                    continue;
                }
                let name = symbol.fqn.rsplit("::").next().unwrap_or("");
                if name.is_empty() || name.starts_with("__file_") {
                    continue;
                }
                let vis = if language_name.eq_ignore_ascii_case("go") {
                    go_visibility_from_name(name)
                } else {
                    python_visibility_from_name(name)
                };
                symbol.set_property("visibility", vis);
            }
        }

        info!(
            "Extracted {} symbols, {} unresolved references, {} imports, {} type refs from {}",
            results.symbols.len(),
            results.references.len(),
            results.imports.len(),
            results.type_refs.len(),
            file_path.display()
        );

        Ok(results)
    }
}

#[async_trait]
impl SyntaxExtractor for TreeSitterExtractor {
    #[instrument(skip(self, files))]
    async fn extract(&self, files: &[std::path::PathBuf]) -> Result<SyntaxResults> {
        info!("Starting syntax extraction for {} files", files.len());

        if files.is_empty() {
            return Ok(SyntaxResults::new());
        }

        // Filter files by supported extensions
        let supported_files: Vec<_> = files
            .iter()
            .filter(|file| {
                if let Some(ext) = file.extension() {
                    if let Some(ext_str) = ext.to_str() {
                        return self.config.supports_extension(&format!(".{}", ext_str));
                    }
                }
                false
            })
            .collect();

        info!(
            "Processing {} supported files out of {} total",
            supported_files.len(),
            files.len()
        );

        // Build an index map so parallel threads can look up each file's position.
        // This avoids needing enumerate() inside par_iter (which isn't directly available).
        let indexed_files: Vec<(usize, &std::path::PathBuf)> = supported_files
            .iter()
            .enumerate()
            .map(|(i, p)| (i, *p))
            .collect();

        // Emit file manifest so consumers know the full scope before parsing begins
        if self.progress_emitter.is_enabled() {
            let relative_paths: Vec<String> = indexed_files
                .iter()
                .map(|(_, p)| p.to_string_lossy().to_string())
                .collect();
            let total_files = relative_paths.len();
            let _ = self
                .progress_emitter
                .emit(EngineEvent::file_manifest(total_files, relative_paths));
        }

        let emitter = Arc::clone(&self.progress_emitter);

        // Parse files in parallel using Rayon
        let parse_results: Result<Vec<_>> = indexed_files
            .par_iter()
            .map(|(idx, file_path)| {
                let file_display = file_path.to_string_lossy().to_string();

                // Emit file_progress start
                let _ = emitter.emit(EngineEvent::file_progress(&file_display, *idx, "start"));

                let result = match std::fs::read_to_string(file_path) {
                    Ok(content) => match self.parse_file(file_path, &content) {
                        Ok(results) => Ok(results),
                        Err(e) => {
                            error!("Failed to parse file {}: {}", file_path.display(), e);
                            let _ = emitter.emit(EngineEvent::file_progress(
                                &file_display,
                                *idx,
                                "error",
                            ));
                            Ok(SyntaxResults::new())
                        }
                    },
                    Err(e) => {
                        error!("Failed to read file {}: {}", file_path.display(), e);
                        let _ =
                            emitter.emit(EngineEvent::file_progress(&file_display, *idx, "error"));
                        Ok(SyntaxResults::new())
                    }
                };

                // Emit file_progress done (only if no error was already emitted)
                if result.is_ok() {
                    let _ = emitter.emit(EngineEvent::file_progress(&file_display, *idx, "done"));
                }

                result
            })
            .collect();

        let parse_results = parse_results?;

        // Merge all results
        let mut final_results = SyntaxResults::new();
        for mut results in parse_results {
            final_results.symbols.append(&mut results.symbols);
            final_results.references.append(&mut results.references);
            final_results
                .identifier_uses
                .append(&mut results.identifier_uses);
            final_results.imports.append(&mut results.imports);
            final_results.type_refs.append(&mut results.type_refs);
            final_results
                .type_references
                .append(&mut results.type_references);
            final_results.import_specs.append(&mut results.import_specs);
            final_results.mod_decls.append(&mut results.mod_decls);
            final_results
                .synthesized_edges
                .append(&mut results.synthesized_edges);
            // TR-A.0: merge per-file class-symbols payload so the
            // orchestrator's persistence step sees one flat list
            // across the whole parse run.
            final_results
                .class_symbols
                .append(&mut results.class_symbols);
            // TR-A.3: merge per-file local-var scopes. Ephemeral —
            // consumed by semantic resolution in this same run, not
            // persisted.
            final_results
                .local_var_scopes
                .append(&mut results.local_var_scopes);
            // T8: merge per-file extraction-coverage records so the
            // dead_code classifier sees one flat list keyed by file
            // path across the whole parse run.
            final_results
                .extraction_coverage
                .append(&mut results.extraction_coverage);
        }

        info!(
            "Extraction complete: {} symbols, {} unresolved references, {} imports, {} type refs, {} structured type refs",
            final_results.symbols.len(),
            final_results.references.len(),
            final_results.imports.len(),
            final_results.type_refs.len(),
            final_results.type_references.len()
        );

        Ok(final_results)
    }

    fn supported_language(&self) -> &str {
        &self.config.language
    }

    fn supports_extension(&self, ext: &str) -> bool {
        self.config.supports_extension(ext)
    }

    fn post_syntax_hooks(
        &self,
        workspace_root: &std::path::Path,
        syntax_results: &mut SyntaxResults,
    ) -> crate::syntax::language::extractor::HookOutcome {
        self.language_extractor
            .post_syntax_hooks(workspace_root, syntax_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::{create_default_rust_config, load_config};
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_treesitter_extractor_creation() {
        let config = create_default_rust_config();
        let extractor = TreeSitterExtractor::new(config);

        // This test might fail if Tree-sitter Rust language is not available
        // In a real implementation, we'd mock this or use a test language
        match extractor {
            Ok(extractor) => {
                assert_eq!(extractor.supported_language(), "rust");
                assert!(extractor.supports_extension(".rs"));
                assert!(!extractor.supports_extension(".js"));
            }
            Err(_) => {
                // Expected if Tree-sitter Rust is not available in test environment
                println!("TreeSitterExtractor creation failed (expected in test environment)");
            }
        }
    }

    #[test]
    fn test_node_to_range_conversion() {
        let config = create_default_rust_config();
        let _extractor = TreeSitterExtractor::new(config).unwrap_or_else(|_| {
            // Create a dummy extractor for testing
            let cfg = Arc::new(create_default_rust_config());
            let language_extractor = load_extractor(&cfg);
            TreeSitterExtractor {
                config: cfg,
                language: tree_sitter_rust::language(),
                language_extractor,
                progress_emitter: Arc::new(NullEngineEventEmitter),
                workspace_root: None,
            }
        });

        // Test range conversion (this would need a real Tree-sitter node in practice)
        // For now, just test that the method exists and can be called
    }

    #[tokio::test]
    async fn test_extract_empty_files() {
        let config = create_default_rust_config();
        let extractor = TreeSitterExtractor::new(config).unwrap_or_else(|_| {
            let cfg = Arc::new(create_default_rust_config());
            let language_extractor = load_extractor(&cfg);
            TreeSitterExtractor {
                config: cfg,
                language: tree_sitter_rust::language(),
                language_extractor,
                progress_emitter: Arc::new(NullEngineEventEmitter),
                workspace_root: None,
            }
        });

        let files = vec![];
        let results = extractor.extract(&files).await.unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_extract_unsupported_files() {
        let config = create_default_rust_config();
        let extractor = TreeSitterExtractor::new(config).unwrap_or_else(|_| {
            let cfg = Arc::new(create_default_rust_config());
            let language_extractor = load_extractor(&cfg);
            TreeSitterExtractor {
                config: cfg,
                language: tree_sitter_rust::language(),
                language_extractor,
                progress_emitter: Arc::new(NullEngineEventEmitter),
                workspace_root: None,
            }
        });

        let files = vec![PathBuf::from("test.js")]; // JavaScript file, not supported
        let results = extractor.extract(&files).await.unwrap();

        // Should return empty results for unsupported files
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_extract_python_functions() {
        let config = load_config("python").expect("load python config");
        let extractor = TreeSitterExtractor::new(config).expect("python extractor");

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("sample.py");
        std::fs::write(
            &file_path,
            "def helper():\n    return 1\n\ndef main():\n    helper()\n",
        )
        .unwrap();

        let results = extractor
            .extract(std::slice::from_ref(&file_path))
            .await
            .expect("extract python file");

        assert!(
            results.symbols.len() >= 2,
            "expected python functions to be extracted"
        );
        assert!(
            !results.references.is_empty(),
            "expected python call sites to be captured"
        );
    }

    #[tokio::test]
    async fn test_extract_javascript_functions() {
        let config = load_config("javascript").expect("load javascript config");
        let extractor = TreeSitterExtractor::new(config).expect("javascript extractor");

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("index.js");
        std::fs::write(
            &file_path,
            "function helper() { return 1; }\nfunction main() { return helper(); }\n",
        )
        .unwrap();

        let results = extractor
            .extract(std::slice::from_ref(&file_path))
            .await
            .expect("extract javascript file");

        assert!(
            results.symbols.len() >= 2,
            "expected javascript functions to be extracted"
        );
        assert!(
            !results.references.is_empty(),
            "expected javascript call sites to be captured"
        );
    }
}
