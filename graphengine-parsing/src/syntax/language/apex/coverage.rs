//! Apex extraction-coverage counter (T8 — universal-fidelity sprint).
//!
//! Walks the tree-sitter-sfapex AST once and emits a
//! [`FileExtractionCoverage`] record naming the regions the Apex
//! extractor does *not* walk. The two D2-validated shapes are R39
//! (property accessor bodies) and R41 (map-literal field
//! initializers); a third variant `ApexTriggerBodyUncaptured` is
//! declared on [`CoverageGap`] for future work.
//!
//! The counter intentionally makes the conservative choice of
//! "any accessor with a block body is an unwalked region" and
//! "any map_initializer is an unwalked region", because the goal
//! of T8 is to stop `dead_code.no_callers` from claiming
//! `High` confidence on files whose AST contains *any* region the
//! extractor query set did not walk. Whether those regions actually
//! contain call sites is outside scope — the classifier will be
//! downgraded by presence alone. If we later want a stricter
//! "only counts regions that actually contain calls" signal, we
//! carry the richer information on new `CoverageGap` variants
//! rather than rewriting this pass.
//!
//! Design doc: `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`.

use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use crate::application::ports::{CoverageConfidence, CoverageGap, FileExtractionCoverage};

/// tree-sitter-sfapex node kind for a property/field accessor
/// declaration (`get { … }` / `set { … }`).
///
/// Verified empirically against `vendor/tree-sitter-sfapex/apex/
/// grammar.js` — the relevant rules are `accessor_list` →
/// `accessor_declaration`. `accessor_declaration`s with a block body
/// (rather than a semicolon) are the R39 gap shape: the Apex
/// extractor's query set does not recurse into them, so any
/// `someMethod(…)` inside the body is invisible to the graph.
const APEX_ACCESSOR_DECLARATION_KIND: &str = "accessor_declaration";

/// tree-sitter-sfapex node kind for a map-literal initializer
/// (`{ k => v, … }` inside `new Map<Id, Foo>{ … }`).
///
/// R41 gap shape: map initializer contents are evaluated expressions,
/// often method calls, but the extractor's query set does not walk
/// them. Any map initializer is an unwalked region.
const APEX_MAP_INITIALIZER_KIND: &str = "map_initializer";

/// Count the R39 / R41 coverage gaps in a parsed Apex file and
/// produce the [`FileExtractionCoverage`] record that
/// [`crate::syntax::language::apex::extractor::ApexExtractor::extract_file_coverage`]
/// appends to `SyntaxResults`.
///
/// The walk is a single depth-first traversal of the AST. Each node
/// is classified:
///   - R39-region root → increment R39 counter, mark subtree as
///     unwalked.
///   - R41-region root → increment R41 counter, mark subtree as
///     unwalked.
///   - otherwise → counted as walked if outside any unwalked region,
///     else as unwalked.
///
/// Confidence is `Low` when the tree contains any tree-sitter parse
/// error (the counts are then approximate); `High` otherwise.
pub fn extract_apex_file_coverage(
    tree: &Tree,
    _source: &[u8],
    file_path: &str,
) -> FileExtractionCoverage {
    let mut counts = CoverageCounts::default();
    let root = tree.root_node();
    visit(root, /* inside_unwalked_region = */ false, &mut counts);

    let mut coverage_gaps = Vec::new();
    if counts.accessor_regions > 0 {
        coverage_gaps.push(CoverageGap::ApexPropertyAccessor {
            count: counts.accessor_regions,
        });
    }
    if counts.map_initializer_regions > 0 {
        coverage_gaps.push(CoverageGap::ApexMapLiteralInitializer {
            count: counts.map_initializer_regions,
        });
    }

    let confidence = if tree.root_node().has_error() {
        CoverageConfidence::Low
    } else {
        CoverageConfidence::High
    };

    FileExtractionCoverage {
        file_path: PathBuf::from(file_path),
        language: "apex".to_string(),
        walked_node_count: counts.walked_nodes,
        unwalked_node_count: counts.unwalked_nodes,
        coverage_gaps,
        confidence,
    }
}

#[derive(Default)]
struct CoverageCounts {
    walked_nodes: u32,
    unwalked_nodes: u32,
    accessor_regions: u32,
    map_initializer_regions: u32,
}

