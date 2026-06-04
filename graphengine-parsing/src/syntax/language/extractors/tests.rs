//! Trait-level regression tests for every `LanguageSpecificExtractor` impl.
//!
//! These tests pin the per-language behaviour that was previously spread
//! across `match self.config.language.as_str()` arms in
//! `complexity_extractor`, `symbol_extractor`, `trait_context_detector`, and
//! `receiver_detector`. Adding a language? Add a section here. Changing a
//! language? A failing test in this file is expected and reviewable.

use crate::syntax::language::extractor::LanguageSpecificExtractor;
use crate::syntax::language::extractors::{
    csharp::CSharpExtractor, go::GoExtractor, java::JavaExtractor, javascript::JavaScriptExtractor,
    python::PythonExtractor, rust::RustExtractor, typescript::TypeScriptExtractor, ApexExtractor,
};

// ---------------------------------------------------------------------------
// language() identity
// ---------------------------------------------------------------------------

#[test]
fn each_extractor_reports_its_canonical_language() {
    assert_eq!(RustExtractor.language(), "rust");
    assert_eq!(TypeScriptExtractor.language(), "typescript");
    assert_eq!(JavaScriptExtractor::default().language(), "javascript");
    assert_eq!(PythonExtractor.language(), "python");
    assert_eq!(GoExtractor.language(), "go");
    assert_eq!(JavaExtractor.language(), "java");
    assert_eq!(CSharpExtractor.language(), "csharp");
    assert_eq!(ApexExtractor.language(), "apex");
}

// ---------------------------------------------------------------------------
// Complexity classification — function definition kinds
// ---------------------------------------------------------------------------

#[test]
fn rust_function_definition_matches_old_match_arm() {
    let e = RustExtractor;
    assert!(e.is_function_definition("function_item"));
    assert!(e.is_function_definition("closure_expression"));
    assert!(!e.is_function_definition("method_declaration"));
    assert!(!e.is_function_definition("struct_item"));
}

#[test]
fn typescript_function_definition_matches_old_match_arm() {
    let e = TypeScriptExtractor;
    for kind in [
        "function_declaration",
        "method_definition",
        "arrow_function",
        "function_expression",
        "generator_function_declaration",
        "generator_function",
    ] {
        assert!(e.is_function_definition(kind), "{kind} should match");
    }
    assert!(!e.is_function_definition("function_item"));
}

#[test]
fn javascript_function_definition_matches_typescript() {
    // JS delegates to TS; the arms were identical pre-refactor.
    let ts = TypeScriptExtractor;
    let js = JavaScriptExtractor::default();
    for kind in [
        "function_declaration",
        "method_definition",
        "arrow_function",
        "function_expression",
        "generator_function_declaration",
        "generator_function",
        "function_item",
    ] {
        assert_eq!(
            js.is_function_definition(kind),
            ts.is_function_definition(kind),
            "js/ts must agree on {kind}"
        );
    }
}

#[test]
fn python_function_definition_is_only_function_definition_kind() {
    let e = PythonExtractor;
    assert!(e.is_function_definition("function_definition"));
    assert!(!e.is_function_definition("function_declaration"));
    assert!(!e.is_function_definition("function_item"));
}

#[test]
fn go_function_definition_matches_old_match_arm() {
    let e = GoExtractor;
    for kind in ["function_declaration", "method_declaration", "func_literal"] {
        assert!(e.is_function_definition(kind), "{kind} should match");
    }
    assert!(!e.is_function_definition("method_definition"));
}

#[test]
fn java_function_definition_matches_old_match_arm() {
    let e = JavaExtractor;
    for kind in [
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
    ] {
        assert!(e.is_function_definition(kind), "{kind} should match");
    }
    assert!(!e.is_function_definition("function_declaration"));
}

#[test]
fn csharp_function_definition_matches_old_match_arm() {
    let e = CSharpExtractor;
    for kind in [
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
        "local_function_statement",
        "anonymous_method_expression",
    ] {
        assert!(e.is_function_definition(kind), "{kind} should match");
    }
}

#[test]
fn apex_function_definition_mirrors_java_family() {
    let e = ApexExtractor;
    for kind in [
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
    ] {
        assert!(e.is_function_definition(kind), "{kind} should match");
    }
    assert!(!e.is_function_definition("function_item"));
}

// ---------------------------------------------------------------------------
// Complexity classification — cyclomatic decision points
// ---------------------------------------------------------------------------

