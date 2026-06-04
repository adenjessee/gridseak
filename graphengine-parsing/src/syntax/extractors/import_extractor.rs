//! Import extraction
//!
//! Extracts imports from source code using Tree-sitter queries
//! and Rust-specific parsing for expansion.

use crate::application::ports::SyntaxResults;
use crate::application::ports::{ImportKind, ImportPath, ImportSpec, ImportVisibility, PathRoot};
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::rust::import_parser::expand_import_declaration;
use crate::syntax::utils::node_converter::node_to_range;
use anyhow::{Context, Result};
use std::sync::Arc;
use tree_sitter::Language;

/// Extracts imports from source code
pub struct ImportExtractor {
    language: Language,
    config: Arc<LanguageConfig>,
}

impl ImportExtractor {
    pub fn new(language: Language, config: Arc<LanguageConfig>) -> Self {
        Self { language, config }
    }

    /// Extract imports from the AST
    pub fn extract(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        if let Some(query_str) = self.config.get_query("imports") {
            let query = tree_sitter::Query::new(self.language, query_str)
                .with_context(|| format!("Invalid imports query: {}", query_str))?;

            let mut cursor = tree_sitter::QueryCursor::new();
            let matches = cursor.matches(&query, *root_node, content.as_bytes());
            let capture_names = query.capture_names();

            for mat in matches {
                // Find the main import statement node (capture name: "import")
                let import_node = mat
                    .captures
                    .iter()
                    .find(|c| {
                        capture_names.get(c.index as usize).map(|n| n.as_str()) == Some("import")
                    })
                    .map(|c| c.node);

                if let Some(import_node) = import_node {
                    let range = node_to_range(&import_node, file_path);
                    results.add_import(range.clone()); // legacy range

                    // Rust: expand nested use trees into structured ImportSpecs.
                    if self.config.language.eq_ignore_ascii_case("rust") {
                        expand_import_declaration(&import_node, content, file_path, range, results);
                        continue;
                    }

                    // JS/TS: build ImportSpec entries for each imported binding.
                    if self.config.language.eq_ignore_ascii_case("typescript")
                        || self.config.language.eq_ignore_ascii_case("javascript")
                    {
                        add_js_ts_import_specs(&mat, capture_names, content, file_path, results)?;
                        continue;
                    }

                    // Python: build ImportSpec from dotted module names.
                    if self.config.language.eq_ignore_ascii_case("python") {
                        add_python_import_specs(&mat, capture_names, content, file_path, results)?;
                        continue;
                    }

                    // Go: build ImportSpec from import path literals.
                    if self.config.language.eq_ignore_ascii_case("go") {
                        add_go_import_specs(&mat, capture_names, content, file_path, results)?;
                        continue;
                    }
                }
            }
        }

        Ok(())
    }
}

fn add_js_ts_import_specs(
    mat: &tree_sitter::QueryMatch,
    capture_names: &[String],
    content: &str,
    file_path: &str,
    results: &mut SyntaxResults,
) -> Result<()> {
    // Capture names expected from configs/typescript.yaml:
    // - imported_name, local_name, default_import, namespace, source
    let mut imported_bindings: Vec<(tree_sitter::Node, String, Option<String>)> = Vec::new();
    let mut module_specifier: Option<String> = None;

    // tree-sitter doesn't auto-apply #eq? predicates; enforce them here.
    // If _require_fn is captured, it MUST be "require" or we skip this match.
    for cap in mat.captures {
        let name = capture_names
            .get(cap.index as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        if name == "_require_fn" {
            let text = cap.node.utf8_text(content.as_bytes()).unwrap_or("");
            if text != "require" {
                return Ok(());
            }
        }
    }

    let source_file = file_path.to_string();
    for cap in mat.captures {
        let name = capture_names
            .get(cap.index as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        match name {
            "imported_name" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .to_string();
                imported_bindings.push((cap.node, text, None));
            }
            "local_name" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .to_string();
                if let Some(last) = imported_bindings.last_mut() {
                    last.2 = Some(text);
                } else {
                    imported_bindings.push((cap.node, text, None));
                }
            }
            "default_import" | "namespace" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .to_string();
                imported_bindings.push((cap.node, text, None));
            }
            "source" => {
                let raw = cap.node.utf8_text(content.as_bytes()).unwrap_or("");
                let cleaned = raw.trim().trim_matches('\'').trim_matches('"').to_string();
                if !cleaned.is_empty() {
                    module_specifier = Some(cleaned);
                }
            }
            _ => {}
        }
    }

    // Even for side-effect imports (e.g. `import "./a";`) we still want a stable module->module
    // dependency edge. For those cases, emit a synthetic binding name.
    if imported_bindings.is_empty() {
        if let Some(spec) = &module_specifier {
            let range = node_to_range(&mat.captures[0].node, file_path);
            results.add_import_spec(ImportSpec {
                range,
                // For JS/TS, we encode module specifier as the first segment so the resolver
                // can emit Module->Module dependency edges deterministically.
                path: ImportPath::new(
                    PathRoot::Unqualified,
                    vec![spec.clone(), "__side_effect__".to_string()],
                ),
                alias: None,
                visibility: ImportVisibility::Private,
                kind: ImportKind::Use,
                is_glob: false,
                source_file: source_file.clone(),
            });
        }
        return Ok(());
    }

    for (node, imported_name, alias) in imported_bindings {
        let imported_name = imported_name.trim().to_string();
        if imported_name.is_empty() {
            continue;
        }
        let range = node_to_range(&node, file_path);
        let alias_clean = alias
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != &imported_name);

        let mut segments: Vec<String> = Vec::new();
        if let Some(spec) = &module_specifier {
            segments.push(spec.clone());
        }
        segments.push(imported_name.clone());

        results.add_import_spec(ImportSpec {
            range,
            // For JS/TS, the first segment is module specifier (e.g. "./util") when present.
            path: ImportPath::new(PathRoot::Unqualified, segments),
            alias: alias_clean,
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: source_file.clone(),
        });
    }

    Ok(())
}

