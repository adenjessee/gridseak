//! Per-method local-variable scope extraction for Apex (TR-A.3).
//!
//! Walks a parsed Apex tree and emits one
//! [`crate::application::ports::LocalVarScope`] per method / constructor
//! body, populated with:
//!
//! * the method's formal parameters, typed from the parameter's
//!   declared type;
//! * every `local_variable_declaration` inside the body, typed from
//!   the declaration's explicit type (`Foo x = ...`).
//!
//! Re-uses [`class_symbols_extractor::parse_type_ref`] so local / field
//! / parameter types collapse to the same [`ApexTypeRef`] variant space
//! at resolver time. Duplicate local names (block-scoped shadowing) are
//! preserved in source-order; the resolver picks the first match whose
//! `declared_at` precedes the call-site location.
//!
//! # Grammar assumptions
//!
//! tree-sitter-sfapex reuses tree-sitter-java node kinds. In
//! particular:
//!
//! * `method_declaration` and `constructor_declaration` both expose a
//!   `body` field (a `block` / `constructor_body` node).
//! * Inside the body, `local_variable_declaration` carries a `type`
//!   child and one-or-more `variable_declarator` children (each with
//!   a `name`).
//! * `formal_parameters` inside `parameters` hold `formal_parameter`
//!   children with `type` + `name`.
//!
//! Anything that doesn't match this shape degrades to
//! [`ApexTypeRef::Unresolved { raw }`] — the resolver treats
//! `Unresolved` receiver types as "no field-type hint" and falls back
//! to its existing name-based dispatch, so unknown shapes cost
//! nothing beyond the walk time.

use tree_sitter::{Node, Tree};

use crate::application::ports::{LocalVarDecl, LocalVarScope};
use crate::domain::apex::class_symbols::ApexTypeRef;
use crate::syntax::utils::node_converter::node_to_range;

use super::class_symbols_extractor;

/// Walk the tree and emit one [`LocalVarScope`] per
/// `method_declaration` / `constructor_declaration` whose body we can
/// locate. The returned scopes are in source order.
pub fn extract_local_var_scopes(tree: &Tree, source: &[u8], file_path: &str) -> Vec<LocalVarScope> {
    let mut out = Vec::new();
    walk(&tree.root_node(), source, file_path, &mut out);
    out
}

fn walk(node: &Node<'_>, source: &[u8], file_path: &str, out: &mut Vec<LocalVarScope>) {
    match node.kind() {
        "method_declaration" | "constructor_declaration" => {
            if let Some(scope) = extract_scope(node, source, file_path) {
                out.push(scope);
            }
            // Descend: inner classes live inside class bodies, not
            // method bodies, so there's no nested-scope consideration
            // here. Still walk children in case future grammar
            // revisions surface lambda / arrow bodies as their own
            // decls.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                walk(&child, source, file_path, out);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                walk(&child, source, file_path, out);
            }
        }
    }
}

fn extract_scope(method_node: &Node<'_>, source: &[u8], file_path: &str) -> Option<LocalVarScope> {
    let body = method_node.child_by_field_name("body")?;
    let body_range = node_to_range(&body, file_path);

    let mut locals = Vec::new();

    // Formal parameters first — they're visible for the whole method
    // body, so their declared_at is the parameter node itself (before
    // any local declaration in the body can shadow them).
    if let Some(params) = method_node.child_by_field_name("parameters") {
        collect_parameters(&params, source, file_path, &mut locals);
    }

    // Walk the body recursively for local_variable_declaration nodes.
    // Recursion is required because Apex allows locals inside `if`,
    // `for`, `while`, and `try` blocks. We intentionally do not
    // recurse into nested function-like declarations — Apex's only
    // nested-function-shape is inner classes (handled at the class
    // level, never inside a method body) — so ordinary recursion is
    // safe.
    collect_locals_in_body(&body, source, file_path, &mut locals);

    Some(LocalVarScope {
        body: body_range,
        locals,
    })
}

fn collect_parameters(
    params_node: &Node<'_>,
    source: &[u8],
    file_path: &str,
    out: &mut Vec<LocalVarDecl>,
) {
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        if child.kind() != "formal_parameter" {
            continue;
        }
        let ty = child
            .child_by_field_name("type")
            .map(|n| class_symbols_extractor::parse_type_ref(&n, source))
            .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(name) = name_node.utf8_text(source) else {
            continue;
        };
        out.push(LocalVarDecl {
            name: name.to_string(),
            ty,
            declared_at: node_to_range(&child, file_path),
        });
    }
}

