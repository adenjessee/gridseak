//! Source code utility functions
//!
//! Provides utilities for extracting information from source code text,
//! such as identifiers at specific ranges.

use crate::domain::Range;
use std::collections::HashMap;

/// Extract an identifier at a given range from source code
///
/// This function reads the source code from a file cache and extracts the
/// identifier (variable, function, type name, etc.) at the specified range.
/// It handles character-level extraction and expands to full identifier boundaries.
///
/// # Arguments
/// * `range` - The range where the identifier should be located
/// * `file_cache` - A map from file paths to their source code contents
///
/// # Returns
/// `Some(String)` if an identifier was found, `None` otherwise
pub fn extract_identifier_at_range(
    range: &Range,
    file_cache: &HashMap<String, String>,
) -> Option<String> {
    let content = file_cache.get(&range.file)?;
    let line_idx = range.start_line.saturating_sub(1) as usize;
    let lines: Vec<&str> = content.lines().collect();
    if line_idx >= lines.len() {
        return None;
    }
    if range.end_line != range.start_line {
        return None;
    }

    let line = lines[line_idx];
    if line.is_empty() {
        return None;
    }

    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }

    let mut idx = range.start_char as usize;
    if idx >= chars.len() {
        idx = chars.len().saturating_sub(1);
    }
    while idx < chars.len() && !is_ident_char(chars[idx]) {
        idx += 1;
    }
    if idx >= chars.len() {
        return None;
    }

    let mut start = idx;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }

    let mut end = idx;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }

    if start == end {
        return None;
    }

    Some(chars[start..end].iter().collect())
}

/// Check if a character is a valid identifier character
///
/// Valid identifier characters include ASCII alphanumeric, underscore, and
/// single quote (for lifetime parameters in Rust).
///
/// # Arguments
/// * `c` - The character to check
///
/// # Returns
/// `true` if the character is valid in an identifier, `false` otherwise
pub fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '\''
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_identifier_simple() {
        let mut cache = HashMap::new();
        cache.insert("test.rs".to_string(), "let x = 42;".to_string());
        let range = Range::with_file(1, 4, 1, 5, "test.rs".to_string());
        assert_eq!(
            extract_identifier_at_range(&range, &cache),
            Some("x".to_string())
        );
    }

    #[test]
    fn test_extract_identifier_with_underscore() {
        let mut cache = HashMap::new();
        cache.insert("test.rs".to_string(), "let my_var = 42;".to_string());
        let range = Range::with_file(1, 4, 1, 10, "test.rs".to_string());
        assert_eq!(
            extract_identifier_at_range(&range, &cache),
            Some("my_var".to_string())
        );
    }

    #[test]
    fn test_is_ident_char() {
        assert!(is_ident_char('a'));
        assert!(is_ident_char('A'));
        assert!(is_ident_char('0'));
        assert!(is_ident_char('_'));
        assert!(is_ident_char('\''));
        assert!(!is_ident_char(' '));
        assert!(!is_ident_char('+'));
        assert!(!is_ident_char('-'));
    }
}
