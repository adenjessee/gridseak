//! Cyclomatic and cognitive complexity computation from tree-sitter ASTs.
//!
//! Walks function subtrees counting decision points (cyclomatic) and
//! nesting-weighted structural constructs (cognitive). Runs once per file
//! after symbol extraction, storing results in Function node properties.
//!
//! Reference: GE_ANALYZE_EXTENDED_SPECIFICATION.md Section 3.
//!
//! All language-specific classification is delegated to
//! [`LanguageSpecificExtractor`](crate::syntax::language::LanguageSpecificExtractor).
//! This file contains only the language-agnostic walker logic.

use std::collections::HashMap;

use crate::syntax::language::LanguageSpecificExtractor;

/// Complexity values for a single function.
#[derive(Debug, Clone, Copy, Default)]
pub struct ComplexityResult {
    pub cyclomatic: u32,
    pub cognitive: u32,
}

/// Compute complexity for all function-definition nodes in a file's AST.
///
/// Returns a map keyed by `(start_line_1based, end_line_1based)` so the caller
/// can match results back to extracted `Node` objects by their `Range`.
pub fn compute_file_complexity(
    root: &tree_sitter::Node,
    source: &[u8],
    extractor: &dyn LanguageSpecificExtractor,
) -> HashMap<(u32, u32), ComplexityResult> {
    let mut results = HashMap::new();
    collect_function_complexities(root, source, extractor, &mut results);
    results
}

fn collect_function_complexities(
    node: &tree_sitter::Node,
    source: &[u8],
    extractor: &dyn LanguageSpecificExtractor,
    results: &mut HashMap<(u32, u32), ComplexityResult>,
) {
    if extractor.is_function_definition(node.kind()) {
        let key = (
            node.start_position().row as u32 + 1,
            node.end_position().row as u32 + 1,
        );
        let cyclomatic = compute_cyclomatic(node, source, extractor);
        let cognitive = compute_cognitive(node, source, extractor);
        results.insert(
            key,
            ComplexityResult {
                cyclomatic,
                cognitive,
            },
        );
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_function_complexities(&child, source, extractor, results);
        }
    }
}

// ---------------------------------------------------------------------------
// Cyclomatic complexity (McCabe)
// ---------------------------------------------------------------------------

fn compute_cyclomatic(
    function_node: &tree_sitter::Node,
    source: &[u8],
    extractor: &dyn LanguageSpecificExtractor,
) -> u32 {
    let mut complexity: u32 = 1;
    walk_cyclomatic(function_node, source, extractor, &mut complexity);
    complexity
}