fn collect_locals_in_body(
    node: &Node<'_>,
    source: &[u8],
    file_path: &str,
    out: &mut Vec<LocalVarDecl>,
) {
    match node.kind() {
        "local_variable_declaration" => {
            // Shape: `Type a = expr, b = expr2;` — one type, multiple
            // variable_declarators. Mirrors `class_symbols_extractor`'s
            // field-declaration handling.
            let ty = node
                .child_by_field_name("type")
                .map(|n| class_symbols_extractor::parse_type_ref(&n, source))
                .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "variable_declarator" {
                    continue;
                }
                let Some(name_node) = child.child_by_field_name("name") else {
                    continue;
                };
                let Ok(name) = name_node.utf8_text(source) else {
                    continue;
                };
                out.push(LocalVarDecl {
                    name: name.to_string(),
                    ty: ty.clone(),
                    declared_at: node_to_range(&child, file_path),
                });
            }
        }
        // `enhanced_for_statement` declares a loop variable:
        // `for (Foo item : items)` — the loop variable is scoped to
        // the loop body but is visible for the entire method body
        // for TR-A.3's purpose (we don't attempt strict block
        // scoping — Apex local names are rarely reused across
        // blocks and doing so would turn a single-pass walk into a
        // scope-stack machine for marginal accuracy gain).
        "enhanced_for_statement" => {
            // Grammar shape: an enhanced-for exposes `type` + `name`
            // (or `value`) fields for the iteration variable. Fall
            // back to heuristic traversal if the field names differ
            // in the vendored grammar.
            let ty = node
                .child_by_field_name("type")
                .map(|n| class_symbols_extractor::parse_type_ref(&n, source))
                .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("value"));
            if let Some(n) = name_node {
                if let Ok(name) = n.utf8_text(source) {
                    out.push(LocalVarDecl {
                        name: name.to_string(),
                        ty,
                        declared_at: node_to_range(node, file_path),
                    });
                }
            }
            // Recurse into the body so locals inside the loop body
            // are picked up too.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_locals_in_body(&child, source, file_path, out);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_locals_in_body(&child, source, file_path, out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Language, Parser};

    fn apex_lang() -> Language {
        tree_sitter_sfapex_vendored::apex::language()
    }

    fn parse(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser.set_language(apex_lang()).unwrap();
        parser.parse(src, None).unwrap()
    }

    #[test]
    fn extracts_formal_parameters_as_locals() {
        let src = r#"
public class C {
    public void run(Integer count, String label) {
        System.debug(label);
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 1, "one scope per method");
        let names: Vec<&str> = scopes[0].locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["count", "label"]);
        match &scopes[0].locals[0].ty {
            ApexTypeRef::Primitive { name } => assert!(name.eq_ignore_ascii_case("Integer")),
            other => panic!("expected Primitive Integer, got {other:?}"),
        }
    }

    #[test]
    fn extracts_local_variable_declaration_with_type() {
        let src = r#"
public class C {
    public void run() {
        UTIL_Permissions permissionsService = new UTIL_Permissions();
        permissionsService.canUpdate(Account.SObjectType);
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 1);
        let names: Vec<&str> = scopes[0].locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["permissionsService"]);
        match &scopes[0].locals[0].ty {
            ApexTypeRef::UserDefined { api_name } => {
                assert_eq!(api_name, "UTIL_Permissions");
            }
            other => panic!("expected UserDefined UTIL_Permissions, got {other:?}"),
        }
    }

    #[test]
    fn extracts_multiple_declarators_under_one_declaration() {
        let src = r#"
public class C {
    public void run() {
        Integer a = 1, b = 2, c = 3;
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 1);
        let names: Vec<&str> = scopes[0].locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn ctor_scope_captures_parameters_and_body_locals() {
        let src = r#"
public class Logger {
    private String label;
    public Logger(String labelIn) {
        String normalized = labelIn.toLowerCase();
        this.label = normalized;
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "Logger.cls");
        assert_eq!(scopes.len(), 1, "one scope for the ctor");
        let names: Vec<&str> = scopes[0].locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["labelIn", "normalized"]);
    }

    #[test]
    fn nested_block_locals_are_collected() {
        let src = r#"
public class C {
    public void run(Boolean flag) {
        if (flag) {
            Integer inside = 1;
        }
        String also = 'x';
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 1);
        let names: Vec<&str> = scopes[0].locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["flag", "inside", "also"]);
    }

    #[test]
    fn multiple_methods_emit_one_scope_each() {
        let src = r#"
public class C {
    public void first(Integer a) {}
    public void second(String b) {}
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].locals.len(), 1);
        assert_eq!(scopes[0].locals[0].name, "a");
        assert_eq!(scopes[1].locals.len(), 1);
        assert_eq!(scopes[1].locals[0].name, "b");
    }

    #[test]
    fn enhanced_for_declares_iteration_variable() {
        let src = r#"
public class C {
    public void run(List<Account> accs) {
        for (Account a : accs) {
            System.debug(a);
        }
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 1);
        let names: Vec<&str> = scopes[0].locals.iter().map(|l| l.name.as_str()).collect();
        // Parameter `accs` plus loop variable `a` should both be present.
        assert!(
            names.contains(&"accs"),
            "params should include accs: {names:?}"
        );
        assert!(
            names.contains(&"a"),
            "loop variable should be recorded: {names:?}"
        );
    }

    #[test]
    fn body_range_contains_first_local_declaration() {
        let src = r#"
public class C {
    public void run() {
        Integer x = 1;
    }
}
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "C.cls");
        assert_eq!(scopes.len(), 1);
        let body = &scopes[0].body;
        let decl = &scopes[0].locals[0].declared_at;
        assert!(
            (body.start_line, body.start_char) <= (decl.start_line, decl.start_char)
                && (decl.end_line, decl.end_char) <= (body.end_line, body.end_char),
            "local decl {decl:?} should be within body range {body:?}",
        );
    }

    #[test]
    fn interface_and_enum_declarations_produce_no_scopes() {
        let src = r#"
public interface IFoo {
    void doIt(Integer x);
}
public enum Color { RED, GREEN }
"#;
        let tree = parse(src);
        let scopes = extract_local_var_scopes(&tree, src.as_bytes(), "IFoo.cls");
        // Interface methods have no body; enums have no methods.
        assert!(
            scopes.is_empty(),
            "interfaces/enums should not emit local scopes, got {:?}",
            scopes
        );
    }
}
