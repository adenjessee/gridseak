//! Rust import parsing utilities
//!
//! Parses Rust import declarations using the syn crate and converts them
//! to our domain ImportSpec types.

use crate::application::ports::{ImportKind, ImportPath, ImportSpec, PathRoot};
use crate::domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range};
use crate::syntax::rust::visibility_mapper::map_visibility;
use crate::syntax::utils::fqn_builder::build_fqn;
use syn::{ItemUse, UseTree};
use tracing::warn;

/// A use binding extracted from a UseTree
#[derive(Debug, Clone)]
pub struct UseBinding {
    pub path: Vec<String>,
    pub alias: Option<String>,
    pub is_glob: bool,
}

/// Collect use bindings from a UseTree
pub fn collect_use_bindings(
    tree: &UseTree,
    stack: &mut Vec<String>,
    bindings: &mut Vec<UseBinding>,
) {
    match tree {
        UseTree::Path(path) => {
            stack.push(path.ident.to_string());
            collect_use_bindings(&path.tree, stack, bindings);
            stack.pop();
        }
        UseTree::Name(name) => {
            stack.push(name.ident.to_string());
            bindings.push(UseBinding {
                path: stack.clone(),
                alias: None,
                is_glob: false,
            });
            stack.pop();
        }
        UseTree::Rename(rename) => {
            stack.push(rename.ident.to_string());
            bindings.push(UseBinding {
                path: stack.clone(),
                alias: Some(rename.rename.to_string()),
                is_glob: false,
            });
            stack.pop();
        }
        UseTree::Glob(_) => {
            bindings.push(UseBinding {
                path: stack.clone(),
                alias: None,
                is_glob: true,
            });
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings(item, stack, bindings);
            }
        }
    }
}

/// Compute the path root from Rust import syntax
pub fn compute_path_root(leading_colon: bool, path: &mut Vec<String>) -> PathRoot {
    if leading_colon {
        return PathRoot::Absolute;
    }

    if path.is_empty() {
        return PathRoot::Unqualified;
    }

    match path.first().map(|s| s.as_str()) {
        Some("crate") => {
            path.remove(0);
            PathRoot::Crate
        }
        Some("self") => {
            path.remove(0);
            PathRoot::SelfPath
        }
        Some("super") => {
            let mut count = 0usize;
            while !path.is_empty() && path.first().map(|s| s == "super").unwrap_or(false) {
                path.remove(0);
                count += 1;
            }
            let capped = count.min(u8::MAX as usize) as u8;
            PathRoot::Super(capped)
        }
        _ => PathRoot::Unqualified,
    }
}

/// Expand an import declaration into ImportSpecs
///
/// Parses a Rust import declaration using syn and creates ImportSpec entries
/// for each binding in the import.
///
/// # Arguments
/// * `node` - The Tree-sitter node containing the import
/// * `content` - The source code content
/// * `file_path` - The file path
/// * `range` - The range of the import declaration
/// * `results` - The SyntaxResults to add import specs to
pub fn expand_import_declaration(
    node: &tree_sitter::Node,
    content: &str,
    file_path: &str,
    range: Range,
    results: &mut crate::application::ports::SyntaxResults,
) {
    let text = match node.utf8_text(content.as_bytes()) {
        Ok(text) => text,
        Err(err) => {
            warn!(
                "Failed to read import declaration text in {}: {}",
                file_path, err
            );
            return;
        }
    };

    let item_use = match syn::parse_str::<ItemUse>(text) {
        Ok(item) => item,
        Err(err) => {
            warn!(
                "Failed to parse import declaration '{}' in {}: {}",
                text.trim(),
                file_path,
                err
            );
            return;
        }
    };

    let visibility = map_visibility(&item_use.vis);
    let import_kind = if visibility.is_public() {
        ImportKind::Reexport
    } else {
        ImportKind::Use
    };

    let mut bindings = Vec::new();
    let mut path_stack = Vec::new();
    collect_use_bindings(&item_use.tree, &mut path_stack, &mut bindings);

    for binding in bindings {
        let mut path_segments = binding.path.clone();
        let root = compute_path_root(item_use.leading_colon.is_some(), &mut path_segments);

        if !binding.is_glob
            && path_segments
                .last()
                .map(|segment| segment == "self")
                .unwrap_or(false)
        {
            path_segments.pop();
        }

        let import_path = ImportPath::new(root, path_segments);
        let spec = ImportSpec {
            range: range.clone(),
            path: import_path,
            alias: binding.alias.clone(),
            visibility: visibility.clone(),
            kind: import_kind.clone(),
            is_glob: binding.is_glob,
            source_file: file_path.to_string(),
        };
        results.add_import_spec(spec);

        // For public re-exports with aliases, create a symbol node so it appears in the graph
        // This allows visualization of aliases like `pub use X::Y as Z`
        if import_kind == ImportKind::Reexport && !binding.is_glob {
            if let Some(alias_name) = binding.alias.as_ref() {
                let alias_fqn = build_fqn(alias_name, file_path, None);
                let alias_range = Range {
                    start_line: range.start_line,
                    start_char: range.start_char,
                    end_line: range.end_line,
                    end_char: range.end_char,
                    file: file_path.to_string(),
                };
                let alias_node = Node::new(
                    NodeKind::Function,
                    alias_fqn,
                    alias_range,
                    Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
                );
                results.add_symbol(alias_node);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_path_root_crate() {
        let mut path = vec!["crate".to_string(), "foo".to_string()];
        let root = compute_path_root(false, &mut path);
        assert!(matches!(root, PathRoot::Crate));
        assert_eq!(path, vec!["foo".to_string()]);
    }

    #[test]
    fn test_compute_path_root_super() {
        let mut path = vec!["super".to_string(), "super".to_string(), "foo".to_string()];
        let root = compute_path_root(false, &mut path);
        assert!(matches!(root, PathRoot::Super(2)));
        assert_eq!(path, vec!["foo".to_string()]);
    }
}