#[test]
fn rust_cyclomatic_decision_points_match_old_match_arm() {
    let e = RustExtractor;
    for kind in [
        "if_expression",
        "for_expression",
        "while_expression",
        "loop_expression",
        "match_arm",
        "if_let_expression",
        "while_let_expression",
        "try_expression",
    ] {
        assert!(e.is_cyclomatic_decision_point(kind), "{kind}");
    }
    assert!(!e.is_cyclomatic_decision_point("if_statement"));
}

#[test]
fn python_cyclomatic_decision_points_include_elif() {
    let e = PythonExtractor;
    for kind in [
        "if_statement",
        "elif_clause",
        "for_statement",
        "while_statement",
        "except_clause",
        "conditional_expression",
    ] {
        assert!(e.is_cyclomatic_decision_point(kind), "{kind}");
    }
}

// ---------------------------------------------------------------------------
// Complexity classification — flow breaks
// ---------------------------------------------------------------------------

#[test]
fn rust_flow_breaks_exclude_return_and_throw() {
    // Rust's flow-break set was break/continue only (no throw/raise).
    let e = RustExtractor;
    assert!(e.is_flow_break("break_expression"));
    assert!(e.is_flow_break("continue_expression"));
    assert!(!e.is_flow_break("return_expression"));
    assert!(!e.is_flow_break("throw_statement"));
}

#[test]
fn python_flow_breaks_include_raise() {
    let e = PythonExtractor;
    assert!(e.is_flow_break("raise_statement"));
    assert!(e.is_flow_break("break_statement"));
    assert!(e.is_flow_break("continue_statement"));
}

#[test]
fn csharp_flow_breaks_include_goto() {
    let e = CSharpExtractor;
    assert!(e.is_flow_break("goto_statement"));
    assert!(e.is_flow_break("throw_statement"));
}

// ---------------------------------------------------------------------------
// Receiver-type defaults — exact pre-refactor equivalence
// ---------------------------------------------------------------------------

#[test]
fn rust_trait_object_patterns_match_old_defaults() {
    let e = RustExtractor;
    assert!(e.is_trait_object_type("&dyn LayoutAlgorithm"));
    assert!(e.is_trait_object_type("Box<dyn LayoutAlgorithm>"));
    assert!(e.is_trait_object_type("Arc<dyn T>"));
    assert!(e.is_trait_object_type("Rc<dyn T>"));
    assert!(e.is_trait_object_type("dyn Trait"));
    assert!(!e.is_trait_object_type("Vec<String>"));
    assert!(!e.is_trait_object_type("&str"));
}

#[test]
fn typescript_trait_object_patterns_match_old_defaults() {
    let e = TypeScriptExtractor;
    assert!(e.is_trait_object_type("interface Foo"));
    assert!(e.is_trait_object_type("(x: number) => string"));
    assert!(!e.is_trait_object_type("string"));
}

#[test]
fn javascript_trait_object_patterns_match_typescript() {
    // Old `typescript | javascript =>` arm shared defaults.
    let ts = TypeScriptExtractor;
    let js = JavaScriptExtractor::default();
    for s in ["interface Foo", "(x: number) => string", "string", "number"] {
        assert_eq!(
            js.is_trait_object_type(s),
            ts.is_trait_object_type(s),
            "js/ts must agree on {s:?}"
        );
    }
}

#[test]
fn python_trait_object_patterns_match_old_defaults() {
    let e = PythonExtractor;
    assert!(e.is_trait_object_type("Protocol"));
    assert!(e.is_trait_object_type("typing.Protocol"));
    assert!(e.is_trait_object_type("ABC"));
    assert!(!e.is_trait_object_type("str"));
}

#[test]
fn java_trait_object_patterns_match_old_defaults() {
    let e = JavaExtractor;
    assert!(e.is_trait_object_type("interface Foo"));
    assert!(e.is_trait_object_type("MyInterface"));
    assert!(!e.is_trait_object_type("String"));
}

#[test]
fn go_trait_object_patterns_match_old_defaults() {
    let e = GoExtractor;
    assert!(e.is_trait_object_type("interface {}"));
    assert!(!e.is_trait_object_type("string"));
}

#[test]
fn csharp_trait_object_patterns_match_old_defaults() {
    let e = CSharpExtractor;
    assert!(e.is_trait_object_type("virtual int Foo"));
    assert!(e.is_trait_object_type("interface IFoo"));
    assert!(!e.is_trait_object_type("int"));
}

#[test]
fn apex_trait_object_patterns_follow_java_style() {
    let e = ApexExtractor;
    assert!(e.is_trait_object_type("interface MyInterface"));
    assert!(e.is_trait_object_type("AccountInterface"));
    assert!(!e.is_trait_object_type("Account"));
}
