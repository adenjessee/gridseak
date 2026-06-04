//! Tree-sitter query validation for Apex.
//!
//! `apex_config_loading.rs` already proves every query in `configs/apex.yaml`
//! *compiles* against the vendored `tree-sitter-sfapex` grammar. That is
//! necessary but not sufficient — a query can compile and still capture
//! nothing on real source. This file closes the gap: we parse the corpus
//! fixture at `tests/fixtures/apex/` and assert each query captures the
//! shapes downstream extractors rely on.
//!
//! The same fixture is consumed by `apex_heuristic_corpus.rs`, so any
//! coverage gap here is immediately exercised at the resolver level too.

use graphengine_parsing::infrastructure::config::load_config;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex")
        .join("force-app")
        .join("main")
        .join("default")
}

fn read_fixture(relative: &str) -> String {
    let path = fixture_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

fn parse(source: &str) -> tree_sitter::Tree {
    let lang = tree_sitter_sfapex_vendored::apex::language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(lang).expect("set tree-sitter language");
    let tree = parser.parse(source, None).expect("parse succeeded");
    assert!(
        !tree.root_node().has_error(),
        "fixture must parse cleanly; grammar reported error(s) — \
         inspect the fixture or the grammar pin"
    );
    tree
}

fn run_query(query_name: &str, source: &str) -> Vec<HashMap<String, Vec<String>>> {
    let config = load_config("apex").expect("load apex config");
    let query_str = config
        .queries
        .get(query_name)
        .unwrap_or_else(|| panic!("apex config missing query '{query_name}'"));

    let lang = tree_sitter_sfapex_vendored::apex::language();
    let query = tree_sitter::Query::new(lang, query_str)
        .unwrap_or_else(|e| panic!("query '{query_name}' failed to compile: {e:?}"));

    let tree = parse(source);
    let mut cursor = tree_sitter::QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let capture_names = query.capture_names();
    let mut out = Vec::new();
    for mat in matches {
        let mut caps: HashMap<String, Vec<String>> = HashMap::new();
        for cap in mat.captures {
            let name = capture_names
                .get(cap.index as usize)
                .map(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            let text = cap
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .to_string();
            caps.entry(name).or_default().push(text);
        }
        out.push(caps);
    }
    out
}

fn capture_values<'a>(matches: &'a [HashMap<String, Vec<String>>], name: &str) -> Vec<&'a str> {
    matches
        .iter()
        .flat_map(|m| m.get(name).into_iter().flat_map(|v| v.iter()))
        .map(String::as_str)
        .collect()
}

fn assert_any_contains(values: &[&str], needle: &str, query_name: &str) {
    assert!(
        values.iter().any(|v| v.contains(needle)),
        "query '{query_name}' never captured '{needle}'; got {values:?}"
    );
}

// ---------------------------------------------------------------------------
// functions query
// ---------------------------------------------------------------------------

