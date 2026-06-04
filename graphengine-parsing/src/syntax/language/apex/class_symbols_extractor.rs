//! Extract [`ApexClassSymbols`] payloads from tree-sitter Apex AST nodes.
//!
//! TR-A.0 foundation module. The extractor is a pure tree walk over
//! `class_declaration` / `interface_declaration` / `enum_declaration`
//! nodes — it produces data, never emits graph nodes or edges, and has
//! no side effects. That split is what makes the byte-identical
//! regression gate meaningful: running this extractor during the
//! existing class-declaration pass must not change a single
//! `graphengine_parsing::domain::Node` or `Edge`.
//!
//! # Shape returned
//!
//! Only the outer class's own direct members are populated. Inner
//! classes are referenced by name in `inner_classes` so downstream
//! code knows they exist, but their full shape is produced by
//! **recursive extraction on the inner's own `class_declaration` node**
//! — not by nesting `ApexClassSymbols` inside each other.
//! [`extract_class_declarations`] walks the whole tree and returns a
//! flat `Vec<(String, ApexClassSymbols)>` keyed by dotted `Outer.Inner`
//! api names, which mirrors how the registry stores inner classes.
//!
//! # Grammar assumptions
//!
//! The tree-sitter-sfapex grammar reuses tree-sitter-java node kinds
//! (verified empirically across `graphengine-parsing/src/syntax/language/apex/*`).
//! Specifically:
//!
//! * `class_declaration` exposes `name`, `superclass`, `interfaces`,
//!   `body` children.
//! * `class_body` contains `field_declaration`, `method_declaration`,
//!   `constructor_declaration`, and nested `class_declaration` /
//!   `interface_declaration` / `enum_declaration` children.
//! * `field_declaration` / `method_declaration` /
//!   `constructor_declaration` each start with an optional `modifiers`
//!   child whose children are keyword tokens (`public`, `private`,
//!   `static`, ...).
//! * `method_declaration` exposes `type` (return type) and
//!   `parameters: formal_parameters`. Constructors do not carry
//!   explicit return types.
//! * `formal_parameter` exposes `type` and `name` children.
//!
//! Any unrecognised shape degrades to
//! [`ApexTypeRef::Unresolved { raw }`] rather than dropping the
//! declaration — losing signal silently is the failure mode TR-A.0
//! exists to prevent.

use tree_sitter::Node;

use crate::domain::apex::class_symbols::{
    Access, ApexClassSymbols, ApexConstructor, ApexField, ApexMethod, ApexParameter, ApexTypeRef,
    CollectionKind,
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Walk a `class_declaration` (or `interface_declaration` /
/// `enum_declaration`) node and return its [`ApexClassSymbols`].
///
/// For interfaces and enums the returned shape is trimmed to what the
/// grammar actually exposes:
/// * Interfaces yield method signatures only — fields (constants) are
///   optional in Apex interfaces; we record them when present.
/// * Enums yield neither fields nor methods in the class-symbols
///   sense; they return a default-valued `ApexClassSymbols` so the
///   caller can still attach them to the registry with an empty
///   oracle rather than falling through to the "no oracle" branch.
pub fn extract_class_symbols(node: &Node<'_>, source: &[u8]) -> ApexClassSymbols {
    let mut symbols = ApexClassSymbols::default();
    match node.kind() {
        "class_declaration" => {
            symbols.parent_class = extract_superclass_name(node, source);
            symbols.implemented_interfaces = extract_implemented_interfaces(node, source);
            if let Some(body) = node.child_by_field_name("body") {
                walk_class_body(&body, source, &mut symbols);
            }
        }
        "interface_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                walk_class_body(&body, source, &mut symbols);
            }
        }
        "enum_declaration" => {
            // Enums carry no fields / methods the resolver consumes;
            // returning the default symbol set is intentional.
        }
        _ => {}
    }
    symbols
}

/// Walk every `class_declaration` / `interface_declaration` /
/// `enum_declaration` in the tree rooted at `root_node` and emit one
/// `(dotted_api_name, ApexClassSymbols)` pair per declaration.
///
/// `top_level_name` is the simple name of the outermost class / trigger
/// in the source file (e.g. `"UTIL_JobProgress_CTRL"`) used to compose
/// the dotted path for inner declarations. When the root node is
/// itself a declaration, its entry is emitted under `top_level_name`
/// exactly (not under `top_level_name.top_level_name`).
pub fn extract_class_declarations(
    root_node: &Node<'_>,
    source: &[u8],
    top_level_name: &str,
) -> Vec<(String, ApexClassSymbols)> {
    let mut out = Vec::new();
    walk_declarations(root_node, source, &[top_level_name.to_string()], &mut out);
    out
}

