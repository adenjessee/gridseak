//! Deterministic module-to-module dependency edges from import specifiers.
//!
//! Creates Import edges between file-scoped Module nodes by resolving import
//! specifiers to local files. Handles:
//! - JS/TS relative imports (`./a`, `../x`)
//! - Python relative imports (`.utils`, `..models`)
//! - Python intra-project absolute imports (`from requests.models import Response`)
//! - Go intra-project imports (matching import paths to local source files)

use crate::application::ports::SyntaxResults;
use crate::domain::{Confidence, Edge, EdgeKind, NodeKind, Provenance, ProvenanceSource};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct ModuleDependencyResolver;

impl ModuleDependencyResolver {
    /// Emit Import edges between file-scoped Module nodes from import specifiers.
    pub fn resolve_relative_module_imports(
        syntax: &SyntaxResults,
        language_extensions: &[String],
    ) -> Vec<Edge> {
        let mut edges: Vec<Edge> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        let file_module_index = build_file_module_index(syntax);

        for spec in &syntax.import_specs {
            let module_spec = spec.path.segments.first().map(|s| s.as_str()).unwrap_or("");

            let from_module =
                crate::infrastructure::lsp::utils::symbol_lookup::find_containing_module(
                    &spec.range,
                    syntax,
                );
            let Some(from_module) = from_module else {
                continue;
            };

            let importer_file = Path::new(&spec.source_file);

            let target_module = if is_js_relative_specifier(module_spec) {
                resolve_js_relative(importer_file, module_spec, language_extensions, syntax)
            } else if is_python_relative_specifier(module_spec) {
                resolve_python_relative(importer_file, module_spec, language_extensions, syntax)
            } else {
                resolve_intra_project(module_spec, &file_module_index, language_extensions)
            };

            let Some(to_module) = target_module else {
                continue;
            };

            if from_module == to_module {
                continue;
            }

            let key = format!("{from_module}:{to_module}:Import:module");
            if !seen.insert(key) {
                continue;
            }

            edges.push(Edge::new(
                from_module,
                to_module,
                EdgeKind::Import,
                Provenance::new(ProvenanceSource::Heuristic, Confidence::Medium),
            ));
        }

        edges
    }
}

fn is_js_relative_specifier(spec: &str) -> bool {
    spec.starts_with("./") || spec.starts_with("../")
}

/// Python relative imports start with one or more dots: `.utils`, `..models`, `...`
fn is_python_relative_specifier(spec: &str) -> bool {
    spec.starts_with('.') && !spec.starts_with("./") && !spec.starts_with("../")
}

/// Convert Python relative import to a filesystem-relative path.
/// `.utils` → `./utils`, `..models` → `../models`, `. ` → `.`
fn python_relative_to_path(spec: &str) -> Option<String> {
    let dot_count = spec.chars().take_while(|c| *c == '.').count();
    if dot_count == 0 {
        return None;
    }
    let remainder = &spec[dot_count..];
    let prefix = if dot_count == 1 {
        ".".to_string()
    } else {
        (0..dot_count - 1)
            .map(|_| "..")
            .collect::<Vec<_>>()
            .join("/")
    };

    if remainder.is_empty() {
        Some(prefix)
    } else {
        let path_part = remainder.replace('.', "/");
        Some(format!("{prefix}/{path_part}"))
    }
}

fn resolve_js_relative(
    importer_file: &Path,
    module_spec: &str,
    extensions: &[String],
    syntax: &SyntaxResults,
) -> Option<String> {
    let target_file = resolve_relative_target_file(importer_file, module_spec, extensions)?;
    let target_file_str = target_file.to_string_lossy().to_string();
    find_file_module_node_id(syntax, &target_file_str)
}

fn resolve_python_relative(
    importer_file: &Path,
    module_spec: &str,
    extensions: &[String],
    syntax: &SyntaxResults,
) -> Option<String> {
    let relative_path = python_relative_to_path(module_spec)?;
    let target_file = resolve_relative_target_file(importer_file, &relative_path, extensions)?;
    let target_file_str = target_file.to_string_lossy().to_string();
    find_file_module_node_id(syntax, &target_file_str)
}

