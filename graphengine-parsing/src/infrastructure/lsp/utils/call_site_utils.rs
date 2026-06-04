//! Call site utility functions
//!
//! Provides utilities for parsing and processing call site names,
//! such as extracting function names from call type prefixes.

/// Extract the actual function name from a call site name, stripping call type prefixes
///
/// Call sites may have prefixes indicating the type of call (method_call, constructor_call, etc.).
/// This function strips those prefixes to get the actual function name.
///
/// # Examples
/// ```
/// use graphengine_parsing::infrastructure::lsp::utils::extract_function_name;
///
/// assert_eq!(extract_function_name("method_call:path"), "path");
/// assert_eq!(extract_function_name("constructor_call:PathBuf::new"), "PathBuf::new");
/// assert_eq!(extract_function_name("function_call:foo"), "foo");
/// assert_eq!(extract_function_name("chained_call:bar"), "bar");
/// assert_eq!(extract_function_name("std::path::PathBuf"), "std::path::PathBuf"); // namespace, not prefix
/// ```
///
/// # Arguments
/// * `call_site_name` - The call site name, potentially with a call type prefix
///
/// # Returns
/// The function name without the call type prefix
pub fn extract_function_name(call_site_name: &str) -> String {
    // Split on first colon to separate call type from function name
    if let Some(colon_pos) = call_site_name.find(':') {
        // Check if it's a valid call type prefix (not part of a namespace like "std::path")
        let prefix = &call_site_name[..colon_pos];
        match prefix {
            "method_call" | "constructor_call" | "function_call" | "chained_call" => {
                // Valid call type prefix - return the function name part
                call_site_name[colon_pos + 1..].to_string()
            }
            _ => {
                // Not a call type prefix, probably a namespace separator
                call_site_name.to_string()
            }
        }
    } else {
        // No colon, return as-is
        call_site_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function_name_method_call() {
        assert_eq!(extract_function_name("method_call:path"), "path");
    }

    #[test]
    fn test_extract_function_name_constructor() {
        assert_eq!(
            extract_function_name("constructor_call:PathBuf::new"),
            "PathBuf::new"
        );
    }

    #[test]
    fn test_extract_function_name_function_call() {
        assert_eq!(extract_function_name("function_call:foo"), "foo");
    }

    #[test]
    fn test_extract_function_name_chained() {
        assert_eq!(extract_function_name("chained_call:bar"), "bar");
    }

    #[test]
    fn test_extract_function_name_namespace() {
        assert_eq!(
            extract_function_name("std::path::PathBuf"),
            "std::path::PathBuf"
        );
    }

    #[test]
    fn test_extract_function_name_no_prefix() {
        assert_eq!(extract_function_name("simple_function"), "simple_function");
    }
}