// ---------------------------------------------------------------------------
// Tree walking
// ---------------------------------------------------------------------------

fn walk_declarations(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &[String],
    out: &mut Vec<(String, ApexClassSymbols)>,
) {
    let kind = node.kind();
    let is_decl = matches!(
        kind,
        "class_declaration" | "interface_declaration" | "enum_declaration"
    );

    if is_decl {
        let name = declaration_name(node, source).unwrap_or_default();
        let mut path: Vec<String> = enclosing.to_vec();
        // The root entry: replace the current tail when we're at depth
        // 1 and the node's own name matches the presupplied top-level
        // name. Otherwise push.
        let dotted = if path.len() == 1 && path[0].eq_ignore_ascii_case(&name) {
            path[0].clone()
        } else {
            path.push(name.clone());
            path.join(".")
        };

        let symbols = extract_class_symbols(node, source);
        out.push((dotted.clone(), symbols.clone()));

        // Descend into inner declarations. The symbols we just emitted
        // already list inner class names via `inner_classes`; we
        // recurse here to emit a flat entry per inner so the registry
        // can store them as dotted-path rows.
        let next_enclosing = if path.last().map(|s| s.as_str()) == Some(dotted.as_str()) {
            // path already includes dotted — use as-is
            path
        } else {
            // path was kept with trailing name from the branch above
            path
        };

        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "class_declaration" | "interface_declaration" | "enum_declaration"
                ) {
                    walk_declarations(&child, source, &next_enclosing, out);
                }
            }
        }
        return;
    }

    // Not a declaration node: recurse into children verbatim so we
    // find top-level declarations sitting at the file root.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_declarations(&child, source, enclosing, out);
    }
}

// ---------------------------------------------------------------------------
// Class body members
// ---------------------------------------------------------------------------

fn walk_class_body(body: &Node<'_>, source: &[u8], symbols: &mut ApexClassSymbols) {
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "field_declaration" => {
                extract_fields(&child, source, symbols);
            }
            "method_declaration" => {
                if let Some(method) = extract_method(&child, source) {
                    symbols.methods.push(method);
                }
            }
            "constructor_declaration" => {
                if let Some(ctor) = extract_constructor(&child, source) {
                    symbols.constructors.push(ctor);
                }
            }
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                if let Some(name) = declaration_name(&child, source) {
                    symbols.inner_classes.push(name);
                }
            }
            _ => {}
        }
    }
}

fn extract_fields(field_node: &Node<'_>, source: &[u8], symbols: &mut ApexClassSymbols) {
    let (access, is_static, is_final, _virt, _abs) = extract_modifiers(field_node, source);
    let ty = field_node
        .child_by_field_name("type")
        .map(|n| parse_type_ref(&n, source))
        .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
    // Apex allows multiple variable_declarator children under one
    // field_declaration: `String a, b, c;` declares three fields.
    let mut cursor = field_node.walk();
    for decl in field_node.named_children(&mut cursor) {
        if decl.kind() != "variable_declarator" {
            continue;
        }
        let Some(name_node) = decl.child_by_field_name("name") else {
            continue;
        };
        let Ok(name) = name_node.utf8_text(source) else {
            continue;
        };
        symbols.fields.push(ApexField {
            name: name.to_string(),
            ty: ty.clone(),
            access,
            is_static,
            is_final,
        });
    }
}

fn extract_method(method_node: &Node<'_>, source: &[u8]) -> Option<ApexMethod> {
    let (access, is_static, _is_final, is_virtual, is_abstract) =
        extract_modifiers(method_node, source);
    let name = method_node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(str::to_string)?;
    let parameters = method_node
        .child_by_field_name("parameters")
        .map(|n| extract_parameters(&n, source))
        .unwrap_or_default();
    let return_type = method_node
        .child_by_field_name("type")
        .map(|n| parse_type_ref(&n, source))
        .filter(
            |t| !matches!(t, ApexTypeRef::Primitive { name } if name.eq_ignore_ascii_case("void")),
        );
    Some(ApexMethod {
        name,
        parameters,
        return_type,
        access,
        is_static,
        is_virtual,
        is_abstract,
    })
}