/// Depth-first traversal. An AST node counts as R39 when it is
/// `accessor_declaration` *and* carries a `body` field (i.e., the
/// `get { … }` / `set { … }` form rather than the `get;` / `set;`
/// auto-property form, which has no body to miss). An R41 is any
/// `map_initializer` regardless of parent context.
fn visit<'a>(node: Node<'a>, inside_unwalked_region: bool, counts: &mut CoverageCounts) {
    let kind = node.kind();
    let is_accessor_region =
        kind == APEX_ACCESSOR_DECLARATION_KIND && node.child_by_field_name("body").is_some();
    let is_map_initializer_region = kind == APEX_MAP_INITIALIZER_KIND;
    let starts_new_region =
        !inside_unwalked_region && (is_accessor_region || is_map_initializer_region);

    if starts_new_region {
        if is_accessor_region {
            counts.accessor_regions += 1;
        }
        if is_map_initializer_region {
            counts.map_initializer_regions += 1;
        }
    }

    let child_inside_unwalked = inside_unwalked_region || starts_new_region;

    if child_inside_unwalked {
        counts.unwalked_nodes += 1;
    } else {
        counts.walked_nodes += 1;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, child_inside_unwalked, counts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_apex(src: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_sfapex_vendored::apex::language())
            .expect("load apex grammar");
        parser.parse(src, None).expect("parse apex fixture")
    }

    #[test]
    fn property_accessor_count_is_exact() {
        let src = r#"
public class Foo {
    public Integer x { get { return doSomething(); } set { doSet(value); } }
    public String y { get { return helper(); } private set; }
}
"#;
        let tree = parse_apex(src);
        let cov = extract_apex_file_coverage(&tree, src.as_bytes(), "/fake/Foo.cls");
        // Three `get {...}` / `set {...}` bodies; the fourth accessor
        // is a setter without a body (just `private set;`), which is
        // not an unwalked region because there is nothing to miss.
        let r39 = cov
            .coverage_gaps
            .iter()
            .find_map(|g| match g {
                CoverageGap::ApexPropertyAccessor { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0);
        assert_eq!(r39, 3, "expected 3 R39 regions, got {r39}");
        assert!(cov.has_invalidating_no_callers_gap());
        assert_eq!(cov.confidence, CoverageConfidence::High);
    }

    #[test]
    fn map_literal_count_is_exact() {
        let src = r#"
public class Bar {
    Map<Id, Id> one = new Map<Id, Id>{ 'a' => doOne() };
    Map<Id, Id> two = new Map<Id, Id>{ 'b' => doTwo(), 'c' => doThree() };
}
"#;
        let tree = parse_apex(src);
        let cov = extract_apex_file_coverage(&tree, src.as_bytes(), "/fake/Bar.cls");
        let r41 = cov
            .coverage_gaps
            .iter()
            .find_map(|g| match g {
                CoverageGap::ApexMapLiteralInitializer { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0);
        assert_eq!(r41, 2, "expected 2 R41 regions, got {r41}");
        assert!(cov.has_invalidating_no_callers_gap());
        assert_eq!(cov.confidence, CoverageConfidence::High);
    }

    #[test]
    fn auto_property_emits_no_r39() {
        // `public String s { get; set; }` is compiled by Apex itself
        // without a custom accessor body, so there is no unwalked
        // region — the extractor has nothing to miss here. T8 must
        // not false-positive on these.
        let src = r#"
public class Plain {
    public String s { get; set; }
    public Integer n { get; private set; }
}
"#;
        let tree = parse_apex(src);
        let cov = extract_apex_file_coverage(&tree, src.as_bytes(), "/fake/Plain.cls");
        assert!(
            cov.coverage_gaps.is_empty(),
            "auto-properties should produce no coverage gaps, got {:?}",
            cov.coverage_gaps,
        );
        assert!(!cov.has_invalidating_no_callers_gap());
    }

    #[test]
    fn nested_accessor_does_not_double_count() {
        // A `get { ... }` body that happens to contain a
        // `map_initializer` inside should count as *one* R39 region
        // (the accessor), not also as an R41 — the R41 is already
        // inside the unwalked region and the classifier downgrade
        // fires exactly once either way.
        let src = r#"
public class Nested {
    public Map<Id, Id> m {
        get { return new Map<Id, Id>{ 'x' => helper() }; }
    }
}
"#;
        let tree = parse_apex(src);
        let cov = extract_apex_file_coverage(&tree, src.as_bytes(), "/fake/Nested.cls");
        let r39 = cov
            .coverage_gaps
            .iter()
            .find_map(|g| match g {
                CoverageGap::ApexPropertyAccessor { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0);
        let r41 = cov
            .coverage_gaps
            .iter()
            .find_map(|g| match g {
                CoverageGap::ApexMapLiteralInitializer { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0);
        assert_eq!(r39, 1);
        assert_eq!(
            r41, 0,
            "map initializer inside already-unwalked accessor must not double-count",
        );
    }

    #[test]
    fn confidence_low_on_parse_error() {
        // Deliberate syntax error: unmatched brace triggers a tree
        // with a parse-error node, which must produce `Low`
        // confidence per T8 §6.1.
        let src = "public class Broken { public Integer x { get { return ; ";
        let tree = parse_apex(src);
        let cov = extract_apex_file_coverage(&tree, src.as_bytes(), "/fake/Broken.cls");
        assert_eq!(cov.confidence, CoverageConfidence::Low);
    }

    #[test]
    fn empty_class_emits_no_gaps_high_confidence() {
        let src = "public class Empty {}";
        let tree = parse_apex(src);
        let cov = extract_apex_file_coverage(&tree, src.as_bytes(), "/fake/Empty.cls");
        assert!(cov.coverage_gaps.is_empty());
        assert_eq!(cov.confidence, CoverageConfidence::High);
    }
}
