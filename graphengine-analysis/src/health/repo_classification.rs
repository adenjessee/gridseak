//! Repository-type classification (library vs application).
//!
//! Inspects manifest files on disk to determine whether a repo is a library
//! (where exported symbols are the intended API surface) or an application
//! (where exports are internal wiring). The classification influences
//! whether the `exported_symbols` entry point rule fires aggressively.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoType {
    Library,
    Application,
}

/// Detect repo type by examining manifest files relative to `workspace_root`.
///
/// Falls back to `Application` when no library indicators are found.
pub fn classify_repo(workspace_root: &Path) -> RepoType {
    if is_node_library(workspace_root)
        || is_rust_library(workspace_root)
        || is_python_library(workspace_root)
        || is_go_library(workspace_root)
    {
        return RepoType::Library;
    }
    RepoType::Application
}

/// Derive the workspace root from file paths present in the graph.
/// Takes the longest common directory prefix of all file paths.
pub fn infer_workspace_root(file_paths: &[&str]) -> Option<PathBuf> {
    let mut iter = file_paths.iter().filter_map(|p| {
        let path = Path::new(p);
        if path.is_absolute() {
            path.parent().map(|pp| pp.to_path_buf())
        } else {
            None
        }
    });

    let mut common: PathBuf = iter.next()?;

    for dir in iter {
        common = longest_common_prefix(&common, &dir);
        if common.as_os_str().is_empty() {
            return None;
        }
    }

    Some(common)
}

fn longest_common_prefix(a: &Path, b: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for (ca, cb) in a.components().zip(b.components()) {
        if ca == cb {
            result.push(ca);
        } else {
            break;
        }
    }
    result
}

// ── Node.js / TypeScript ──────────────────────────────────────────────

fn is_node_library(root: &Path) -> bool {
    let pkg_path = root.join("package.json");
    let content = match std::fs::read_to_string(&pkg_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let is_private = parsed
        .get("private")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_private {
        return false;
    }

    parsed.get("main").is_some() || parsed.get("exports").is_some()
}

// ── Rust ──────────────────────────────────────────────────────────────

fn is_rust_library(root: &Path) -> bool {
    let cargo_path = root.join("Cargo.toml");
    let content = match std::fs::read_to_string(&cargo_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    content.contains("[lib]") || root.join("src/lib.rs").is_file()
}

// ── Python ────────────────────────────────────────────────────────────

fn is_python_library(root: &Path) -> bool {
    if root.join("setup.py").is_file() || root.join("setup.cfg").is_file() {
        return true;
    }
    let pyproject = root.join("pyproject.toml");
    let content = match std::fs::read_to_string(&pyproject) {
        Ok(c) => c,
        Err(_) => return false,
    };

    content.contains("[project]") || content.contains("[tool.poetry]")
}

// ── Go ────────────────────────────────────────────────────────────────

fn is_go_library(root: &Path) -> bool {
    if !root.join("go.mod").is_file() {
        return false;
    }

    // A Go library typically has no `main` package at the root.
    // Check if `main.go` exists at the root or `cmd/` exists.
    let has_main = root.join("main.go").is_file() || root.join("cmd").is_dir();
    !has_main
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn node_library_with_main_field() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"my-lib","main":"index.js"}"#,
        )
        .unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Library);
    }

    #[test]
    fn node_private_package_is_application() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"my-app","private":true,"main":"index.js"}"#,
        )
        .unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Application);
    }

    #[test]
    fn rust_library_with_lib_section() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"mylib\"\n\n[lib]\nname = \"mylib\"\n",
        )
        .unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Library);
    }

    #[test]
    fn rust_library_with_src_lib_rs() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Library);
    }

    #[test]
    fn python_library_with_setup_py() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("setup.py"), "from setuptools import setup").unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Library);
    }

    #[test]
    fn python_library_with_pyproject_toml() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"mypkg\"\n",
        )
        .unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Library);
    }

    #[test]
    fn go_library_without_main() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module github.com/foo/bar\n").unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Library);
    }

    #[test]
    fn go_application_with_main() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module github.com/foo/bar\n").unwrap();
        fs::write(dir.path().join("main.go"), "package main\n").unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Application);
    }

    #[test]
    fn unknown_repo_defaults_to_application() {
        let dir = tempdir().unwrap();
        assert_eq!(classify_repo(dir.path()), RepoType::Application);
    }

    #[test]
    fn infer_root_from_absolute_paths() {
        let paths = vec!["/repo/src/main.rs", "/repo/src/lib.rs", "/repo/tests/t.rs"];
        let root = infer_workspace_root(&paths).unwrap();
        assert_eq!(root, PathBuf::from("/repo"));
    }

    #[test]
    fn infer_root_returns_none_for_empty() {
        let paths: Vec<&str> = vec![];
        assert!(infer_workspace_root(&paths).is_none());
    }
}
