//! Symbol extraction
//!
//! Extracts symbols (functions, structs, modules, traits, enums, etc.) from source code
//! using Tree-sitter queries. Delegates trait-aware function extraction to TraitContextDetector.

use crate::application::ports::SyntaxResults;
use crate::domain::{Confidence, Edge, Node, NodeKind, Provenance, ProvenanceSource};
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::extractors::trait_context_detector::TraitContextDetector;
use crate::syntax::language::LanguageSpecificExtractor;
use crate::syntax::utils::{
    fqn_builder::build_fqn, name_validator::is_reserved_keyword, node_converter::node_to_range,
    visibility_detector::detect_visibility_from_ast,
};
use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::warn;
use tree_sitter::Language;

/// Extracts symbols from source code
pub struct SymbolExtractor {
    language: Language,
    config: Arc<LanguageConfig>,
    language_extractor: Arc<dyn LanguageSpecificExtractor>,
    workspace_root: Option<String>,
}

impl SymbolExtractor {
    pub fn new(
        language: Language,
        config: Arc<LanguageConfig>,
        language_extractor: Arc<dyn LanguageSpecificExtractor>,
    ) -> Self {
        Self {
            language,
            config,
            language_extractor,
            workspace_root: None,
        }
    }

    pub fn with_workspace_root(mut self, root: Option<String>) -> Self {
        self.workspace_root = root;
        self
    }