fn extract_constructor(ctor_node: &Node<'_>, source: &[u8]) -> Option<ApexConstructor> {
    let (access, _is_static, _is_final, _is_virtual, _is_abstract) =
        extract_modifiers(ctor_node, source);
    let parameters = ctor_node
        .child_by_field_name("parameters")
        .map(|n| extract_parameters(&n, source))
        .unwrap_or_default();
    Some(ApexConstructor { parameters, access })
}

fn extract_parameters(params_node: &Node<'_>, source: &[u8]) -> Vec<ApexParameter> {
    let mut params = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        if child.kind() != "formal_parameter" {
            continue;
        }
        let ty = child
            .child_by_field_name("type")
            .map(|n| parse_type_ref(&n, source))
            .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
        let name = child
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok())
            .map(str::to_string)
            .unwrap_or_default();
        params.push(ApexParameter { name, ty });
    }
    params
}

// ---------------------------------------------------------------------------
// Modifiers, types, names
// ---------------------------------------------------------------------------

/// Returns `(access, is_static, is_final, is_virtual, is_abstract)`.
/// When no `modifiers` child is present the access defaults to
/// `Access::Private` (Apex's default for class members).
fn extract_modifiers(node: &Node<'_>, source: &[u8]) -> (Access, bool, bool, bool, bool) {
    let mut access = Access::Private;
    let mut is_static = false;
    let mut is_final = false;
    let mut is_virtual = false;
    let mut is_abstract = false;

    let Some(modifiers) = first_named_child(node, "modifiers") else {
        return (access, is_static, is_final, is_virtual, is_abstract);
    };
    let mut cursor = modifiers.walk();
    for child in modifiers.children(&mut cursor) {
        let Ok(text) = child.utf8_text(source) else {
            continue;
        };
        match text {
            "public" => access = Access::Public,
            "protected" => access = Access::Protected,
            "private" => access = Access::Private,
            "global" => access = Access::Global,
            "static" => is_static = true,
            "final" => is_final = true,
            "virtual" | "override" => is_virtual = true,
            "abstract" => is_abstract = true,
            _ => {}
        }
    }
    (access, is_static, is_final, is_virtual, is_abstract)
}

fn first_named_child<'tree>(node: &Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

fn declaration_name(node: &Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(str::to_string)
}

fn extract_superclass_name(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let superclass = first_named_child(node, "superclass")?;
    let mut cursor = superclass.walk();
    for child in superclass.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "type_identifier" | "scoped_type_identifier" | "generic_type"
        ) {
            return Some(render_type_reference_text(&child, source));
        }
    }
    None
}

