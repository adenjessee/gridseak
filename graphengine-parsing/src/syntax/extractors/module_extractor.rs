//! Module declaration extraction
//!
//! Extracts module declarations from source code using Tree-sitter queries
//! and Rust-specific parsing.

use crate::application::ports::SyntaxResults;
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::rust::module_parser::parse_mod_decl;
use crate::syntax::utils::node_converter::node_to_range;
use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::warn;
use tree_sitter::Language;

/// Extracts module declarations from source code
pub struct ModuleExtractor {
    language: Language,
    config: Arc<LanguageConfig>,
}

impl ModuleExtractor {
    pub fn new(language: Language, config: Arc<LanguageConfig>) -> Self {
        Self { language, config }
    }

    /// Extract module declarations from the AST
    pub fn extract(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        if let Some(query_str) = self.config.get_query("modules") {
            let query = tree_sitter::Query::new(self.language, query_str)
                .with_context(|| format!("Invalid modules query: {}", query_str))?;

            let mut cursor = tree_sitter::QueryCursor::new();
            let matches = cursor.matches(&query, *root_node, content.as_bytes());

            for mat in matches {
                for capture in mat.captures {
                    let capture_name = &query.capture_names()[capture.index as usize];
                    if capture_name != "module" {
                        continue;
                    }

                    let range = node_to_range(&capture.node, file_path);
                    let text = match capture.node.utf8_text(content.as_bytes()) {
                        Ok(text) => text,
                        Err(err) => {
                            warn!(
                                "Failed to read module declaration text in {}: {}",
                                file_path, err
                            );
                            continue;
                        }
                    };

                    if let Some(decl) = parse_mod_decl(text, file_path, range.clone()) {
                        results.add_mod_decl(decl);
                    }
                }
            }
        }

        Ok(())
    }
}
