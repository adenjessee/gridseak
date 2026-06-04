//! Trait context detection for function extraction
//!
//! Detects whether functions are in trait definitions, trait implementations,
//! or are regular functions. This enables proper differentiation and prevents
//! circular call issues.

use crate::application::ports::SyntaxResults;
use crate::domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, TraitMetadata};
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::language::LanguageSpecificExtractor;
use crate::syntax::utils::{
    fqn_builder::build_fqn, name_validator::is_reserved_keyword, node_converter::node_to_range,
    visibility_detector::detect_visibility_from_ast,
};
use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::{debug, warn};
use tree_sitter::Language;

/// Detects trait context for functions
pub struct TraitContextDetector {
    language: Language,
    config: Arc<LanguageConfig>,
    language_extractor: Arc<dyn LanguageSpecificExtractor>,
    workspace_root: Option<String>,
}

impl TraitContextDetector {
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

    /// Extract functions with full context awareness (trait/impl blocks)
    ///
    /// This method properly identifies:
    /// - Functions in trait definitions (trait defaults)
    /// - Functions in impl blocks (trait implementations)
    /// - Regular functions (non-trait methods)
    ///
    /// This ensures proper differentiation and prevents circular call issues.
    pub fn extract_functions_with_trait_context(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        query_str: &str,
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
            let mut trait_name = None;
            let mut impl_type = None;
            let mut is_trait_default = false;
            let mut is_trait_signature = false;
            let mut is_impl_method = false;
            let mut func_node: Option<tree_sitter::Node> = None;

            // Extract function_item from captures
            for capture in mat.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                if capture_name == "func" {
                    func_node = Some(capture.node);
                } else if capture_name == "name" {
                    name = Some(capture.node.utf8_text(content.as_bytes())?.to_string());
                }
            }

