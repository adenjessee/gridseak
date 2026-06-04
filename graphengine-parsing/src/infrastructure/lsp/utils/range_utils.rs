//! Range utility functions
//!
//! Provides utilities for working with source code ranges, including
//! containment checks and range comparisons.

use crate::domain::Range;

/// Check if a range is within another range
///
/// This checks if the inner range is completely contained within the outer range,
/// including same-file enforcement and character-level precision for same-line ranges.
///
/// # Arguments
/// * `inner` - The range that should be contained
/// * `outer` - The range that should contain the inner range
///
/// # Returns
/// `true` if inner is completely within outer, `false` otherwise
pub fn is_within_range(inner: &Range, outer: &Range) -> bool {
    // Enforce same-file containment first
    if inner.file != outer.file {
        return false;
    }

    // Check if the inner range is completely contained within the outer range
    if inner.start_line < outer.start_line || inner.end_line > outer.end_line {
        return false;
    }

    // If the call site starts on the same line as the function, check character position
    if inner.start_line == outer.start_line && inner.start_char < outer.start_char {
        return false;
    }

    // If the call site ends on the same line as the function, check character position
    if inner.end_line == outer.end_line && inner.end_char > outer.end_char {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_within_range_same_file() {
        let outer = Range::with_file(1, 0, 10, 0, "test.rs".to_string());
        let inner = Range::with_file(5, 0, 7, 0, "test.rs".to_string());
        assert!(is_within_range(&inner, &outer));
    }

    #[test]
    fn test_is_within_range_different_files() {
        let outer = Range::with_file(1, 0, 10, 0, "test.rs".to_string());
        let inner = Range::with_file(5, 0, 7, 0, "other.rs".to_string());
        assert!(!is_within_range(&inner, &outer));
    }

    #[test]
    fn test_is_within_range_same_line_chars() {
        let outer = Range::with_file(5, 10, 5, 50, "test.rs".to_string());
        let inner = Range::with_file(5, 20, 5, 30, "test.rs".to_string());
        assert!(is_within_range(&inner, &outer));
    }

    #[test]
    fn test_is_within_range_outside_start() {
        let outer = Range::with_file(5, 10, 5, 50, "test.rs".to_string());
        let inner = Range::with_file(5, 5, 5, 30, "test.rs".to_string());
        assert!(!is_within_range(&inner, &outer));
    }
}
