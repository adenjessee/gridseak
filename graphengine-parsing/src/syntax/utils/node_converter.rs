//! Tree-sitter node conversion utilities
//!
//! Provides utilities for converting Tree-sitter nodes to domain types.

use crate::domain::Range;
use tracing::debug;

/// Convert a Tree-sitter node to our Range type
///
/// Tree-sitter uses 0-based line numbers, but our Range type uses 1-based.
/// This function handles the conversion.
///
/// # Arguments
/// * `node` - The Tree-sitter node to convert
/// * `file_path` - The file path where the node is located
///
/// # Returns
/// A Range representing the node's position in the file
pub fn node_to_range(node: &tree_sitter::Node, file_path: &str) -> Range {
    let range = Range {
        start_line: node.start_position().row as u32 + 1, // Tree-sitter is 0-based, we use 1-based
        start_char: node.start_position().column as u32,
        end_line: node.end_position().row as u32 + 1,
        end_char: node.end_position().column as u32,
        file: file_path.to_string(),
    };
    debug!("Created range with file context: {:?}", range);
    range
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_node_to_range_conversion() {
        // This test would need a real Tree-sitter node
        // For now, just test that the function exists and signature is correct
        // In practice, you'd create a Tree-sitter parser and parse some code
        // to get a real node to test with
    }
}
