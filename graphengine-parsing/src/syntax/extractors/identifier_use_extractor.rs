//! Identifier usage extraction
//!
//! Extracts identifier references (variable usages) from source code using Tree-sitter queries.

use crate::application::ports::SyntaxResults;
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::utils::node_converter::node_to_range;
use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::debug;
use tree_sitter::Language;

/// Extracts identifier usages from source code
pub struct IdentifierUseExtractor {
    language: Language,
    config: Arc<LanguageConfig>,
}

impl IdentifierUseExtractor {
    pub fn new(language: Language, config: Arc<LanguageConfig>) -> Self {
        Self { language, config }
    }

    /// Extract identifier uses from the AST
    pub fn extract(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        if let Some(query_str) = self.config.get_query("identifier_uses") {
            debug!("Extracting identifier uses from file: {}", file_path);
            let query = tree_sitter::Query::new(self.language, query_str)
                .with_context(|| format!("Invalid identifier_uses query: {}", query_str))?;

            let mut cursor = tree_sitter::QueryCursor::new();
            let matches = cursor.matches(&query, *root_node, content.as_bytes());

            for mat in matches {
                for capture in mat.captures {
                    let capture_name = &query.capture_names()[capture.index as usize];
                    if capture_name != "identifier" {
                        continue;
                    }
                    let node = capture.node;
                    if Self::is_declaration_context(&node) {
                        continue;
                    }
                    let name = node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let range = node_to_range(&node, file_path);
                    results.add_identifier_use(range, name);
                }
            }
        }

        Ok(())
    }

    fn is_declaration_context(node: &tree_sitter::Node) -> bool {
        let parent = node.parent();
        let parent_kind = parent.map(|p| p.kind()).unwrap_or("");
        matches!(
            parent_kind,
            "variable_declarator"
                | "function_declaration"
                | "function"
                | "method_definition"
                | "class_declaration"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "import_specifier"
                | "import_clause"
                | "namespace_import"
                | "formal_parameter"
                | "required_parameter"
                | "optional_parameter"
                | "property_signature"
        )
    }
}
