//! Tree-sitter query execution utilities
//!
//! Provides abstractions for executing Tree-sitter queries and processing
//! their results, reducing duplication across extraction methods.

use anyhow::{Context, Result};
use tree_sitter::{Language, Node, Query, QueryCursor};

/// Execute a Tree-sitter query and process matches
///
/// This is a generic query executor that handles the common pattern of:
/// 1. Creating a query from a query string
/// 2. Executing the query against a root node
/// 3. Processing matches with a callback
///
/// # Arguments
/// * `language` - The Tree-sitter language
/// * `query_str` - The query string
/// * `root_node` - The root node to query against
/// * `content` - The source code content
/// * `process_match` - Callback to process each match
///
/// # Returns
/// `Result<()>` indicating success or failure
pub fn execute_query<F>(
    language: Language,
    query_str: &str,
    root_node: Node,
    content: &str,
    mut process_match: F,
) -> Result<()>
where
    F: FnMut(&Query, &tree_sitter::QueryMatch, &str) -> Result<()>,
{
    let query = Query::new(language, query_str)
        .with_context(|| format!("Invalid query: {}", query_str))?;

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, root_node, content.as_bytes());

    for mat in matches {
        process_match(&query, &mat, content)?;
    }

    Ok(())
}

/// Execute a query and collect captures by name
///
/// This is a convenience function for queries where you want to extract
/// specific captures by their names.
///
/// # Arguments
/// * `language` - The Tree-sitter language
/// * `query_str` - The query string
/// * `root_node` - The root node to query against
/// * `content` - The source code content
/// * `capture_names` - Names of captures to extract
///
/// # Returns
/// Vector of tuples (capture_name, node) for each match
pub fn execute_query_collect_captures(
    language: Language,
    query_str: &str,
    root_node: Node,
    content: &str,
    capture_names: &[&str],
) -> Result<Vec<(String, Node)>> {
    let mut results = Vec::new();

    execute_query(language, query_str, root_node, content, |query, mat, _| {
        for capture in mat.captures {
            let capture_name = &query.capture_names()[capture.index as usize];
            if capture_names.contains(&capture_name.as_str()) {
                results.push((capture_name.clone(), capture.node));
            }
        }
        Ok(())
    })?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_executor_exists() {
        // Placeholder test - in practice would need real Tree-sitter setup
        assert!(true);
    }
}

