//! Apex call-site argument-type inference (TR-A.1 sub-scope).
//!
//! Given an `argument_list` node from the Apex tree-sitter grammar,
//! return an [`ApexTypeRef`] per positional argument so the Apex
//! heuristic resolver can disambiguate overloaded constructors and
//! methods without re-parsing.
//!
//! # Scope (Commit 1, TR-A.1 — deliberately narrow)
//!
//! This inferrer is the **minimum viable** arg-type oracle that
//! unblocks TR-A.1's overloaded-ctor fixture. We recognise:
//!
//! | Argument shape                                    | Result                                    |
//! |---------------------------------------------------|-------------------------------------------|
//! | `'hello'` / `"x"` string literal                  | `Primitive("String")`                     |
//! | integer literal (`42`, `-3`)                      | `Primitive("Integer")`                    |
//! | decimal literal (`1.5`, `0.0`)                    | `Primitive("Decimal")`                    |
//! | `true` / `false`                                  | `Primitive("Boolean")`                    |
//! | `null`                                            | `Primitive("Null")`                       |
//! | `new Foo()` / `new Foo(args)`                     | `UserDefined("Foo")` (or SObject / List / Map / Generic via `parse_type_ref`) |
//! | `new List<T>()` / `new Set<T>()` / `new Map<K,V>()` | `Collection { … }` / `Map { … }`         |
//! | `Account.SObjectType` / `SObjectType.Account`     | `Sobject("Account")`                      |
//! | anything else                                     | `Unresolved { raw }` (wildcard at signature-match time) |
//!
//! Identifier-typed arguments (bare locals, fields, method-call
//! returns) are intentionally NOT inferred here; that requires a
//! local-variable / field-type scope table which lands in TR-A.3
//! (see `docs/workstreams/proof-foundation-gap/PHASE_A_EXECUTION_PLAN.md` §4.1).
//! Implicit-conversion widening (e.g. `String.valueOf(...)`) lands in
//! TR-A.4 (§5.1). Return-type inference across chained method calls
//! is a Phase-B non-goal.
//!
//! Unresolved results act as a **wildcard** at their position in
//! [`signature_matcher`](super::signature_matcher) — they never drop
//! a candidate on unknown input.
//!
//! # Why "Null" as a Primitive variant
//!
//! Apex allows `null` to satisfy any reference type but not a
//! primitive. Representing it as `Primitive("Null")` keeps the
//! variant set uniform; the matcher treats `Null` as a wildcard
//! against any non-primitive parameter. This mirrors the convention
//! used in [`super::class_symbols_extractor::is_primitive`] which
//! already accepts the Apex canonical primitive set.

use tree_sitter::Node;

use crate::domain::apex::class_symbols::ApexTypeRef;
#[cfg(test)]
use crate::domain::apex::class_symbols::CollectionKind;

/// Infer the [`ApexTypeRef`] of each positional argument in an
/// `argument_list` node. Returns an empty vec if `args_node` is not
/// an `argument_list` (defensive; the caller is expected to pass the
/// correct node kind).
pub fn infer_arg_types(args_node: &Node<'_>, source: &[u8]) -> Vec<ApexTypeRef> {
    if args_node.kind() != "argument_list" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        out.push(infer_expression_type(&child, source));
    }
    out
}

