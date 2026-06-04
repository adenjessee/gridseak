//! Path utility functions
//!
//! Provides utilities for working with file paths, including canonicalization
//! and module path resolution.

use std::fs;
use std::path::Path;

/// Convert a path to its canonical string representation
///
/// # Arguments
/// * `path` - The path to canonicalize
///
/// # Returns
/// The canonical path as a string, or the original path if canonicalization fails
pub fn canonical_string(path: &Path) -> String {
    fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

/// Resolve the path to an external module file
///
/// For Rust modules, this checks for `{module_name}.rs` or `{module_name}/mod.rs`
/// in the parent directory of the source file.
///
/// # Arguments
/// * `file_path` - The path of the file containing the module declaration
/// * `module_name` - The name of the module to resolve
///
/// # Returns
/// `Some(String)` with the resolved module file path if found, `None` otherwise
pub fn resolve_external_module_path(file_path: &str, module_name: &str) -> Option<String> {
    let parent_dir = Path::new(file_path).parent()?;

    let direct_file = parent_dir.join(format!("{}.rs", module_name));
    if direct_file.exists() {
        return Some(canonical_string(&direct_file));
    }

    let mod_rs = parent_dir.join(module_name).join("mod.rs");
    if mod_rs.exists() {
        return Some(canonical_string(&mod_rs));
    }

    None
}

/// Extract crate name from file path
///
/// Walks up the directory tree to find the crate root (directory containing Cargo.toml).
///
/// # Arguments
/// * `file_path` - The path of the file
///
/// # Returns
/// `Some(String)` with the crate name if found, `None` otherwise
pub fn extract_crate_name(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);

    // Walk up the directory tree to find the crate root
    let mut current = path.parent();
    while let Some(dir) = current {
        if let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) {
            // Check if this looks like a crate root (has Cargo.toml)
            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.exists() {
                return Some(dir_name.to_string());
            }
        }
        current = dir.parent();
    }

    None
}

/// Find the Cargo.toml path for a file by walking up the directory tree
///
/// # Arguments
/// * `file_path` - The path of the file
///
/// # Returns
/// `Some(String)` with the canonical Cargo.toml path if found, `None` otherwise
pub fn find_cargo_toml_path(file_path: &str) -> Option<String> {
    let path = Path::new(file_path);
    let mut current = path.parent();
    while let Some(dir) = current {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return Some(canonical_string(&cargo_toml));
        }
        current = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_string() {
        let path = Path::new("test.rs");
        let result = canonical_string(path);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_extract_crate_name() {
        // This test would need a real file system structure
        // For now, just test that the function exists
        let result = extract_crate_name("/some/path/to/crate/src/lib.rs");
        // Result depends on actual file system, so we just check it doesn't panic
        let _ = result;
    }
}