    /// Extract symbols (functions, structs, modules, etc.) from the AST
    ///
    /// This method coordinates extraction of different symbol types.
    /// For functions, it delegates to trait-aware extraction if available.
    pub fn extract(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        // Extract functions with trait context awareness
        if let Some(query_str) = self.config.get_query("functions") {
            let trait_detector = TraitContextDetector::new(
                self.language,
                Arc::clone(&self.config),
                Arc::clone(&self.language_extractor),
            )
            .with_workspace_root(self.workspace_root.clone());
            trait_detector.extract_functions_with_trait_context(
                root_node, content, query_str, file_path, results,
            )?;
        }

        // Extract structs
        if let Some(query_str) = self.config.get_query("structs") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Struct,
                file_path,
                results,
            )?;
        }

        // Extract modules
        if let Some(query_str) = self.config.get_query("modules") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Module,
                file_path,
                results,
            )?;
        }

        // Extract traits
        if let Some(query_str) = self.config.get_query("traits") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Interface,
                file_path,
                results,
            )?;
        }

        // Extract enums
        if let Some(query_str) = self.config.get_query("enums") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Enum,
                file_path,
                results,
            )?;
        }

        // Extract constants and statics
        if let Some(query_str) = self.config.get_query("constants") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Variable,
                file_path,
                results,
            )?;
        }

        if let Some(query_str) = self.config.get_query("statics") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Variable,
                file_path,
                results,
            )?;
        }

        // Extract type aliases
        if let Some(query_str) = self.config.get_query("type_aliases") {
            self.extract_with_query(
                root_node,
                content,
                query_str,
                NodeKind::Type,
                file_path,
                results,
            )?;
        }

        Ok(())
    }

    /// Extract symbols using a specific query (for non-function symbols)
    fn extract_with_query(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        query_str: &str,
        node_kind: NodeKind,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        let query = tree_sitter::Query::new(self.language, query_str)
            .with_context(|| format!("Invalid query: {}", query_str))?;

        let mut cursor = tree_sitter::QueryCursor::new();
        let matches = cursor.matches(&query, *root_node, content.as_bytes());

        for mat in matches {
            let mut name = None;
            let mut range = None;
            let mut main_node: Option<tree_sitter::Node> = None;
            // Apex-only: `trigger_declaration` captures carry the SObject
            // identifier under `@sobject`. We record it as a node property
            // so downstream analysis can build `Trigger -> SObject` edges
            // without re-parsing the file.
            let mut sobject: Option<String> = None;
            // Apex-only flag: the `structs` query routes both classes AND
            // triggers to NodeKind::Struct via two top-level captures
            // (`@struct` and `@trigger`). We remember which branch fired so
            // the subtype property can be tagged.
            let mut is_trigger = false;

            for capture in mat.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                match capture_name.as_str() {
                    "func" | "struct" | "module" | "trait" | "enum" | "const" | "static"
                    | "type_alias" => {
                        range = Some(node_to_range(&capture.node, file_path));
                        main_node = Some(capture.node);
                    }
                    "trigger" => {
                        range = Some(node_to_range(&capture.node, file_path));
                        main_node = Some(capture.node);
                        is_trigger = true;
                    }
                    "name" => {
                        name = Some(capture.node.utf8_text(content.as_bytes())?.to_string());
                    }
                    "sobject" => {
                        sobject = Some(capture.node.utf8_text(content.as_bytes())?.to_string());
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(range)) = (name, range) {
                // R46: language-scoped keyword filter. The previous
                // language-blind filter dropped any name that was reserved
                // in ANY supported language — e.g. Apex methods named
                // `match` (Rust keyword), `type` (TS keyword), `lambda`
                // (Python keyword). Route through the current extractor's
                // language so we only filter words that are actually
                // reserved in the language being parsed.
                if is_reserved_keyword(&name, self.language_extractor.language()) {
                    warn!(
                        "[SYMBOL_EXTRACT] Skipping reserved keyword as {:?} name: '{}' in {}",
                        node_kind, name, file_path
                    );
                    continue;
                }

                // Start with the shared path-based FQN, then let the
                // language-specific extractor override it when needed.
                // Apex uses this to encode inner-class paths and method
                // parameter signatures (Sprint E.2); every other language
                // returns `None` and keeps the default shape.
                let mut fqn = build_fqn(&name, file_path, self.workspace_root.as_deref());
                if let Some(ast_node) = main_node.as_ref() {
                    if let Some(override_fqn) = self.language_extractor.build_symbol_fqn(
                        ast_node,
                        content.as_bytes(),
                        &name,
                        file_path,
                        self.workspace_root.as_deref(),
                    ) {
                        fqn = override_fqn;
                    }
                }
                let provenance = Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium);
                // T2: use the AST body bytes to stamp a content-based ID.
                // This keeps the ID stable across formatter passes, blank
                // line insertions, and comment edits that don't change the
                // symbol's semantics. If we couldn't slice UTF-8 cleanly
                // (mal-encoded source), fall back to the FQN-only ID rather
                // than panicking.
                let body_text: Option<&str> = main_node
                    .as_ref()
                    .and_then(|n| n.utf8_text(content.as_bytes()).ok());
                let language = self.config.language.as_str();
                let mut node = match body_text {
                    Some(body) => {
                        Node::with_body(node_kind, fqn, range, provenance, body, Some(language))
                    }
                    None => Node::new(node_kind, fqn, range, provenance),
                };

                if let Some(ast_node) = main_node {
                    if let Some(vis) = detect_visibility_from_ast(
                        &ast_node,
                        self.config.language.as_str(),
                        content.as_bytes(),
                    ) {
                        node.set_property("visibility", vis);
                    }

                    // Pre-refactor behaviour: Rust only tests at Module granularity
                    // here (classes/structs/enums are never flagged); Java/C# test
                    // every symbol kind. Encoded by letting each extractor gate on
                    // its own rules — Rust's `is_test_symbol` hits only on modules
                    // via the attribute presence, other languages hit on all kinds.
                    let is_rust_non_module = self.language_extractor.language() == "rust"
                        && node_kind != NodeKind::Module;
                    if !is_rust_non_module
                        && self
                            .language_extractor
                            .is_test_symbol(&ast_node, content.as_bytes())
                    {
                        node.set_property("is_test", true);
                    }

                    // Entry-point tagging (externally-reachable APIs).
                    // Apex uses this for @AuraEnabled / @RestResource / Queueable
                    // etc.; all other languages default to an empty tag set.
                    // The YAML `annotations` query is the single source of truth
                    // for grammar shape — we pass it through so the impl doesn't
                    // duplicate the shape in Rust constants.
                    let annotation_query = self.config.get_query("annotations").map(|s| s.as_str());
                    let tags = self.language_extractor.entry_point_tags(
                        &ast_node,
                        content.as_bytes(),
                        annotation_query,
                    );
                    if !tags.is_empty() {
                        node.set_property("entry_points", serde_json::json!(tags));
                    }

                    // Language-specific struct/class metadata (Apex sharing
                    // model today). Restrict to type-like kinds so we don't
                    // ask the extractor to inspect every module/function.
                    if matches!(
                        node_kind,
                        NodeKind::Struct | NodeKind::Interface | NodeKind::Enum
                    ) {
                        for (key, value) in self
                            .language_extractor
                            .extract_struct_metadata(&ast_node, content.as_bytes())
                        {
                            node.set_property(key, value);
                        }
                    }
                }

                // Apex trigger-specific properties. Attaching these on the
                // Struct node avoids introducing a dedicated NodeKind per
                // the plan's "no new NodeKind" decision.
                if is_trigger {
                    node.set_property("subtype", "trigger");
                    if let Some(sobj) = sobject.as_deref() {
                        node.set_property("sobject", sobj);
                    }
                    // Sprint E.4: surface the trigger's declared
                    // events (before insert / after update / ...) via
                    // the YAML-defined `trigger_events` query. The
                    // language trait method is a no-op for any
                    // language that hasn't bound that query.
                    if let (Some(ast_node), Some(events_query)) =
                        (main_node, self.config.get_query("trigger_events"))
                    {
                        for (key, value) in self.language_extractor.extract_trigger_metadata(
                            &ast_node,
                            content.as_bytes(),
                            events_query,
                            self.language,
                        ) {
                            node.set_property(key, value);
                        }
                    }
                }

                // Language-specific sibling synthesis (Sprint E.3).
                // Apex uses this to emit a `__trigger__` Function node
                // that owns the trigger-body range so top-level calls
                // resolve their caller. Every other language returns
                // an empty vec and this loop is a no-op.
                let siblings = if let Some(ast_node) = main_node {
                    self.language_extractor.synthesize_symbol_siblings(
                        &ast_node,
                        content.as_bytes(),
                        &node,
                        file_path,
                        self.workspace_root.as_deref(),
                    )
                } else {
                    Vec::new()
                };

                let parent_id = node.id.clone();
                results.add_symbol(node);
                for sibling in siblings {
                    let contain_edge = Edge::contains(
                        parent_id.clone(),
                        sibling.id.clone(),
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
                    );
                    results.add_synthesized_edge(contain_edge);
                    results.add_symbol(sibling);
                }
            }
        }

        Ok(())
    }
}