fn walk_cyclomatic(
    node: &tree_sitter::Node,
    source: &[u8],
    extractor: &dyn LanguageSpecificExtractor,
    count: &mut u32,
) {
    let kind = node.kind();

    if extractor.is_cyclomatic_decision_point(kind) {
        *count += 1;
    }

    if extractor.is_logical_operator_node(node, source) {
        *count += 1;
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if !extractor.is_function_definition(child.kind()) {
                walk_cyclomatic(&child, source, extractor, count);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cognitive complexity (Sonar-style)
// ---------------------------------------------------------------------------

fn compute_cognitive(
    function_node: &tree_sitter::Node,
    source: &[u8],
    extractor: &dyn LanguageSpecificExtractor,
) -> u32 {
    let mut total: u32 = 0;
    walk_cognitive(function_node, source, extractor, 0, &mut total);
    total
}

fn walk_cognitive(
    node: &tree_sitter::Node,
    source: &[u8],
    extractor: &dyn LanguageSpecificExtractor,
    nesting: u32,
    total: &mut u32,
) {
    let kind = node.kind();

    if extractor.is_function_definition(kind) && nesting > 0 {
        return;
    }

    if extractor.is_cognitive_structural(kind) {
        let is_else_if = extractor.is_continuation_if(node);

        if is_else_if {
            *total += 1;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    walk_cognitive(&child, source, extractor, nesting, total);
                }
            }
        } else {
            *total += 1 + nesting;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    walk_cognitive(&child, source, extractor, nesting + 1, total);
                }
            }
        }
        return;
    }

    if extractor.is_logical_operator_node(node, source) {
        *total += 1;
    }

    if extractor.is_flow_break(kind) {
        *total += 1;
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_cognitive(&child, source, extractor, nesting, total);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::language::extractors::{
        python::PythonExtractor, rust::RustExtractor, typescript::TypeScriptExtractor,
    };
    use crate::syntax::language::LanguageSpecificExtractor;

    use crate::syntax::language::extractors::ApexExtractor;

    fn parse_and_compute(source: &str, language_name: &str) -> Vec<ComplexityResult> {
        let (language, extractor): (_, Box<dyn LanguageSpecificExtractor>) = match language_name {
            "rust" => (tree_sitter_rust::language(), Box::new(RustExtractor)),
            "typescript" => (
                tree_sitter_typescript::language_typescript(),
                Box::new(TypeScriptExtractor),
            ),
            "javascript" => (
                tree_sitter_javascript::language(),
                Box::new(TypeScriptExtractor),
            ),
            "python" => (tree_sitter_python::language(), Box::new(PythonExtractor)),
            "apex" => (
                tree_sitter_sfapex_vendored::apex::language(),
                Box::new(ApexExtractor),
            ),
            _ => panic!("unsupported language: {language_name}"),
        };

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(language).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let results =
            compute_file_complexity(&tree.root_node(), source.as_bytes(), extractor.as_ref());
        let mut sorted: Vec<_> = results.into_iter().collect();
        sorted.sort_by_key(|(k, _)| *k);
        sorted.into_iter().map(|(_, v)| v).collect()
    }

    // --- Cyclomatic: TypeScript ---

    #[test]
    fn empty_function_has_cyclomatic_one() {
        let results = parse_and_compute("function empty() {}", "typescript");
        assert_eq!(results[0].cyclomatic, 1);
    }

    #[test]
    fn single_if_adds_one_path() {
        let src = "function f() { if (x) { return 1; } return 0; }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn if_else_if_adds_two_paths() {
        let src = "function f() { if (a) { } else if (b) { } else { } }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cyclomatic, 3);
    }

    #[test]
    fn for_loop_adds_one_path() {
        let src = "function f() { for (let i = 0; i < 10; i++) { } }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn logical_operators_each_add_one_path() {
        let src = "function f() { if (a && b || c) { } }";
        let results = parse_and_compute(src, "typescript");
        // 1 (base) + 1 (if) + 1 (&&) + 1 (||) = 4
        assert_eq!(results[0].cyclomatic, 4);
    }

    #[test]
    fn switch_cases_each_add_one_path() {
        let src = r#"function f(x) { switch(x) { case 1: break; case 2: break; case 3: break; } }"#;
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cyclomatic, 4);
    }

    #[test]
    fn catch_adds_one_path() {
        let src = "function f() { try { } catch(e) { } }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cyclomatic, 2);
    }

    // --- Cognitive: TypeScript ---

    #[test]
    fn empty_function_has_zero_cognitive() {
        let results = parse_and_compute("function empty() {}", "typescript");
        assert_eq!(results[0].cognitive, 0);
    }

    #[test]
    fn single_if_has_cognitive_one() {
        let src = "function f() { if (x) { } }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cognitive, 1);
    }

    #[test]
    fn nested_if_gets_nesting_penalty() {
        let src = "function f() { if (a) { if (b) { } } }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cognitive, 3);
    }

    #[test]
    fn else_if_has_no_nesting_penalty() {
        let src = "function f() { if (a) { } else if (b) { } }";
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results[0].cognitive, 2);
    }

    // --- Cyclomatic: Rust ---

    #[test]
    fn rust_empty_function_has_cyclomatic_one() {
        let results = parse_and_compute("fn empty() {}", "rust");
        assert_eq!(results[0].cyclomatic, 1);
    }

    #[test]
    fn rust_if_let_adds_one_path() {
        let src = "fn f() { if let Some(x) = opt { } }";
        let results = parse_and_compute(src, "rust");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn rust_match_arms_add_paths() {
        let src = r#"fn f(x: i32) { match x { 1 => {}, 2 => {}, _ => {} } }"#;
        let results = parse_and_compute(src, "rust");
        assert_eq!(results[0].cyclomatic, 4);
    }

    #[test]
    fn rust_loop_adds_one_path() {
        let src = "fn f() { loop { break; } }";
        let results = parse_and_compute(src, "rust");
        assert_eq!(results[0].cyclomatic, 2);
    }

    // --- Cognitive: Rust ---

    #[test]
    fn rust_nested_match_gets_nesting_penalty() {
        let src = r#"fn f(x: i32) {
            if x > 0 {
                match x {
                    1 => {},
                    _ => {}
                }
            }
        }"#;
        let results = parse_and_compute(src, "rust");
        assert_eq!(results[0].cognitive, 3);
    }

    // --- Python ---

    #[test]
    fn python_elif_is_decision_point() {
        let src = "def f():\n    if a:\n        pass\n    elif b:\n        pass\n    else:\n        pass\n";
        let results = parse_and_compute(src, "python");
        assert_eq!(results[0].cyclomatic, 3);
    }

    #[test]
    fn python_boolean_operators_add_paths() {
        let src = "def f():\n    if a and b or c:\n        pass\n";
        let results = parse_and_compute(src, "python");
        assert_eq!(results[0].cyclomatic, 4);
    }

    // --- Multiple functions in one file ---

    #[test]
    fn multiple_functions_computed_independently() {
        let src = r#"
function simple() { return 1; }
function complex() { if (a) { for (let i = 0; i < 10; i++) { if (b) { } } } }
"#;
        let results = parse_and_compute(src, "typescript");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].cyclomatic, 1);
        assert!(results[1].cyclomatic >= 4);
    }

    // --- Nested function bodies don't pollute outer function ---

    // --- Apex ---
    //
    // Apex's tree-sitter grammar mostly reuses Java-family names but has a few
    // surprises (logical operators are `and_expression` / `or_expression`, not
    // `binary_expression`; switch uses `switch_rule` / `switch_label`, not
    // `when_clause`). These tests pin the real behaviour so the ApexExtractor
    // classifier doesn't silently regress.

    #[test]
    fn apex_empty_method_has_cyclomatic_one() {
        let src = r#"public class C { public void f() {} }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 1);
    }

    #[test]
    fn apex_if_adds_one_path() {
        let src = r#"public class C { public void f(Integer x) { if (x > 0) { } } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn apex_for_loop_adds_one_path() {
        let src = r#"public class C { public void f() { for (Integer i = 0; i < 10; i++) { } } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn apex_enhanced_for_soql_adds_one_path() {
        let src = r#"public class C { public void f() { for (Account a : [SELECT Id FROM Account]) { } } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn apex_logical_and_adds_one_path() {
        let src = r#"public class C { public void f(Boolean a, Boolean b) { if (a && b) { } } }"#;
        let results = parse_and_compute(src, "apex");
        // base + if + && = 3
        assert_eq!(results[0].cyclomatic, 3);
    }

    #[test]
    fn apex_logical_and_outside_if_still_adds_one_path() {
        // Bare `&&` without an `if` should still contribute +1.
        // Pins logical-operator detection independent of control-flow nodes.
        let src = r#"public class C { public Boolean f(Boolean a, Boolean b) { return a && b; } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2, "base + && = 2");
    }

    #[test]
    fn apex_logical_or_outside_if_still_adds_one_path() {
        let src = r#"public class C { public Boolean f(Boolean a, Boolean b) { return a || b; } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2, "base + || = 2");
    }

    #[test]
    fn apex_logical_or_adds_one_path() {
        let src = r#"public class C { public void f(Boolean a, Boolean b) { if (a || b) { } } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 3);
    }

    #[test]
    fn apex_switch_rule_each_adds_one_path() {
        let src = r#"public class C { public void f(Integer x) {
            switch on x {
                when 1 { }
                when 2, 3 { }
                when else { }
            }
        } }"#;
        let results = parse_and_compute(src, "apex");
        // Each `when` is a branch. 1 base + 3 rules = 4.
        assert_eq!(results[0].cyclomatic, 4);
    }

    #[test]
    fn apex_try_catch_adds_one_path() {
        let src = r#"public class C { public void f() { try { } catch (Exception e) { } } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn apex_ternary_adds_one_path() {
        let src = r#"public class C { public Integer f(Integer x) { return x > 0 ? 1 : 0; } }"#;
        let results = parse_and_compute(src, "apex");
        assert_eq!(results[0].cyclomatic, 2);
    }

    #[test]
    fn apex_nested_if_gets_cognitive_nesting_penalty() {
        let src = r#"public class C { public void f(Integer x, Integer y) {
            if (x > 0) { if (y > 0) { } }
        } }"#;
        let results = parse_and_compute(src, "apex");
        // outer if: +1, inner if: +1+1 = 3
        assert_eq!(results[0].cognitive, 3);
    }

    #[test]
    fn apex_trigger_is_not_a_function_definition() {
        // Triggers should not be classified as function definitions — their
        // bodies are inlined into the file-scope, not treated as an independent
        // function with its own complexity container.
        let src = r#"
trigger AccountTrg on Account (before insert) {
    for (Account a : Trigger.new) {
        if (a.Name == null) { a.Name = 'unset'; }
    }
}
"#;
        let results = parse_and_compute(src, "apex");
        // If the trigger body itself were treated as a function, we'd get
        // one ComplexityResult covering the whole body. Instead we expect
        // zero results because there are no method_declaration / constructor
        // nodes.
        assert!(
            results.is_empty(),
            "trigger body should not be classified as a function: {results:?}"
        );
    }

    #[test]
    fn nested_function_complexity_is_separate() {
        let src = r#"
function outer() {
    function inner() { if (x) { if (y) { } } }
    return 1;
}
"#;
        let results = parse_and_compute(src, "typescript");
        let outer = &results[0];
        let inner = &results[1];
        assert_eq!(outer.cyclomatic, 1);
        assert_eq!(inner.cyclomatic, 3);
    }
}