/// Try to resolve an absolute module name to a local file by matching against
/// known file paths in the project. Works for:
/// - Python: `requests.models` → `requests/models.py`
/// - Go: `github.com/go-chi/chi/v5` → last segment `v5` or `chi` matches a local dir
fn resolve_intra_project(
    module_spec: &str,
    file_module_index: &HashMap<String, String>,
    _extensions: &[String],
) -> Option<String> {
    if module_spec.is_empty() {
        return None;
    }

    // Python dotted module: `requests.models` → try `requests/models`
    let as_path = module_spec.replace('.', "/");

    // Go module path: `github.com/go-chi/chi/v5` → try the full path and suffixes
    // Strip quotes if present (Go paths come with quotes from tree-sitter)
    let cleaned = as_path.trim_matches('"').trim_matches('\'');

    // Try exact match then progressively shorter suffixes
    for (file_key, module_id) in file_module_index {
        if file_key.contains(cleaned)
            || file_key.ends_with(&format!("/{cleaned}"))
            || file_key.ends_with(&format!("/{cleaned}.py"))
            || file_key.ends_with(&format!("/{cleaned}.go"))
        {
            return Some(module_id.clone());
        }
    }

    // For Go, also try matching the last path component (package name)
    if let Some(last_seg) = cleaned.rsplit('/').next() {
        if last_seg != cleaned {
            for (file_key, module_id) in file_module_index {
                let file_lower = file_key.to_lowercase();
                if file_lower.ends_with(&format!("/{last_seg}"))
                    || file_lower.ends_with(&format!("/{last_seg}.go"))
                {
                    return Some(module_id.clone());
                }
            }
        }
    }

    None
}

/// Build an index of normalized file paths → file-scoped module node IDs.
fn build_file_module_index(syntax: &SyntaxResults) -> HashMap<String, String> {
    let mut index = HashMap::new();
    for node in &syntax.symbols {
        if node.kind == NodeKind::Module {
            let file_path = normalize_path(&node.location.file);
            if !file_path.is_empty() {
                index.insert(file_path, node.id.clone());
            }
        }
    }
    index
}

fn resolve_relative_target_file(
    importer_file: &Path,
    module_specifier: &str,
    extensions: &[String],
) -> Option<PathBuf> {
    let importer_dir = importer_file.parent()?;
    let joined = importer_dir.join(module_specifier);

    if joined.extension().is_some() && joined.is_file() {
        return joined.canonicalize().ok().or(Some(joined));
    }

    for ext in extensions {
        let p = PathBuf::from(format!("{}{}", joined.to_string_lossy(), ext));
        if p.is_file() {
            return p.canonicalize().ok().or(Some(p));
        }
    }

    // Try as directory: JS/TS uses `index.*`, Python uses `__init__.*`
    let dir_entry_names = ["index", "__init__"];
    let check_dir = |dir: &Path| -> Option<PathBuf> {
        if !dir.is_dir() {
            return None;
        }
        for entry_name in &dir_entry_names {
            for ext in extensions {
                let p = dir.join(format!("{entry_name}{ext}"));
                if p.is_file() {
                    return p.canonicalize().ok().or(Some(p));
                }
            }
        }
        None
    };

    if let Some(found) = check_dir(&joined) {
        return Some(found);
    }

    None
}

fn find_file_module_node_id(syntax: &SyntaxResults, file: &str) -> Option<String> {
    let normalized_target = normalize_path(file);

    // Prefer explicit marker set by extractor.
    if let Some(node) = syntax.symbols.iter().find(|n| {
        n.kind == NodeKind::Module
            && normalize_path(&n.location.file) == normalized_target
            && n.properties.get("file_module").and_then(|v| v.as_bool()) == Some(true)
    }) {
        return Some(node.id.clone());
    }

    // Fallback: any module in the same file (deterministic, but less precise).
    syntax
        .symbols
        .iter()
        .find(|n| {
            n.kind == NodeKind::Module && normalize_path(&n.location.file) == normalized_target
        })
        .map(|n| n.id.clone())
}

fn normalize_path(p: &str) -> String {
    // Canonicalize when possible; macOS temp paths may differ only by "/private" prefix.
    let pb = Path::new(p);
    if let Ok(canon) = pb.canonicalize() {
        return strip_private_prefix(&canon.to_string_lossy());
    }
    strip_private_prefix(p)
}

fn strip_private_prefix(p: &str) -> String {
    p.strip_prefix("/private").unwrap_or(p).to_string()
}