/// Infer the type of a single expression node. Exposed for unit
/// testing and potential reuse by TR-A.3's local-var extractor.
pub fn infer_expression_type(node: &Node<'_>, source: &[u8]) -> ApexTypeRef {
    match node.kind() {
        "string_literal" => ApexTypeRef::Primitive {
            name: "String".to_string(),
        },
        "int" => ApexTypeRef::Primitive {
            name: "Integer".to_string(),
        },
        "decimal_floating_point_literal" => ApexTypeRef::Primitive {
            name: "Decimal".to_string(),
        },
        "boolean" => ApexTypeRef::Primitive {
            name: "Boolean".to_string(),
        },
        "null_literal" => ApexTypeRef::Primitive {
            name: "Null".to_string(),
        },
        "object_creation_expression" => infer_object_creation(node, source),
        "field_access" => infer_field_access(node, source),
        "parenthesized_expression" => node
            .named_child(0)
            .map(|inner| infer_expression_type(&inner, source))
            .unwrap_or_else(|| unresolved_raw(node, source)),
        "unary_expression" => {
            // `-3` / `+3` retain the inner literal's type. Tree-sitter
            // models this as `unary_expression` over an `int` /
            // `decimal_floating_point_literal`.
            node.child_by_field_name("operand")
                .or_else(|| node.named_child(0))
                .map(|inner| infer_expression_type(&inner, source))
                .unwrap_or_else(|| unresolved_raw(node, source))
        }
        _ => unresolved_raw(node, source),
    }
}

fn infer_object_creation(node: &Node<'_>, source: &[u8]) -> ApexTypeRef {
    let Some(type_node) = node.child_by_field_name("type") else {
        return unresolved_raw(node, source);
    };
    // Delegate to the shared type parser used by `class_symbols_extractor`
    // so `new List<Account>()` yields `Collection { kind: List, element: Sobject("Account") }`
    // and bare `new Foo()` yields `UserDefined("Foo")` (or `Primitive` /
    // `Sobject` per `classify_simple_name`'s tables).
    super::class_symbols_extractor::parse_type_ref(&type_node, source)
}

/// `Account.SObjectType` and `SObjectType.Account` both describe the
/// SObject type token for `Account`. Tree-sitter models both as a
/// `field_access` with `object` + `field` children.
fn infer_field_access(node: &Node<'_>, source: &[u8]) -> ApexTypeRef {
    let Some(object) = node.child_by_field_name("object") else {
        return unresolved_raw(node, source);
    };
    let Some(field) = node.child_by_field_name("field") else {
        return unresolved_raw(node, source);
    };
    let object_text = utf8(&object, source);
    let field_text = utf8(&field, source);

    // `X.SObjectType` → SObject("X")
    if field_text.eq_ignore_ascii_case("SObjectType") {
        return ApexTypeRef::Sobject {
            api_name: object_text,
        };
    }
    // `SObjectType.X` → SObject("X")
    if object_text.eq_ignore_ascii_case("SObjectType") {
        return ApexTypeRef::Sobject {
            api_name: field_text,
        };
    }
    unresolved_raw(node, source)
}

