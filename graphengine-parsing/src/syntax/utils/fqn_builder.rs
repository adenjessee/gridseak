//! Fully Qualified Name (FQN) builder utilities
//!
//! Provides utilities for building fully qualified names for symbols
//! based on file paths and module structure.

use crate::syntax::utils::path_utils::extract_crate_name;

/// Build a fully qualified name for a symbol.
///
/// # Arguments
/// * `name` - The symbol name
/// * `file_path` - The file path where the symbol is located
/// * `workspace_root` - Optional workspace root for computing relative paths
///   when no `src/` directory marker is found (e.g., TypeScript files outside `src/`)
pub fn build_fqn(name: &str, file_path: &str, workspace_root: Option<&str>) -> String {
    build_simple_fqn(name, file_path, workspace_root)
}

/// Build a simple FQN based on file path.
///
/// For Rust: finds Cargo.toml, uses crate name + path segments after `src/`.
/// For other languages: uses path segments after `src/` when found, otherwise
/// computes repo-relative path from `workspace_root` so that files outside
/// `src/` still get unique, meaningful FQNs.
pub fn build_simple_fqn(name: &str, file_path: &str, workspace_root: Option<&str>) -> String {
    let path = std::path::Path::new(file_path);

    let mut fqn_parts = Vec::new();

    // Look for the crate root (directory containing Cargo.toml)
    if let Some(crate_name) = extract_crate_name(file_path) {
        fqn_parts.push(crate_name);
    }

    // Extract module path from directory structure
    let mut found_src = false;
    if let Some(parent) = path.parent() {
        let components: Vec<String> = parent
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .map(|s| s.to_string())
            .collect();

        if let Some(src_index) = components.iter().position(|c| c == "src") {
            found_src = true;
            for component in &components[src_index + 1..] {
                if component != "lib" && component != "main" {
                    let module_name = component.replace('-', "_");
                    fqn_parts.push(module_name);
                }
            }
        }

        // Fallback: no `src/` found. Use workspace-relative path components
        // so files in test dirs, benchmarks, etc. still get unique FQNs.
        if !found_src {
            if let Some(root) = workspace_root {
                let root_path = std::path::Path::new(root);
                if let Ok(rel) = path.parent().unwrap_or(path).strip_prefix(root_path) {
                    for component in rel.components() {
                        if let Some(s) = component.as_os_str().to_str() {
                            fqn_parts.push(s.replace('-', "_"));
                        }
                    }
                }
            }
        }
    }

    // Extract module name from file name
    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
        if file_stem != "lib" && file_stem != "main" {
            let module_name = file_stem.replace('-', "_");
            fqn_parts.push(module_name);
        }
    }

    fqn_parts.push(name.to_string());

    fqn_parts.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_fqn() {
        let fqn = build_simple_fqn("my_function", "/path/to/crate/src/lib.rs", None);
        assert!(fqn.contains("my_function"));
    }

    #[test]
    fn test_build_simple_fqn_with_module() {
        let fqn = build_simple_fqn("test", "/path/to/crate/src/module/file.rs", None);
        assert!(fqn.contains("test"));
        assert!(fqn.contains("module"));
    }

    #[test]
    fn test_fqn_outside_src_with_workspace_root() {
        let fqn = build_simple_fqn(
            "res",
            "/workspace/runtime-tests/deno/hono.test.ts",
            Some("/workspace"),
        );
        assert!(
            fqn.contains("runtime_tests"),
            "FQN should include runtime_tests: {fqn}"
        );
        assert!(fqn.contains("deno"), "FQN should include deno: {fqn}");
        assert!(
            fqn.contains("hono.test"),
            "FQN should include hono.test: {fqn}"
        );
        assert!(fqn.contains("res"), "FQN should include symbol name: {fqn}");
    }

    #[test]
    fn test_fqn_outside_src_without_workspace_root() {
        let fqn = build_simple_fqn("res", "/workspace/runtime-tests/deno/hono.test.ts", None);
        assert!(
            fqn.contains("hono.test"),
            "FQN should still include file stem: {fqn}"
        );
        assert!(fqn.contains("res"), "FQN should include symbol name: {fqn}");
    }
}