fn extract_implemented_interfaces(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    let Some(interfaces) = node.child_by_field_name("interfaces") else {
        return Vec::new();
    };
    let Some(list) = first_named_child(&interfaces, "type_list") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = list.walk();
    for child in list.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "type_identifier" | "scoped_type_identifier" | "generic_type"
        ) {
            out.push(render_type_reference_text(&child, source));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Type classification
// ---------------------------------------------------------------------------

/// Shared Apex type-reference parser. Exposed at `pub(super)` so the
/// sibling [`super::arg_type_inferrer`] reuses the same
/// `type_identifier` / `generic_type` / `array_type` classification
/// path used by the class-symbols extractor — keeps
/// `new List<Account>()` argument inference consistent with field /
/// parameter / return-type classification.
pub(super) fn parse_type_ref(node: &Node<'_>, source: &[u8]) -> ApexTypeRef {
    match node.kind() {
        "type_identifier" | "scoped_type_identifier" | "identifier" => {
            let raw = render_type_reference_text(node, source);
            classify_simple_name(&raw)
        }
        "generic_type" => parse_generic_type(node, source),
        "array_type" => {
            // `String[]` → `List<String>`-equivalent for resolver purposes.
            // The grammar calls this `array_type` with a single child
            // `element`. We classify as Collection::List over the element.
            let element = node
                .child_by_field_name("element")
                .or_else(|| node.named_child(0))
                .map(|n| parse_type_ref(&n, source))
                .unwrap_or(ApexTypeRef::Unresolved {
                    raw: render_type_reference_text(node, source),
                });
            ApexTypeRef::Collection {
                kind: CollectionKind::List,
                element: Box::new(element),
            }
        }
        "void_type" => ApexTypeRef::Primitive {
            name: "void".to_string(),
        },
        _ => {
            // Catch-all: preserve the raw source text so downstream
            // code can still attempt a lookup. This is the
            // "degrades gracefully" branch the module doc-comment
            // calls out.
            let raw = render_type_reference_text(node, source);
            classify_simple_name(&raw)
        }
    }
}

fn parse_generic_type(node: &Node<'_>, source: &[u8]) -> ApexTypeRef {
    // generic_type shape: base identifier / scoped_type_identifier
    // followed by a type_arguments list `(<T1, T2, ...>)`.
    let mut base: Option<String> = None;
    let mut parameters: Vec<ApexTypeRef> = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "scoped_type_identifier" => {
                base = Some(render_type_reference_text(&child, source));
            }
            "type_arguments" => {
                let mut inner_cursor = child.walk();
                for arg in child.named_children(&mut inner_cursor) {
                    // Skip punctuation-only tokens (commas etc.) that
                    // sometimes sneak into the named-child set.
                    if arg.kind() == "," {
                        continue;
                    }
                    parameters.push(parse_type_ref(&arg, source));
                }
            }
            _ => {}
        }
    }
    let Some(base) = base else {
        return ApexTypeRef::Unresolved {
            raw: render_type_reference_text(node, source),
        };
    };
    match base.as_str() {
        "List" => ApexTypeRef::Collection {
            kind: CollectionKind::List,
            element: Box::new(
                parameters
                    .into_iter()
                    .next()
                    .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() }),
            ),
        },
        "Set" => ApexTypeRef::Collection {
            kind: CollectionKind::Set,
            element: Box::new(
                parameters
                    .into_iter()
                    .next()
                    .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() }),
            ),
        },
        "Map" => {
            let mut iter = parameters.into_iter();
            let key = iter
                .next()
                .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
            let value = iter
                .next()
                .unwrap_or(ApexTypeRef::Unresolved { raw: String::new() });
            ApexTypeRef::Map {
                key: Box::new(key),
                value: Box::new(value),
            }
        }
        _ => ApexTypeRef::Generic { base, parameters },
    }
}

fn render_type_reference_text(node: &Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(""))
        .unwrap_or_default()
}

/// Classify a bare identifier by name. Match lists are kept in sync
/// with the Apex primitive / system-type tables owned by
/// [`super::class_registry`]. When no static classification applies we
/// degrade to [`ApexTypeRef::UserDefined`] which allows the resolver
/// to consult the registry at lookup time rather than baking its own
/// knowledge here.
fn classify_simple_name(name: &str) -> ApexTypeRef {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return ApexTypeRef::Unresolved { raw: String::new() };
    }
    if is_primitive(trimmed) {
        return ApexTypeRef::Primitive {
            name: trimmed.to_string(),
        };
    }
    ApexTypeRef::UserDefined {
        api_name: trimmed.to_string(),
    }
}

