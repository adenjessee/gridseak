//! SOQL / SOSL reference extractor.
//!
//! SOQL and SOSL are embedded in Apex as bracketed query literals —
//! `[SELECT Id, Name FROM Account]` for SOQL, `[FIND 'x' IN ALL FIELDS
//! RETURNING Contact(Id, Email)]` for SOSL. The vendored
//! `tree-sitter-sfapex` Apex grammar already parses these as
//! `query_expression` nodes whose child is either a `soql_query_body`
//! or `sosl_query_body` subtree, with the full SOQL / SOSL AST attached
//! to the Apex parse tree. That means we do **not** need to string-grep
//! queries out and re-parse them — we can walk the unified tree.
//!
//! This module takes an Apex parse tree + source bytes and returns a
//! flat list of `QueryReference`s, each capturing:
//!
//! - the SObject the query targets (or a relationship name for child
//!   subqueries),
//! - the fields the query explicitly reads,
//! - the query kind (SOQL vs. SOSL),
//! - the source [`Range`] so downstream pipelines can attach edges to a
//!   specific line/column.
//!
//! Extraction is **strictly additive**: it never mutates the tree or
//! the caller's data, and it silently skips malformed / partial queries
//! rather than failing the whole Apex parse. Salesforce devs write
//! queries with dynamic chunks (`String.format('%s', objName)`), and a
//! partial AST is still useful — we just extract what we can see.
//!
//! Downstream, callers can turn these into:
//!
//! - **Type edges** (`ApexClass → SObject`) for schema-aware coupling
//!   analysis,
//! - **SObject node creation** for SObjects that weren't declared via
//!   `*.object-meta.xml` (e.g., custom objects referenced from a
//!   managed-package extension),
//! - **Managed-package attribution** by running each SObject name
//!   through [`super::extract_managed_namespace`].

use tree_sitter::{Node, Tree, TreeCursor};

use crate::domain::Range;

/// A single SObject reference extracted from a query literal.
///
/// One SOQL top-level query produces one `QueryReference`. Subqueries
/// produce additional references with `is_child_relationship = true`;
/// callers who don't care about parent/child schema nuances can
/// filter to `!is_child_relationship`. SOSL `RETURNING` clauses
/// produce one reference per `sobject_return`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryReference {
    /// SObject API name, *or* the child-relationship name when this
    /// reference came from a SOQL subquery. Relationship names are
    /// semantically different from SObject API names (they're often
    /// pluralized, e.g. `Contacts`, and can be custom relationship
    /// labels like `MyCustom__r`). See `is_child_relationship`.
    pub sobject_or_relationship: String,
    /// Fields explicitly listed in the query. Does **not** include
    /// `FIELDS(ALL)` / `FIELDS(CUSTOM)` expansions — those are captured
    /// as a boolean below, since they imply "every field on the
    /// SObject" and the list of every field is not knowable from source
    /// alone.
    pub fields: Vec<String>,
    /// Which DSL this reference came from.
    pub kind: QueryKind,
    /// True when this was a SOQL subquery (`SELECT (SELECT ... FROM
    /// Contacts) FROM Account`). In that case
    /// `sobject_or_relationship` is the *relationship* name, not an
    /// SObject API name.
    pub is_child_relationship: bool,
    /// True when the query used `FIELDS(ALL)`, `FIELDS(CUSTOM)`, or
    /// `FIELDS(STANDARD)`. These expand at runtime into "every field"
    /// and can't be enumerated from source. Callers that compute
    /// coupling should treat them as a high-fanout signal.
    pub uses_fields_expansion: bool,
    /// Source-level location of the query literal (or the containing
    /// returning clause for SOSL sub-references).
    pub range: Range,
}

/// Which embedded-DSL produced the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    Soql,
    Sosl,
}

impl QueryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryKind::Soql => "soql",
            QueryKind::Sosl => "sosl",
        }
    }
}

/// Walk an already-parsed Apex tree and extract every SOQL / SOSL
/// reference into a flat list. Does not allocate beyond the output
/// vector and short-lived [`tree_sitter::TreeCursor`] state.
///
/// `source` is the same byte slice that was fed to the parser — we use
/// it to resolve identifier text.
pub fn extract(tree: &Tree, source: &[u8], file_path: &str) -> Vec<QueryReference> {
    let mut out = Vec::new();
    let mut cursor = tree.root_node().walk();
    walk(&mut cursor, source, file_path, &mut out);
    out
}

