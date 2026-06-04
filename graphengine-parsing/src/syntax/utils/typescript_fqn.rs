//! TypeScript FQN (Fully Qualified Name) Builder
//!
//! Generates fully qualified names for TypeScript symbols following the format:
//! `{relative_path_without_extension}::{symbol_name}`
//!
//! For nested symbols (methods within classes):
//! `{relative_path_without_extension}::{class_name}::{method_name}`

use std::path::Path;

/// Build a fully qualified name for a TypeScript symbol
///
/// # Arguments
/// * `name` - The symbol name (class, interface, function, etc.)
/// * `file_path` - The file path where the symbol is located
///
/// # Returns
/// FQN in format: `{relative_path_without_extension}::{symbol_name}`
///
/// # Examples
/// ```
/// use graphengine_parsing::syntax::utils::typescript_fqn::build_typescript_fqn;
///
/// let fqn = build_typescript_fqn("AuthService", "src/auth/auth.service.ts");
/// assert_eq!(fqn, "src/auth/auth.service::AuthService");
/// ```
pub fn build_typescript_fqn(name: &str, file_path: &str) -> String {
    let module_path = extract_module_path(file_path);
    format!("{}::{}", module_path, name)
}

/// Build a fully qualified name for a TypeScript method within a class
///
/// # Arguments
/// * `method_name` - The method name
/// * `class_name` - The containing class name
/// * `file_path` - The file path where the class is located
///
/// # Returns
/// FQN in format: `{relative_path_without_extension}::{class_name}::{method_name}`
///
/// # Examples
/// ```
/// use graphengine_parsing::syntax::utils::typescript_fqn::build_typescript_method_fqn;
///
/// let fqn = build_typescript_method_fqn("login", "AuthService", "src/auth/auth.service.ts");
/// assert_eq!(fqn, "src/auth/auth.service::AuthService::login");
/// ```
pub fn build_typescript_method_fqn(method_name: &str, class_name: &str, file_path: &str) -> String {
    let module_path = extract_module_path(file_path);
    format!("{}::{}::{}", module_path, class_name, method_name)
}

/// Extract the module path from a TypeScript file path
///
/// Removes the file extension and normalizes path separators.
/// If the path contains `src/`, extracts from there.
/// For absolute paths, attempts to extract relative portion.
fn extract_module_path(file_path: &str) -> String {
    // Normalize path separators (Windows → Unix)
    let normalized = file_path.replace('\\', "/");

    // Remove any leading slash for consistency
    let path_str = normalized.trim_start_matches('/');

    // Try to find src/ in the path for absolute paths
    let relative_path = if let Some(src_idx) = path_str.find("src/") {
        &path_str[src_idx..]
    } else if let Some(lib_idx) = path_str.find("lib/") {
        &path_str[lib_idx..]
    } else {
        // Use as-is if no common root found
        path_str
    };

    // Remove extension (.ts, .tsx, .mts, .cts)
    //
    // IMPORTANT: We must keep FQNs unique across files that differ only by extension
    // (e.g. `src/foo.ts` vs `src/foo.tsx`). We disambiguate non-`.ts` extensions by
    // appending a stable suffix without using a dot (so the FQN still satisfies
    // "does not contain .ts/.tsx in the module path" invariants used by tests/templates).
    let path = Path::new(relative_path);
    if let Some(stem) = path.file_stem() {
        let ext_suffix = match path.extension().and_then(|e| e.to_str()) {
            Some("tsx") => Some("__tsx"),
            Some("mts") => Some("__mts"),
            Some("cts") => Some("__cts"),
            // `.ts` is the default and remains unsuffixed.
            _ => None,
        };

        let stem_str = stem.to_string_lossy();
        let stem_str = if let Some(suffix) = ext_suffix {
            format!("{}{}", stem_str, suffix)
        } else {
            stem_str.to_string()
        };
        if let Some(parent) = path.parent() {
            let parent_str = parent.to_string_lossy();
            if parent_str.is_empty() || parent_str == "." {
                stem_str
            } else {
                format!("{}/{}", parent_str, stem_str)
            }
        } else {
            stem_str
        }
    } else {
        relative_path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_module_path_simple() {
        assert_eq!(extract_module_path("src/auth.ts"), "src/auth");
    }

    #[test]
    fn test_extract_module_path_nested() {
        assert_eq!(
            extract_module_path("src/auth/auth.service.ts"),
            "src/auth/auth.service"
        );
    }

    #[test]
    fn test_extract_module_path_tsx() {
        assert_eq!(
            extract_module_path("src/components/Button.tsx"),
            "src/components/Button__tsx"
        );
    }

    #[test]
    fn test_extract_module_path_disambiguates_ts_vs_tsx() {
        assert_ne!(
            extract_module_path("src/s.ts"),
            extract_module_path("src/s.tsx")
        );
    }

    #[test]
    fn test_extract_module_path_windows() {
        assert_eq!(
            extract_module_path("src\\services\\api.ts"),
            "src/services/api"
        );
    }

    #[test]
    fn test_extract_module_path_absolute() {
        assert_eq!(
            extract_module_path("/home/user/project/src/utils.ts"),
            "src/utils"
        );
    }

    #[test]
    fn test_extract_module_path_lib() {
        assert_eq!(extract_module_path("lib/config.ts"), "lib/config");
    }
}