fn is_primitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "boolean"
            | "integer"
            | "long"
            | "double"
            | "decimal"
            | "date"
            | "datetime"
            | "time"
            | "string"
            | "id"
            | "blob"
            | "object"
            | "void"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Language, Parser};

    fn apex_lang() -> Language {
        // Loading the grammar through the same vendored crate used by
        // production parsing keeps this test honest — if the vendored
        // grammar changes, the extractor sees the same tree shape in
        // the test as it would in production.
        tree_sitter_sfapex_vendored::apex::language()
    }

    fn parse(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser.set_language(apex_lang()).unwrap();
        parser.parse(src, None).unwrap()
    }

    fn first_class<'t>(tree: &'t tree_sitter::Tree) -> Node<'t> {
        fn walk<'t>(n: Node<'t>) -> Option<Node<'t>> {
            if n.kind() == "class_declaration" {
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
        walk(tree.root_node()).expect("class_declaration not found")
    }

    #[test]
    fn extracts_fields_with_types_and_access() {
        let src = r#"
public class MyService {
    private Integer counter;
    public String label;
    private final Decimal ratio = 0.5;
    public static List<Account> cached;
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        let names: Vec<_> = symbols.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["counter", "label", "ratio", "cached"]);
        assert_eq!(symbols.fields[0].access, Access::Private);
        assert_eq!(symbols.fields[1].access, Access::Public);
        assert!(symbols.fields[2].is_final);
        assert!(symbols.fields[3].is_static);
        // counter is Integer (primitive).
        assert!(matches!(
            &symbols.fields[0].ty,
            ApexTypeRef::Primitive { name } if name.eq_ignore_ascii_case("Integer")
        ));
        // cached is List<Account>.
        assert!(matches!(
            &symbols.fields[3].ty,
            ApexTypeRef::Collection {
                kind: CollectionKind::List,
                ..
            }
        ));
    }

    #[test]
    fn extracts_methods_with_overloads_and_return_types() {
        let src = r#"
public class fflib_Comparator {
    public static Integer compare(Object a, Object b) { return 0; }
    public static Integer compare(String a, String b) { return 0; }
    public void noop() {}
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        assert_eq!(symbols.methods.len(), 3);
        let compares: Vec<_> = symbols.methods_named("compare").collect();
        assert_eq!(compares.len(), 2);
        // Object overload first, String overload second (declaration order).
        assert_eq!(compares[0].parameters[0].ty.to_api_name(), "Object");
        assert_eq!(compares[1].parameters[0].ty.to_api_name(), "String");
        // noop returns void → None.
        let noop = symbols.methods_named("noop").next().unwrap();
        assert!(noop.return_type.is_none());
    }

    #[test]
    fn extracts_constructors_with_parameters() {
        let src = r#"
public class Logger {
    public Logger() {}
    public Logger(String label) {}
    public Logger(String label, Integer verbosity) {}
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        assert_eq!(symbols.constructors.len(), 3);
        assert_eq!(symbols.constructors[0].parameters.len(), 0);
        assert_eq!(symbols.constructors[1].parameters.len(), 1);
        assert_eq!(symbols.constructors[2].parameters.len(), 2);
    }

    #[test]
    fn extracts_inner_classes_and_inheritance() {
        let src = r#"
public class HouseholdNamingService extends BaseService implements IAuditable, ICacheable {
    public class HouseholdMembers {
        public Id householdId;
    }
    public class Builder {
        public HouseholdMembers build() { return new HouseholdMembers(); }
    }
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        assert_eq!(symbols.parent_class.as_deref(), Some("BaseService"));
        assert_eq!(
            symbols.implemented_interfaces,
            vec!["IAuditable".to_string(), "ICacheable".to_string()]
        );
        assert_eq!(
            symbols.inner_classes,
            vec!["HouseholdMembers".to_string(), "Builder".to_string()]
        );
    }

    #[test]
    fn extract_class_declarations_emits_flat_map_including_inners() {
        let src = r#"
public class Outer {
    public class Inner {
        public Integer payload;
    }
    public class Sibling {
        public String label;
    }
}
"#;
        let tree = parse(src);
        let all = extract_class_declarations(&tree.root_node(), src.as_bytes(), "Outer");
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"Outer"));
        assert!(names.contains(&"Outer.Inner"));
        assert!(names.contains(&"Outer.Sibling"));
        // Each entry carries its own shape.
        let (_, outer_sym) = all.iter().find(|(n, _)| n == "Outer").unwrap();
        assert_eq!(outer_sym.inner_classes.len(), 2);
        let (_, inner_sym) = all.iter().find(|(n, _)| n == "Outer.Inner").unwrap();
        assert_eq!(inner_sym.fields.len(), 1);
        assert_eq!(inner_sym.fields[0].name, "payload");
    }

    #[test]
    fn extracts_void_return_type_as_none_not_primitive_void() {
        let src = r#"
public class S {
    public void run() {}
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        assert_eq!(symbols.methods.len(), 1);
        assert!(symbols.methods[0].return_type.is_none());
    }

    #[test]
    fn field_declarations_with_multiple_declarators_emit_multiple_fields() {
        let src = r#"
public class Multi {
    private Integer a, b, c;
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        let names: Vec<&str> = symbols.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        for f in &symbols.fields {
            assert!(matches!(
                &f.ty,
                ApexTypeRef::Primitive { name } if name.eq_ignore_ascii_case("Integer")
            ));
        }
    }

    #[test]
    fn map_types_are_parsed_as_map_variant() {
        let src = r#"
public class X {
    public Map<Id, Account> cache;
}
"#;
        let tree = parse(src);
        let class = first_class(&tree);
        let symbols = extract_class_symbols(&class, src.as_bytes());
        match &symbols.fields[0].ty {
            ApexTypeRef::Map { key, value } => {
                assert_eq!(key.to_api_name(), "Id");
                assert_eq!(value.to_api_name(), "Account");
            }
            other => panic!("expected Map variant, got {other:?}"),
        }
    }
}