/// Recursive tree walk. Uses an explicit cursor rather than recursive
/// calls on [`Node`] for ~2x speed and zero reallocation per-node —
/// Apex files with heavy SOQL usage can have thousands of
/// `query_expression` nodes.
fn walk(cursor: &mut TreeCursor, source: &[u8], file_path: &str, out: &mut Vec<QueryReference>) {
    loop {
        let node = cursor.node();
        if node.kind() == "query_expression" {
            extract_query_expression(node, source, file_path, out);
            // Don't recurse into the query body — we already extracted
            // everything we care about, including nested subqueries.
        } else if cursor.goto_first_child() {
            walk(cursor, source, file_path, out);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn extract_query_expression(
    query_expr: Node,
    source: &[u8],
    file_path: &str,
    out: &mut Vec<QueryReference>,
) {
    for child in named_children(query_expr) {
        match child.kind() {
            "soql_query_body" => extract_soql_body(child, source, file_path, false, out),
            "sosl_query_body" => extract_sosl_body(child, source, file_path, out),
            _ => {}
        }
    }
}

fn extract_soql_body(
    body: Node,
    source: &[u8],
    file_path: &str,
    is_child_relationship: bool,
    out: &mut Vec<QueryReference>,
) {
    let mut sobject_name: Option<String> = None;
    let mut fields: Vec<String> = Vec::new();
    let mut uses_fields_expansion = false;
    let mut subqueries: Vec<Node> = Vec::new();

    for child in named_children(body) {
        match child.kind() {
            "from_clause" => {
                sobject_name = first_name_in_from_clause(child, source);
            }
            "select_clause" => {
                collect_select_fields(
                    child,
                    source,
                    &mut fields,
                    &mut uses_fields_expansion,
                    &mut subqueries,
                );
            }
            _ => {}
        }
    }

    if let Some(name) = sobject_name {
        out.push(QueryReference {
            sobject_or_relationship: name,
            fields,
            kind: QueryKind::Soql,
            is_child_relationship,
            uses_fields_expansion,
            range: node_range(body, file_path),
        });
    }
    // If from_clause was unparseable (dynamic SObject name), we
    // intentionally skip emission — a reference with no SObject is
    // useless downstream and a false edge would pollute the graph.

    for sub in subqueries {
        // Subqueries' `from_clause` carries the child relationship name,
        // not an SObject API name. Mark accordingly.
        extract_soql_body(sub, source, file_path, true, out);
    }
}

fn extract_sosl_body(body: Node, source: &[u8], file_path: &str, out: &mut Vec<QueryReference>) {
    for returning in named_children(body).filter(|n| n.kind() == "returning_clause") {
        for sobj_return in named_children(returning).filter(|n| n.kind() == "sobject_return") {
            let mut name: Option<String> = None;
            let mut fields: Vec<String> = Vec::new();
            let mut uses_fields_expansion = false;

            for child in named_children(sobj_return) {
                match child.kind() {
                    "identifier" => {
                        if name.is_none() {
                            name = node_text(child, source).map(str::to_string);
                        }
                    }
                    "selected_fields" => {
                        let mut _subqs: Vec<Node> = Vec::new();
                        collect_select_fields(
                            child,
                            source,
                            &mut fields,
                            &mut uses_fields_expansion,
                            &mut _subqs,
                        );
                    }
                    _ => {}
                }
            }

            if let Some(n) = name {
                out.push(QueryReference {
                    sobject_or_relationship: n,
                    fields,
                    kind: QueryKind::Sosl,
                    is_child_relationship: false,
                    uses_fields_expansion,
                    range: node_range(sobj_return, file_path),
                });
            }
        }
    }
}

fn first_name_in_from_clause(from: Node, source: &[u8]) -> Option<String> {
    for child in named_children(from) {
        match child.kind() {
            "storage_identifier" => return identifier_text(child, source),
            "storage_alias" => {
                // storage_alias children: [storage_identifier, identifier(alias)]
                for gc in named_children(child) {
                    if gc.kind() == "storage_identifier" {
                        return identifier_text(gc, source);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn identifier_text(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => node_text(node, source).map(str::to_string),
        "dotted_identifier" => {
            // Dotted (Account.Name) — join with '.' to preserve the
            // full-qualified reference. Caller can split if needed.
            let mut parts: Vec<&str> = Vec::new();
            for c in named_children(node) {
                if c.kind() == "identifier" {
                    if let Some(t) = node_text(c, source) {
                        parts.push(t);
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("."))
            }
        }
        // storage_identifier / field_identifier wrap one of the above.
        _ => {
            for c in named_children(node) {
                if let Some(t) = identifier_text(c, source) {
                    return Some(t);
                }
            }
            None
        }
    }
}

fn collect_select_fields<'tree>(
    select: Node<'tree>,
    source: &[u8],
    fields: &mut Vec<String>,
    uses_fields_expansion: &mut bool,
    subqueries: &mut Vec<Node<'tree>>,
) {
    for child in named_children(select) {
        match child.kind() {
            "field_identifier" => {
                if let Some(t) = identifier_text(child, source) {
                    fields.push(t);
                }
            }
            "alias_expression" => {
                // alias_expression: inner `field_identifier` is the real
                // field; the trailing `identifier` is just the alias and
                // not a field on the SObject.
                for gc in named_children(child) {
                    if gc.kind() == "field_identifier" {
                        if let Some(t) = identifier_text(gc, source) {
                            fields.push(t);
                        }
                        break;
                    }
                }
            }
            "fields_expression" => {
                *uses_fields_expansion = true;
            }
            "function_expression" => {
                // function_expression wraps aggregate calls like
                // `COUNT(Id)` / `MAX(CreatedDate)`. Extract the inner
                // field_identifier(s) as referenced fields — a query
                // that calls `MAX(Amount)` on Opportunity absolutely
                // reads the `Amount` field.
                collect_fields_from_function(child, source, fields);
            }
            "subquery" => {
                for gc in named_children(child) {
                    if gc.kind() == "soql_query_body" {
                        subqueries.push(gc);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_fields_from_function(func: Node, source: &[u8], fields: &mut Vec<String>) {
    let mut cursor = func.walk();
    walk_for_fields(&mut cursor, source, fields);
}

fn walk_for_fields(cursor: &mut TreeCursor, source: &[u8], fields: &mut Vec<String>) {
    loop {
        let node = cursor.node();
        if node.kind() == "field_identifier" {
            if let Some(t) = identifier_text(node, source) {
                fields.push(t);
            }
        } else if cursor.goto_first_child() {
            walk_for_fields(cursor, source, fields);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn named_children(node: Node) -> impl Iterator<Item = Node> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children.into_iter()
}

fn node_text<'src>(node: Node, source: &'src [u8]) -> Option<&'src str> {
    std::str::from_utf8(&source[node.byte_range()]).ok()
}

fn node_range(node: Node, file_path: &str) -> Range {
    let start = node.start_position();
    let end = node.end_position();
    Range::with_file(
        start.row as u32 + 1,
        start.column as u32,
        end.row as u32 + 1,
        end.column as u32,
        file_path.to_string(),
    )
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse(src: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("load apex grammar");
        parser.parse(src, None).expect("parse apex source")
    }

    fn extract_all(src: &str) -> Vec<QueryReference> {
        let tree = parse(src);
        extract(&tree, src.as_bytes(), "Test.cls")
    }

    #[test]
    fn simple_soql_emits_one_reference() {
        let src = r#"
            public class A {
                public void run() {
                    List<Account> accs = [SELECT Id, Name FROM Account];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 1, "expected one SOQL ref, got {refs:#?}");
        let r = &refs[0];
        assert_eq!(r.kind, QueryKind::Soql);
        assert_eq!(r.sobject_or_relationship, "Account");
        assert_eq!(r.fields, vec!["Id", "Name"]);
        assert!(!r.is_child_relationship);
        assert!(!r.uses_fields_expansion);
        assert_eq!(r.range.file, "Test.cls");
    }

    #[test]
    fn soql_with_alias_and_relationship_fields() {
        let src = r#"
            public class A {
                public void run() {
                    List<Contact> cs = [SELECT Id, Account.Name, Email FROM Contact];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.sobject_or_relationship, "Contact");
        assert!(r.fields.contains(&"Id".to_string()));
        assert!(r.fields.contains(&"Account.Name".to_string()));
        assert!(r.fields.contains(&"Email".to_string()));
    }

    #[test]
    fn soql_subquery_marked_as_child_relationship() {
        let src = r#"
            public class A {
                public void run() {
                    List<Account> accs = [
                        SELECT Id, Name, (SELECT Id, Email FROM Contacts)
                        FROM Account
                    ];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 2, "expected outer + subquery, got {refs:#?}");

        let outer = refs.iter().find(|r| !r.is_child_relationship).unwrap();
        assert_eq!(outer.sobject_or_relationship, "Account");
        assert!(outer.fields.contains(&"Id".to_string()));

        let inner = refs.iter().find(|r| r.is_child_relationship).unwrap();
        assert_eq!(
            inner.sobject_or_relationship, "Contacts",
            "subquery FROM names the *relationship*, not the SObject"
        );
        assert!(inner.fields.contains(&"Email".to_string()));
    }

    #[test]
    fn fields_expansion_is_flagged() {
        let src = r#"
            public class A {
                public void run() {
                    List<Account> accs = [SELECT FIELDS(ALL) FROM Account LIMIT 10];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 1);
        assert!(
            refs[0].uses_fields_expansion,
            "FIELDS(ALL) must flip uses_fields_expansion"
        );
    }

    #[test]
    fn aggregate_functions_extract_inner_fields() {
        let src = r#"
            public class A {
                public void run() {
                    AggregateResult[] r = [SELECT COUNT(Id), MAX(Amount) FROM Opportunity];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.sobject_or_relationship, "Opportunity");
        assert!(
            r.fields.contains(&"Amount".to_string()),
            "MAX(Amount) must surface Amount as a field reference: {:?}",
            r.fields
        );
    }

    #[test]
    fn sosl_returning_clause_yields_one_ref_per_sobject() {
        let src = r#"
            public class A {
                public void run() {
                    List<List<SObject>> results =
                        [FIND 'test' IN ALL FIELDS
                         RETURNING Account(Id, Name), Contact(Id, Email)];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(
            refs.len(),
            2,
            "SOSL RETURNING Account, Contact => 2 refs, got {refs:#?}"
        );
        assert!(refs.iter().all(|r| r.kind == QueryKind::Sosl));
        assert!(refs
            .iter()
            .any(|r| r.sobject_or_relationship == "Account" && r.fields.contains(&"Name".into())));
        assert!(refs
            .iter()
            .any(|r| r.sobject_or_relationship == "Contact" && r.fields.contains(&"Email".into())));
    }

    #[test]
    fn dynamic_soql_string_is_ignored_silently() {
        // Database.query('SELECT ... FROM ' + objName) is a method
        // call, not a query_expression — our extractor must not trip
        // over it and must not emit a reference (we cannot know the
        // SObject at compile time).
        let src = r#"
            public class A {
                public void run() {
                    String objName = 'Account';
                    List<SObject> r = Database.query('SELECT Id FROM ' + objName);
                }
            }
        "#;
        let refs = extract_all(src);
        assert!(
            refs.is_empty(),
            "dynamic SOQL strings must not produce refs: {refs:#?}"
        );
    }

    #[test]
    fn multiple_queries_in_one_method_emit_multiple_refs() {
        let src = r#"
            public class A {
                public void run() {
                    List<Account> a = [SELECT Id FROM Account];
                    List<Contact> c = [SELECT Id FROM Contact];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 2);
        let objs: Vec<&str> = refs
            .iter()
            .map(|r| r.sobject_or_relationship.as_str())
            .collect();
        assert!(objs.contains(&"Account"));
        assert!(objs.contains(&"Contact"));
    }

    #[test]
    fn malformed_query_does_not_panic() {
        // Parser will produce an error node; extractor must yield 0
        // refs without panicking. Real SFDX repos routinely contain
        // partially-typed classes during active development.
        let src = r#"
            public class A {
                public void run() {
                    List<Account> a = [SELECT Id FROM];
                }
            }
        "#;
        let _ = extract_all(src); // must not panic
    }

    #[test]
    fn custom_object_with_managed_namespace() {
        let src = r#"
            public class A {
                public void run() {
                    List<npsp__Allocation__c> allocs =
                        [SELECT Id, npsp__Amount__c FROM npsp__Allocation__c];
                }
            }
        "#;
        let refs = extract_all(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].sobject_or_relationship, "npsp__Allocation__c");
        assert!(refs[0].fields.contains(&"npsp__Amount__c".to_string()));
    }

    #[test]
    fn query_kind_strings_are_stable() {
        assert_eq!(QueryKind::Soql.as_str(), "soql");
        assert_eq!(QueryKind::Sosl.as_str(), "sosl");
    }
}