fn utf8(node: &Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn unresolved_raw(node: &Node<'_>, source: &[u8]) -> ApexTypeRef {
    ApexTypeRef::Unresolved {
        raw: node
            .utf8_text(source)
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(""))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Language, Parser, Tree};

    fn apex_lang() -> Language {
        tree_sitter_sfapex_vendored::apex::language()
    }

    fn parse(src: &str) -> Tree {
        let mut parser = Parser::new();
        parser.set_language(apex_lang()).unwrap();
        parser.parse(src, None).unwrap()
    }

    /// Walks the tree and returns the first `argument_list` node.
    fn find_argument_list<'t>(tree: &'t Tree) -> Node<'t> {
        fn walk<'t>(n: Node<'t>) -> Option<Node<'t>> {
            if n.kind() == "argument_list" {
                return Some(n);
            }
            let mut c = n.walk();
            for child in n.named_children(&mut c) {
                if let Some(hit) = walk(child) {
                    return Some(hit);
                }
            }
            None
        }
        walk(tree.root_node()).expect("no argument_list found in source")
    }

    /// Parses a snippet that exercises `argument_list` by embedding
    /// the call inside a minimal class/method shell.
    fn infer_call(args_src: &str) -> Vec<ApexTypeRef> {
        let src = format!("public class T {{ void m() {{ f({}); }} }}\n", args_src);
        let tree = parse(&src);
        let args = find_argument_list(&tree);
        infer_arg_types(&args, src.as_bytes())
    }

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive {
            name: name.to_string(),
        }
    }

    fn user(name: &str) -> ApexTypeRef {
        ApexTypeRef::UserDefined {
            api_name: name.to_string(),
        }
    }

    fn sobject(name: &str) -> ApexTypeRef {
        ApexTypeRef::Sobject {
            api_name: name.to_string(),
        }
    }

    #[test]
    fn string_literal_infers_string() {
        assert_eq!(infer_call("'hello'"), vec![prim("String")]);
    }

    #[test]
    fn integer_literal_infers_integer() {
        assert_eq!(infer_call("42"), vec![prim("Integer")]);
    }

    #[test]
    fn negative_integer_literal_preserves_type() {
        // `-3` is a unary_expression over int — the inferrer unwraps to Integer.
        let inferred = infer_call("-3");
        assert_eq!(inferred, vec![prim("Integer")]);
    }

    #[test]
    fn decimal_literal_infers_decimal() {
        assert_eq!(infer_call("1.5"), vec![prim("Decimal")]);
    }

    #[test]
    fn boolean_literal_infers_boolean() {
        assert_eq!(infer_call("true"), vec![prim("Boolean")]);
        assert_eq!(infer_call("false"), vec![prim("Boolean")]);
    }

    #[test]
    fn null_literal_infers_null_primitive() {
        assert_eq!(infer_call("null"), vec![prim("Null")]);
    }

    #[test]
    fn mixed_literals_preserve_order() {
        assert_eq!(
            infer_call("'tag', 42"),
            vec![prim("String"), prim("Integer")]
        );
    }

    #[test]
    fn new_user_class_infers_user_defined() {
        // `new Logger()` → UserDefined("Logger")
        assert_eq!(infer_call("new Logger()"), vec![user("Logger")]);
    }

    #[test]
    fn new_list_infers_collection_list() {
        // `Account` classifies as `UserDefined` at the extractor
        // layer — the class_registry reconciles it to SObject at
        // lookup time. Matches the field/param/return-type contract
        // in `class_symbols_extractor::parse_type_ref` so signature
        // matching on `List<Account>` ctor params lines up exactly.
        let inferred = infer_call("new List<Account>()");
        assert_eq!(
            inferred,
            vec![ApexTypeRef::Collection {
                kind: CollectionKind::List,
                element: Box::new(user("Account")),
            }]
        );
    }

    #[test]
    fn new_map_infers_map_with_both_parameters() {
        let inferred = infer_call("new Map<Id, Account>()");
        assert_eq!(
            inferred,
            vec![ApexTypeRef::Map {
                key: Box::new(prim("Id")),
                value: Box::new(user("Account")),
            }]
        );
    }

    #[test]
    fn account_dot_sobjecttype_infers_sobject() {
        // `Account.SObjectType` → Sobject("Account")
        assert_eq!(infer_call("Account.SObjectType"), vec![sobject("Account")]);
    }

    #[test]
    fn sobjecttype_dot_account_infers_sobject() {
        // `SObjectType.Account` → Sobject("Account")
        assert_eq!(infer_call("SObjectType.Account"), vec![sobject("Account")]);
    }

    #[test]
    fn identifier_argument_falls_through_to_unresolved() {
        // `this.someField` / bare `someVar` → Unresolved (TR-A.3 scope).
        let inferred = infer_call("someBatchId");
        assert_eq!(inferred.len(), 1);
        matches!(inferred[0], ApexTypeRef::Unresolved { .. });
    }

    #[test]
    fn method_call_result_falls_through_to_unresolved() {
        let inferred = infer_call("getBar()");
        assert_eq!(inferred.len(), 1);
        matches!(inferred[0], ApexTypeRef::Unresolved { .. });
    }

    #[test]
    fn zero_arg_call_yields_empty_vec() {
        assert!(infer_call("").is_empty());
    }
}