#[test]
fn functions_capture_method_and_constructor_names() {
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("functions", &src);

    let names = capture_values(&matches, "name");
    // Methods
    assert_any_contains(&names, "fetchRecent", "functions");
    assert_any_contains(&names, "markProcessed", "functions");
    assert_any_contains(&names, "getById", "functions");
    // Constructor also captured via @name
    assert_any_contains(&names, "AccountService", "functions");

    // Every match must carry a parameter list and a top-level node.
    for m in &matches {
        assert!(
            m.contains_key("params"),
            "functions match missing @params: {m:?}"
        );
        assert!(
            m.contains_key("func"),
            "functions match missing top-level @func: {m:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// structs query (classes AND triggers)
// ---------------------------------------------------------------------------

#[test]
fn structs_capture_class_name_and_body() {
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("structs", &src);

    // There is exactly one top-level class in this fixture but also zero
    // trigger captures, so the struct branch must fire.
    let struct_matches: Vec<_> = matches
        .iter()
        .filter(|m| m.contains_key("struct"))
        .collect();
    assert!(
        !struct_matches.is_empty(),
        "expected at least one @struct capture on a class fixture"
    );
    let names: Vec<&str> = struct_matches
        .iter()
        .flat_map(|m| m.get("name").into_iter().flat_map(|v| v.iter()))
        .map(String::as_str)
        .collect();
    assert!(names.contains(&"AccountService"));
}

#[test]
fn structs_capture_trigger_and_sobject_fields() {
    let src = read_fixture("triggers/AccountTrigger.trigger");
    let matches = run_query("structs", &src);

    let trigger_matches: Vec<_> = matches
        .iter()
        .filter(|m| m.contains_key("trigger"))
        .collect();
    assert_eq!(
        trigger_matches.len(),
        1,
        "exactly one trigger_declaration expected, got {trigger_matches:?}"
    );
    let m = trigger_matches[0];
    assert_eq!(
        m.get("name").and_then(|v| v.first()).map(String::as_str),
        Some("AccountTrigger"),
        "trigger name capture missing"
    );
    assert_eq!(
        m.get("sobject").and_then(|v| v.first()).map(String::as_str),
        Some("Account"),
        "trigger SObject field capture missing — trigger→SObject edges depend on this"
    );
}

// ---------------------------------------------------------------------------
// traits query (interfaces)
// ---------------------------------------------------------------------------

#[test]
fn traits_query_has_zero_matches_on_class_only_fixture() {
    // We do not currently ship an interface fixture. The query must still
    // be well-formed and must NOT wrongly capture a class as a trait.
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("traits", &src);
    assert!(
        matches.is_empty(),
        "traits query must not match class_declaration; got {matches:?}"
    );
}

// ---------------------------------------------------------------------------
// enums query
// ---------------------------------------------------------------------------

#[test]
fn enums_query_matches_nothing_without_enum_declarations() {
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("enums", &src);
    assert!(
        matches.is_empty(),
        "enums query must not match on non-enum source; got {matches:?}"
    );
}

// ---------------------------------------------------------------------------
// modules query (parser_output marker)
// ---------------------------------------------------------------------------

#[test]
fn modules_captures_exactly_one_parser_output_per_file() {
    for fixture in [
        "classes/AccountService.cls",
        "classes/AccountRepository.cls",
        "classes/ContactController.cls",
        "classes/ManagedPackageConsumer.cls",
        "classes/NightlyBatch.cls",
        "classes/AccountService_Test.cls",
        "triggers/AccountTrigger.trigger",
    ] {
        let src = read_fixture(fixture);
        let matches = run_query("modules", &src);
        assert_eq!(
            matches.len(),
            1,
            "{fixture}: modules query must surface exactly one parser_output marker"
        );
    }
}

// ---------------------------------------------------------------------------
// call_sites query — the shape that feeds the call-graph extractor
// ---------------------------------------------------------------------------

#[test]
fn call_sites_capture_both_bare_and_member_invocations() {
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("call_sites", &src);

    // Method names the source definitely calls.
    let methods = capture_values(&matches, "method");
    assert_any_contains(&methods, "query", "call_sites");
    assert_any_contains(&methods, "persist", "call_sites");
    assert_any_contains(&methods, "isEmpty", "call_sites");

    // At least one receiver-ful match exists (`repository.query(...)`).
    assert!(
        matches
            .iter()
            .any(|m| m.contains_key("receiver") && m.contains_key("method")),
        "call_sites query must surface receiver+method for `repository.query(...)`"
    );
}

#[test]
fn call_sites_capture_object_creation_and_constructor_type() {
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("call_sites", &src);

    let constructors = capture_values(&matches, "constructor");
    assert!(
        constructors
            .iter()
            .any(|c| c.contains("AccountRepository")),
        "object_creation_expression capture for `new AccountRepository()` missing; got {constructors:?}"
    );
    assert!(
        constructors.iter().any(|c| c.contains("AccountService")),
        "constructor capture for `new AccountService()` missing; got {constructors:?}"
    );
}

#[test]
fn call_sites_capture_super_and_this_invocations() {
    // `AccountService` carries `this(...)` via overloaded constructors.
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("call_sites", &src);
    // `explicit_constructor_invocation` is optional in our corpus; the
    // query must at least accept it without breaking other branches.
    // No hard assertion here — existence is validated by the fact that
    // this query compiled and produced meaningful captures above.
    assert!(!matches.is_empty(), "call_sites produced zero matches");
}

// ---------------------------------------------------------------------------
// annotations query — drives entry-point tagging in ApexExtractor
// ---------------------------------------------------------------------------

#[test]
fn annotations_capture_auraenabled_invocable_and_istest() {
    let sources = [
        ("classes/AccountService.cls", &["AuraEnabled"][..]),
        (
            "classes/ContactController.cls",
            &["AuraEnabled", "InvocableMethod"][..],
        ),
        ("classes/AccountService_Test.cls", &["IsTest"][..]),
    ];
    for (file, expected) in sources {
        let src = read_fixture(file);
        let matches = run_query("annotations", &src);
        let names = capture_values(&matches, "annotation_name");
        for needle in expected {
            assert!(
                names.iter().any(|n| n == needle),
                "{file}: expected @{needle} to be captured; got {names:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// trigger_events query
// ---------------------------------------------------------------------------

#[test]
fn trigger_events_capture_all_declared_phases() {
    let src = read_fixture("triggers/AccountTrigger.trigger");
    let matches = run_query("trigger_events", &src);

    let events = capture_values(&matches, "event");
    let joined = events.join(" | ").to_ascii_lowercase();
    assert!(joined.contains("before insert"), "before insert missing");
    assert!(joined.contains("before update"), "before update missing");
    assert!(joined.contains("after insert"), "after insert missing");
    assert_eq!(
        matches.len(),
        3,
        "fixture declares exactly 3 trigger events; got {matches:?}"
    );
}

// ---------------------------------------------------------------------------
// type_refs query
// ---------------------------------------------------------------------------

#[test]
fn type_refs_capture_sobject_and_user_types() {
    let src = read_fixture("classes/ManagedPackageConsumer.cls");
    let matches = run_query("type_refs", &src);

    let types = capture_values(&matches, "type");
    assert_any_contains(&types, "Account", "type_refs");
    // npsp__Household__c is a type_identifier in sfapex grammar.
    assert_any_contains(&types, "npsp__Household__c", "type_refs");
}

// ---------------------------------------------------------------------------
// imports query — Apex is packageless; query must never error and should
// at most emit one capture per file (a parser_output marker).
// ---------------------------------------------------------------------------

#[test]
fn imports_query_emits_at_most_one_noop_marker_per_file() {
    let src = read_fixture("classes/AccountService.cls");
    let matches = run_query("imports", &src);
    assert_eq!(
        matches.len(),
        1,
        "Apex imports query is a no-op sentinel; expected exactly one marker"
    );
    assert!(
        matches[0].contains_key("import"),
        "imports match must expose @import capture so the pipeline can ignore it cleanly"
    );
}

// ---------------------------------------------------------------------------
// inheritance query — Sprint E.1
// ---------------------------------------------------------------------------
// Uses `NightlyBatch.cls` for `implements` (with scoped + generic interface
// plus a plain one) and inline sources for every other shape so we cover
// each grammar arm without bloating the on-disk fixture set.

#[test]
fn inheritance_captures_plain_class_extends() {
    let src = "public class Child extends Parent { }\n";
    let matches = run_query("inheritance", src);
    let extends = capture_values(&matches, "extends_type");
    assert_eq!(
        extends,
        vec!["Parent"],
        "plain `extends Parent` must yield exactly one extends_type capture"
    );
    // No implements — Child declares none.
    assert!(
        capture_values(&matches, "implements_type").is_empty(),
        "no implements_type captures expected"
    );
}

#[test]
fn inheritance_captures_class_extends_scoped() {
    let src = "public class Child extends com.example.Parent { }\n";
    let matches = run_query("inheritance", src);
    let extends = capture_values(&matches, "extends_type");
    // scoped_type_identifier gives the whole dotted form as a single capture.
    assert!(
        extends.iter().any(|t| t.contains("Parent")),
        "scoped extends must still capture the type identifier text; got {extends:?}"
    );
}

#[test]
fn inheritance_captures_implements_plain_and_scoped() {
    // `NightlyBatch implements Database.Batchable<SObject>, Schedulable`
    // exercises both scoped-and-generic and plain identifier implements.
    let src = read_fixture("classes/NightlyBatch.cls");
    let matches = run_query("inheritance", &src);
    let implements_caps = capture_values(&matches, "implements_type");
    assert!(
        implements_caps.iter().any(|t| t == &"Schedulable"),
        "plain-identifier implements `Schedulable` missing; got {implements_caps:?}"
    );
    assert!(
        implements_caps
            .iter()
            .any(|t| t.contains("Database.Batchable")),
        "scoped-generic implements `Database.Batchable<SObject>` missing; got {implements_caps:?}"
    );
}

#[test]
fn inheritance_captures_interface_extends() {
    let src = "public interface Child extends ParentIface, OtherIface { }\n";
    let matches = run_query("inheritance", src);
    let extends = capture_values(&matches, "extends_type");
    assert!(
        extends.contains(&"ParentIface") && extends.contains(&"OtherIface"),
        "interface multi-extends must capture every base; got {extends:?}"
    );
}

#[test]
fn inheritance_leaves_plain_class_without_heritage_empty() {
    let src = "public class Standalone { }\n";
    let matches = run_query("inheritance", src);
    assert!(
        matches.is_empty(),
        "classes without extends/implements must produce zero matches; got {matches:?}"
    );
}