/// Build ImportSpec entries for Python import statements.
///
/// Handles: `import os`, `from os import path`, `from os import path as p`,
/// `from . import foo`, `from os import *`
fn add_python_import_specs(
    mat: &tree_sitter::QueryMatch,
    capture_names: &[String],
    content: &str,
    file_path: &str,
    results: &mut SyntaxResults,
) -> Result<()> {
    let mut module_name: Option<String> = None;
    let mut imported_name: Option<String> = None;
    let mut local_name: Option<String> = None;
    let mut is_glob = false;

    let source_file = file_path.to_string();

    for cap in mat.captures {
        let name = capture_names
            .get(cap.index as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        match name {
            "module" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !text.is_empty() {
                    module_name = Some(text);
                }
            }
            "imported_name" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !text.is_empty() {
                    imported_name = Some(text);
                }
            }
            "local_name" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !text.is_empty() {
                    local_name = Some(text);
                }
            }
            "glob" => {
                is_glob = true;
            }
            _ => {}
        }
    }

    let module = match module_name {
        Some(m) => m,
        None => return Ok(()),
    };

    let range = node_to_range(&mat.captures[0].node, file_path);

    let mut segments = vec![module.clone()];
    if let Some(ref imp) = imported_name {
        segments.push(imp.clone());
    }

    let alias = local_name.filter(|a| imported_name.as_ref() != Some(a));

    results.add_import_spec(ImportSpec {
        range,
        path: ImportPath::new(PathRoot::Unqualified, segments),
        alias,
        visibility: ImportVisibility::Private,
        kind: ImportKind::Use,
        is_glob,
        source_file,
    });

    Ok(())
}

/// Build ImportSpec entries for Go import declarations.
///
/// Handles: `import "fmt"`, `import myfmt "fmt"`, `import . "fmt"`, `import _ "net/http/pprof"`
fn add_go_import_specs(
    mat: &tree_sitter::QueryMatch,
    capture_names: &[String],
    content: &str,
    file_path: &str,
    results: &mut SyntaxResults,
) -> Result<()> {
    let mut import_path: Option<String> = None;
    let mut local_name: Option<String> = None;
    let mut is_glob = false;

    let source_file = file_path.to_string();

    for cap in mat.captures {
        let name = capture_names
            .get(cap.index as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        match name {
            "path" => {
                let raw = cap.node.utf8_text(content.as_bytes()).unwrap_or("");
                let cleaned = raw.trim().trim_matches('"').to_string();
                if !cleaned.is_empty() {
                    import_path = Some(cleaned);
                }
            }
            "local_name" => {
                let text = cap
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if text != "_" && !text.is_empty() {
                    local_name = Some(text);
                }
            }
            "glob" => {
                is_glob = true;
            }
            _ => {}
        }
    }

    let path_str = match import_path {
        Some(p) => p,
        None => return Ok(()),
    };

    let range = node_to_range(&mat.captures[0].node, file_path);

    // Go import path segments: "net/http" → ["net/http"]
    // The last component is the package name: "net/http" → "http"
    let package_name = path_str.rsplit('/').next().unwrap_or(&path_str).to_string();
    let segments = vec![path_str, package_name];

    results.add_import_spec(ImportSpec {
        range,
        path: ImportPath::new(PathRoot::Unqualified, segments),
        alias: local_name,
        visibility: ImportVisibility::Private,
        kind: ImportKind::Use,
        is_glob,
        source_file,
    });

    Ok(())
}
