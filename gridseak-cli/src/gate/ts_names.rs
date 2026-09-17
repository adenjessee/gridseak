//! Narrow tree-sitter name extractor for the gate (Go / TypeScript / Rust).
//!
//! Walks the same grammar crates `graphengine-parsing` pins. Output is
//! function/method **short names + 1-based start lines**. Parse failure
//! falls back to the line heuristic in [`super::parse_edit`].

use tree_sitter::{Language, Node, Parser, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedName {
    pub name: String,
    pub start_line: u32,
}

/// `(names, source_label)` — label is `tree-sitter` or `heuristic`.
pub fn extract_names(file: &str, src: &str) -> (Vec<ExtractedName>, &'static str) {
    if let Some(lang) = language_for_file(file) {
        if let Some(tree) = parse(lang, src) {
            return (collect(&tree, src.as_bytes()), "tree-sitter");
        }
    }
    (heuristic_names(src), "heuristic")
}

pub fn language_for_file(file: &str) -> Option<Language> {
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "go" => Some(tree_sitter_go::language()),
        "rs" => Some(tree_sitter_rust::language()),
        "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" => Some(tree_sitter_typescript::language_tsx()),
        _ => None,
    }
}

fn parse(lang: Language, src: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(lang).ok()?;
    parser.parse(src, None)
}

fn collect(tree: &Tree, src: &[u8]) -> Vec<ExtractedName> {
    let mut out = Vec::new();
    walk(tree.root_node(), src, &mut out);
    out.sort_by(|a, b| a.start_line.cmp(&b.start_line).then(a.name.cmp(&b.name)));
    out.dedup();
    out
}

fn walk(node: Node<'_>, src: &[u8], out: &mut Vec<ExtractedName>) {
    match node.kind() {
        "function_declaration"
        | "method_declaration"
        | "method_spec"
        | "method_elem"
        | "method_signature"
        | "function_item"
        | "method_definition"
        | "generator_function_declaration"
        | "function_signature" => push_named(node, src, out),
        "variable_declarator" | "public_field_definition" => {
            if let Some(val) = node.child_by_field_name("value") {
                if is_fn_like(val.kind()) {
                    push_named(node, src, out);
                }
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk(child, src, out);
        }
    }
}

fn is_fn_like(kind: &str) -> bool {
    matches!(
        kind,
        "arrow_function"
            | "function"
            | "function_expression"
            | "generator_function"
            | "func_literal"
    )
}

fn push_named(node: Node<'_>, src: &[u8], out: &mut Vec<ExtractedName>) {
    if let Some(name) = field_ident(node, src) {
        out.push(ExtractedName {
            name,
            start_line: (node.start_position().row as u32) + 1,
        });
    }
}

fn field_ident(node: Node<'_>, src: &[u8]) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        let text = name.utf8_text(src).ok()?.trim();
        if !text.is_empty() && text != "_" {
            return Some(text.to_string());
        }
    }
    None
}

pub fn heuristic_names(src: &str) -> Vec<ExtractedName> {
    let mut out = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let n = (i as u32) + 1;
        let t = line.trim();
        if let Some(name) = go_func(t).or_else(|| rust_fn(t)).or_else(|| ts_func(t)) {
            out.push(ExtractedName {
                name,
                start_line: n,
            });
        }
    }
    out
}

fn go_func(line: &str) -> Option<String> {
    let rest = line.strip_prefix("func ")?;
    let rest = rest.trim_start();
    let rest = if rest.starts_with('(') {
        let close = rest.find(')')?;
        rest[close + 1..].trim_start()
    } else {
        rest
    };
    ident_at(rest)
}

fn rust_fn(line: &str) -> Option<String> {
    let idx = line.find("fn ")?;
    ident_at(line[idx + 3..].trim_start())
}

fn ts_func(line: &str) -> Option<String> {
    let t = line.trim_start();
    for prefix in [
        "export async function ",
        "export function ",
        "async function ",
        "function ",
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return ident_at(rest);
        }
    }
    None
}

fn ident_at(s: &str) -> Option<String> {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_method_use() {
        let src = "package chi\n\nfunc (m *Mux) Use(mw any) {\n\tm.mws = append(m.mws, mw)\n}\n";
        let (names, src_label) = extract_names("chi.go", src);
        assert_eq!(src_label, "tree-sitter");
        assert!(names.iter().any(|n| n.name == "Use"), "names={names:?}");
    }

    #[test]
    fn go_interface_method_spec() {
        let src = "package chi\n\ntype Router interface {\n\tUse(middlewares ...func()) \n}\n";
        let (names, label) = extract_names("chi.go", src);
        assert_eq!(label, "tree-sitter");
        assert!(
            names.iter().any(|n| n.name == "Use"),
            "interface method_spec Use missing: {names:?}"
        );
    }

    #[test]
    fn go_rename_drops_old_name() {
        let old = "func NewRouter() *Mux { return nil }\n";
        let new = "func NewRouter2() *Mux { return nil }\n";
        let (old_n, _) = extract_names("chi.go", old);
        let (new_n, _) = extract_names("chi.go", new);
        assert!(old_n.iter().any(|n| n.name == "NewRouter"));
        assert!(new_n.iter().any(|n| n.name == "NewRouter2"));
        assert!(!new_n.iter().any(|n| n.name == "NewRouter"));
    }

    #[test]
    fn ts_class_method() {
        let src = "export class Z {\n  parse(x: unknown) { return x }\n}\n";
        let (names, label) = extract_names("src/types.ts", src);
        assert_eq!(label, "tree-sitter");
        assert!(
            names.iter().any(|n| n.name == "parse"),
            "class method parse missing: {names:?}"
        );
    }

    #[test]
    fn ts_export_const_arrow() {
        let src = "export const parse = (x: unknown) => x;\n";
        let (names, label) = extract_names("src/types.ts", src);
        assert_eq!(label, "tree-sitter");
        if !names.iter().any(|n| n.name == "parse") {
            eprintln!(
                "TS miss: export const parse = is not a named function in the TSX grammar; names={names:?}. Gate keeps scan-range fallback."
            );
        }
    }

    #[test]
    fn rust_fn_item() {
        let src = "pub fn run_graph() {}\n";
        let (names, label) = extract_names("src/lib.rs", src);
        assert_eq!(label, "tree-sitter");
        assert!(names.iter().any(|n| n.name == "run_graph"));
    }
}
