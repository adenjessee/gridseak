//! Type reference extraction
//!
//! Extracts type references from source code using Tree-sitter queries.
//! Captures both the type name and the context in which it's used.

use crate::application::ports::{SyntaxResults, TypeUsageKind};
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::utils::node_converter::node_to_range;
use anyhow::{Context, Result};
use std::sync::Arc;
use tree_sitter::Language;

/// Extracts type references from source code
pub struct TypeRefExtractor {
    language: Language,
    config: Arc<LanguageConfig>,
}

impl TypeRefExtractor {
    pub fn new(language: Language, config: Arc<LanguageConfig>) -> Self {
        Self { language, config }
    }

    /// Extract type references from the AST
    pub fn extract(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        self.extract_type_refs(root_node, content, file_path, results)?;
        self.extract_inheritance(root_node, content, file_path, results)?;
        Ok(())
    }

    /// Extract general type references
    fn extract_type_refs(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        if let Some(query_str) = self.config.get_query("type_refs") {
            let query = tree_sitter::Query::new(self.language, query_str)
                .with_context(|| format!("Invalid type_refs query: {}", query_str))?;

            let mut cursor = tree_sitter::QueryCursor::new();
            let matches = cursor.matches(&query, *root_node, content.as_bytes());
            let type_ref_idx = query.capture_index_for_name("type_ref");

            for mat in matches {
                for capture in mat.captures {
                    let range = node_to_range(&capture.node, file_path);

                    // Check if this is the type_ref capture
                    let is_type_ref = type_ref_idx == Some(capture.index);

                    if is_type_ref {
                        let type_name = capture
                            .node
                            .utf8_text(content.as_bytes())
                            .unwrap_or("")
                            .to_string();

                        if !type_name.is_empty() && !is_builtin_type(&type_name) {
                            results.add_type_reference(
                                range.clone(),
                                type_name,
                                TypeUsageKind::Other,
                            );
                        }
                    }

                    // Legacy: add first capture to type_refs
                    if capture.index == 0 {
                        results.add_type_ref(range);
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract inheritance relationships (extends/implements)
    fn extract_inheritance(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        if let Some(query_str) = self.config.get_query("inheritance") {
            let query = tree_sitter::Query::new(self.language, query_str)
                .with_context(|| format!("Invalid inheritance query: {}", query_str))?;

            let mut cursor = tree_sitter::QueryCursor::new();
            let matches = cursor.matches(&query, *root_node, content.as_bytes());

            let extends_idx = query.capture_index_for_name("extends_type");
            let implements_idx = query.capture_index_for_name("implements_type");

            for mat in matches {
                for capture in mat.captures {
                    let type_name = capture
                        .node
                        .utf8_text(content.as_bytes())
                        .unwrap_or("")
                        .to_string();

                    if type_name.is_empty() || is_builtin_type(&type_name) {
                        continue;
                    }

                    let range = node_to_range(&capture.node, file_path);
                    let is_extends = extends_idx == Some(capture.index);
                    let is_implements = implements_idx == Some(capture.index);

                    if is_extends {
                        results.add_type_reference(range, type_name, TypeUsageKind::Extends);
                    } else if is_implements {
                        results.add_type_reference(range, type_name, TypeUsageKind::Implements);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Check if a type name is a builtin/primitive type
fn is_builtin_type(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "string"
            | "number"
            | "boolean"
            | "void"
            | "null"
            | "undefined"
            | "any"
            | "unknown"
            | "never"
            | "object"
            | "symbol"
            | "bigint"
            | "true"
            | "false"
            | "array"
            | "promise"
            | "map"
            | "set"
            | "date"
            | "regexp"
            | "error"
            | "function"
    )
}