            // If we found a function, walk up the tree to determine context
            if let Some(func) = func_node {
                // Extract function name and range
                if name.is_none() {
                    if let Some(name_node) = func.child_by_field_name("name") {
                        name = name_node
                            .utf8_text(content.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                }
                range = Some(node_to_range(&func, file_path));

                // Walk up the tree to find parent trait_item or impl_item
                let mut current = func.parent();
                let mut depth = 0;
                while let Some(parent) = current {
                    depth += 1;
                    let parent_kind = parent.kind();

                    // Debug: log parent kinds we encounter
                    if depth <= 5 {
                        debug!(
                            "[TRAIT_DETECT] Function '{}' parent[{}]: {}",
                            name.as_deref().unwrap_or("unknown"),
                            depth,
                            parent_kind
                        );
                    }

                    match parent_kind {
                        "declaration_list" => {
                            // declaration_list can be inside trait_item OR impl_item
                            // Check the grandparent to determine which
                            debug!(
                                "[TRAIT_DETECT] Found declaration_list for function '{}'",
                                name.as_deref().unwrap_or("unknown")
                            );
                            if let Some(grandparent) = parent.parent() {
                                match grandparent.kind() {
                                    "trait_item" => {
                                        // This function is inside a trait
                                        // Check if it has a body (default) or is just a signature
                                        let has_body = func.child_by_field_name("body").is_some();
                                        if has_body {
                                            is_trait_default = true;
                                            debug!(
                                                "[TRAIT_DETECT] Found trait_item with body (default)"
                                            );
                                        } else {
                                            is_trait_signature = true;
                                            debug!(
                                                "[TRAIT_DETECT] Found trait_item without body (signature)"
                                            );
                                        }
                                        // Extract trait name
                                        if let Some(trait_name_node) =
                                            grandparent.child_by_field_name("name")
                                        {
                                            trait_name = trait_name_node
                                                .utf8_text(content.as_bytes())
                                                .ok()
                                                .map(|s| s.to_string());
                                            debug!("[TRAIT_DETECT] Trait name: {:?}", trait_name);
                                        }
                                        break;
                                    }
                                    "impl_item" => {
                                        // This function is inside an impl - it's an implementation
                                        is_impl_method = true;
                                        debug!(
                                            "[TRAIT_DETECT] Found impl_item, extracting trait info"
                                        );
                                        // Extract trait name from trait_bounds
                                        if let Some(trait_bounds) =
                                            grandparent.child_by_field_name("trait_bounds")
                                        {
                                            let mut bounds_cursor = trait_bounds.walk();
                                            for child in
                                                trait_bounds.named_children(&mut bounds_cursor)
                                            {
                                                if child.kind() == "trait_bound" {
                                                    if let Some(type_node) =
                                                        child.child_by_field_name("type")
                                                    {
                                                        trait_name = type_node
                                                            .utf8_text(content.as_bytes())
                                                            .ok()
                                                            .map(|s| s.to_string());
                                                        debug!(
                                                            "[TRAIT_DETECT] Trait name from bounds: {:?}",
                                                            trait_name
                                                        );
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        // Extract implementing type
                                        if let Some(type_node) =
                                            grandparent.child_by_field_name("type")
                                        {
                                            impl_type = type_node
                                                .utf8_text(content.as_bytes())
                                                .ok()
                                                .map(|s| s.to_string());
                                            debug!(
                                                "[TRAIT_DETECT] Implementing type: {:?}",
                                                impl_type
                                            );
                                        }
                                        break;
                                    }
                                    _ => {
                                        // declaration_list but not in trait or impl - continue walking
                                    }
                                }
                            }
                        }
                        "trait_body" => {
                            // Fallback: some tree-sitter versions might use trait_body
                            debug!(
                                "[TRAIT_DETECT] Found trait_body for function '{}'",
                                name.as_deref().unwrap_or("unknown")
                            );
                            let mut trait_cursor = parent.parent();
                            while let Some(trait_node) = trait_cursor {
                                if trait_node.kind() == "trait_item" {
                                    is_trait_default = true;
                                    debug!(
                                        "[TRAIT_DETECT] Found trait_item, extracting trait name"
                                    );
                                    if let Some(trait_name_node) =
                                        trait_node.child_by_field_name("name")
                                    {
                                        trait_name = trait_name_node
                                            .utf8_text(content.as_bytes())
                                            .ok()
                                            .map(|s| s.to_string());
                                        debug!("[TRAIT_DETECT] Trait name: {:?}", trait_name);
                                    }
                                    break;
                                }
                                trait_cursor = trait_node.parent();
                            }
                            break;
                        }
                        _ => {}
                    }
                    current = parent.parent();
                }
            }

            // If we found a function, extract it with proper context
            if let (Some(name), Some(range)) = (name, range) {
                // Filter out reserved keywords that shouldn't be function names
                // This catches parser bugs where control flow statements are misidentified.
                //
                // R46: language-scoped filter. See `name_validator.rs` for the
                // full rationale — before this was language-aware, Apex
                // methods named `match` (a Rust keyword) were silently
                // dropped because the single-list filter applied keywords
                // from every supported language to every extraction.
                if is_reserved_keyword(&name, self.language_extractor.language()) {
                    warn!(
                        "[TRAIT_DETECT] Skipping reserved keyword as function name: '{}' in {}",
                        name, file_path
                    );
                    continue;
                }

                // Shared path-based FQN first; languages like Apex
                // override to encode enclosing-class dotted paths and
                // method parameter signatures (Sprint E.2).
                let mut fqn = build_fqn(&name, file_path, self.workspace_root.as_deref());
                if let Some(func) = func_node.as_ref() {
                    if let Some(override_fqn) = self.language_extractor.build_symbol_fqn(
                        func,
                        content.as_bytes(),
                        &name,
                        file_path,
                        self.workspace_root.as_deref(),
                    ) {
                        fqn = override_fqn;
                    }
                }
                let provenance = Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium);

                // T2: compute content-based IDs from the AST body. For
                // trait signatures without a default body the body text
                // may be just the signature — which is still a stable,
                // deterministic string that churns only on real rename /
                // signature changes.
                let body_text: Option<&str> = func_node
                    .as_ref()
                    .and_then(|n| n.utf8_text(content.as_bytes()).ok());
                let language = self.config.language.as_str();

                // Create node with trait metadata if applicable
                // Note: We mark BOTH trait signatures (no body) AND trait defaults (with body) as trait methods
                // This allows filtering to work correctly - trait signatures should be filtered when implementations exist
                let mut node = if is_trait_default || is_trait_signature || is_impl_method {
                    if let Some(trait_name_val) = trait_name {
                        let trait_metadata = TraitMetadata {
                            trait_name: trait_name_val.clone(),
                            is_trait_default,
                            implementing_type: if is_impl_method { impl_type } else { None },
                        };
                        match body_text {
                            Some(body) => Node::with_trait_metadata_and_body(
                                NodeKind::Function,
                                fqn,
                                range,
                                provenance,
                                trait_metadata,
                                body,
                                Some(language),
                            ),
                            None => Node::with_trait_metadata(
                                NodeKind::Function,
                                fqn,
                                range,
                                provenance,
                                trait_metadata,
                            ),
                        }
                    } else {
                        match body_text {
                            Some(body) => Node::with_body(
                                NodeKind::Function,
                                fqn,
                                range,
                                provenance,
                                body,
                                Some(language),
                            ),
                            None => Node::new(NodeKind::Function, fqn, range, provenance),
                        }
                    }
                } else {
                    match body_text {
                        Some(body) => Node::with_body(
                            NodeKind::Function,
                            fqn,
                            range,
                            provenance,
                            body,
                            Some(language),
                        ),
                        None => Node::new(NodeKind::Function, fqn, range, provenance),
                    }
                };

                if let Some(func) = func_node {
                    if let Some(vis) = detect_visibility_from_ast(
                        &func,
                        self.config.language.as_str(),
                        content.as_bytes(),
                    ) {
                        node.set_property("visibility", vis);
                    }

                    if self
                        .language_extractor
                        .is_test_function(&func, content.as_bytes())
                    {
                        node.set_property("is_test", true);
                    }

                    let annotation_query = self.config.get_query("annotations").map(|s| s.as_str());
                    let tags = self.language_extractor.entry_point_tags(
                        &func,
                        content.as_bytes(),
                        annotation_query,
                    );
                    if !tags.is_empty() {
                        node.set_property("entry_points", serde_json::json!(tags));
                    }
                }

                results.add_symbol(node);
            }
        }

        Ok(())
    }
}
