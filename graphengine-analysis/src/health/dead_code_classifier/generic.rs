//! Language-agnostic dead-code reason classifier.
//!
//! Used as the terminal fallback for every ecosystem. Uses only
//! signals that are valid across all languages: fan-in count,
//! visibility string, `is_attribute_invoked` / `is_callback_target`
//! flags, and the parent file's test/vendor classification. It must
//! never rely on language-specific file extensions, annotation
//! strings, or framework conventions — those belong in the
//! ecosystem-specific modules (Apex, Python, etc.).
//!
//! # Decision order
//!
//! 1. `entry_point_tags` non-empty → `FrameworkAnnotationUnresolved`
//!    (the language-specific extractor recognised a framework signal
//!    on the declaration. Even when a specialist classifier is not
//!    registered for this ecosystem, the presence of a tag is
//!    authoritative evidence that the function is invoked by a
//!    framework runtime outside the static graph.)
//! 2. `is_attribute_invoked` → `FrameworkAnnotationUnresolved`
//! 3. `is_callback_target` → `CallbackTargetNotTracked`
//! 4. Parent file `is_test` → `TestOnlyReference`
//!    (a test function that survived the entry-point filter must be
//!    only referenced from other tests; if it had production callers,
//!    fan-in > 0 would have excluded it from the dead set.)
//! 5. Visibility private + fan_in == 0 → `VisibilityPrivateUnused`
//! 6. Fan-in == 0 (otherwise) → `NoCallers`
//! 7. Unknown → `Unclassified`

use crate::health::dead_code_confidence::{
    framework_extraction_coverage, FrameworkExtractionCoverage,
};
use crate::health::report::DeadCodeReason;

use super::{ClassifyContext, TerminalClassifier};

pub struct GenericClassifier;

impl GenericClassifier {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GenericClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalClassifier for GenericClassifier {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;

        if !node.entry_point_tags.is_empty() {
            return Some((
                DeadCodeReason::FrameworkAnnotationUnresolved,
                format!(
                    "fan_in={}; entry_point_tags={:?}; framework dispatch edge not parsed",
                    ctx.fan_in, node.entry_point_tags
                ),
            ));
        }

        if node.is_attribute_invoked {
            return Some((
                DeadCodeReason::FrameworkAnnotationUnresolved,
                format!(
                    "fan_in={}; is_attribute_invoked=true; framework dispatch edge not parsed",
                    ctx.fan_in
                ),
            ));
        }

        if node.is_callback_target {
            return Some((
                DeadCodeReason::CallbackTargetNotTracked,
                format!(
                    "fan_in={}; is_callback_target=true; caller passes this as a function reference",
                    ctx.fan_in
                ),
            ));
        }

        let parent_is_test = ctx
            .graph
            .classification_of(ctx.node_id)
            .map(|f| f.is_test)
            .unwrap_or(false);
        if parent_is_test {
            return Some((
                DeadCodeReason::TestOnlyReference,
                format!(
                    "fan_in={}; parent file is_test=true; only visible to test callers",
                    ctx.fan_in
                ),
            ));
        }

        if ctx.fan_in == 0 {
            let visibility = node.visibility.as_deref().unwrap_or("");
            let is_private = matches!(visibility, "private" | "fileprivate" | "module_private");

            let reason = if is_private {
                DeadCodeReason::VisibilityPrivateUnused
            } else {
                DeadCodeReason::NoCallers
            };

            // Be honest about what the parser actually checked vs.
            // claimed. The previous phrasing "no framework attribute"
            // implied a check that didn't happen for most languages
            // (B2). Surface the per-ecosystem extraction-coverage
            // gaps directly in the evidence string so a consumer can
            // tell whether "no framework attribute" means "parser
            // looked and found none" or "parser doesn't look at
            // this language's framework attributes at all yet."
            let extraction_note = match framework_extraction_coverage(ctx.ecosystem) {
                FrameworkExtractionCoverage::Full { description } => format!(
                    "no framework-dispatch marker on this node (parser checked: {description})"
                ),
                FrameworkExtractionCoverage::Partial { missing } => format!(
                    "no parsed framework-dispatch marker (note: parser does not currently \
                     traverse {missing} for this language; if this function is invoked by \
                     such a mechanism it will be falsely flagged)"
                ),
                FrameworkExtractionCoverage::Unknown => {
                    "language not recognised — parser did not check any framework-dispatch \
                     patterns for this node"
                        .into()
                }
            };

            let evidence = format!(
                "fan_in=0; visibility={}; no entry_point_tags extracted; {extraction_note}",
                if visibility.is_empty() {
                    "<unset>"
                } else {
                    visibility
                }
            );
            return Some((reason, evidence));
        }

        // Fan-in > 0 with no other signal: this shouldn't happen in
        // the normal pipeline because a node with fan_in > 0 is
        // excluded from the dead set by detect_dead_code. If it does,
        // we stamp Unclassified rather than fabricate a reason.
        Some((
            DeadCodeReason::Unclassified,
            format!(
                "fan_in={}; no generic signal matched (unexpected: fan_in>0 in dead set)",
                ctx.fan_in
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::config::Ecosystem;
    use crate::health::graph::{AnalysisGraph, EdgeKind, GraphEdge, GraphNode, NodeKind};
    use std::collections::BTreeMap;

    fn mk_fn(id: &str, visibility: Option<&str>, file_path: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: NodeKind::Function,
            fqn: format!("test::{id}"),
            name: id.into(),
            file_path: Some(file_path.into()),
            start_line: None,
            end_line: None,
            path_repo_rel: Some(file_path.into()),
            role: None,
            is_test: false,
            is_vendor: false,
            is_build_output: false,
            is_generated: false,
            cyclomatic_complexity: None,
            cognitive_complexity: None,
            visibility: visibility.map(str::to_string),
            import_sources: vec![],
            is_trait_impl: false,
            trait_name: None,
            is_attribute_invoked: false,
            is_callback_target: false,
            entry_point_tags: vec![],
            language: None,
            frameworks: vec![],
            is_synthetic: false,
        }
    }

    fn mk_file(id: &str, path: &str, is_test: bool) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: NodeKind::File,
            fqn: format!("file::{id}"),
            name: id.into(),
            file_path: Some(path.into()),
            start_line: None,
            end_line: None,
            path_repo_rel: Some(path.into()),
            role: Some("source".into()),
            is_test,
            is_vendor: false,
            is_build_output: false,
            is_generated: false,
            cyclomatic_complexity: None,
            cognitive_complexity: None,
            visibility: None,
            import_sources: vec![],
            is_trait_impl: false,
            trait_name: None,
            is_attribute_invoked: false,
            is_callback_target: false,
            entry_point_tags: vec![],
            language: None,
            frameworks: vec![],
            is_synthetic: false,
        }
    }

    fn mk_graph(nodes: Vec<GraphNode>, edges: Vec<(&str, &str, EdgeKind)>) -> AnalysisGraph {
        let mut m = BTreeMap::new();
        for n in nodes {
            m.insert(n.id.clone(), n);
        }
        let es = edges
            .into_iter()
            .map(|(f, t, k)| GraphEdge {
                from_id: f.into(),
                to_id: t.into(),
                kind: k,
                confidence: crate::health::graph::Confidence::High,
            })
            .collect();
        let mut g = AnalysisGraph::build(m, es);
        g.compute_module_membership();
        g
    }

    fn ctx<'a>(g: &'a AnalysisGraph, id: &'a str) -> ClassifyContext<'a> {
        ClassifyContext {
            node_id: id,
            graph: g,
            ecosystem: Ecosystem::Unknown,
            fan_in: g.fan_in(id),
        }
    }

    #[test]
    fn private_unused_is_highest_signal() {
        let g = mk_graph(
            vec![
                mk_file("f", "src/a.ts", false),
                mk_fn("fn", Some("private"), "src/a.ts"),
            ],
            vec![("f", "fn", EdgeKind::Contains)],
        );
        let (r, ev) = GenericClassifier::new().classify(&ctx(&g, "fn")).unwrap();
        assert_eq!(r, DeadCodeReason::VisibilityPrivateUnused);
        assert!(ev.contains("private"));
    }

    #[test]
    fn no_callers_when_public() {
        let g = mk_graph(
            vec![
                mk_file("f", "src/a.ts", false),
                mk_fn("fn", Some("public"), "src/a.ts"),
            ],
            vec![("f", "fn", EdgeKind::Contains)],
        );
        let (r, _) = GenericClassifier::new().classify(&ctx(&g, "fn")).unwrap();
        assert_eq!(r, DeadCodeReason::NoCallers);
    }

    #[test]
    fn attribute_invoked_overrides_fan_in_zero() {
        let node = GraphNode {
            is_attribute_invoked: true,
            ..mk_fn("fn", Some("public"), "src/a.ts")
        };
        let g = mk_graph(
            vec![mk_file("f", "src/a.ts", false), node],
            vec![("f", "fn", EdgeKind::Contains)],
        );
        let (r, _) = GenericClassifier::new().classify(&ctx(&g, "fn")).unwrap();
        assert_eq!(r, DeadCodeReason::FrameworkAnnotationUnresolved);
    }

    #[test]
    fn entry_point_tags_classify_as_framework_invisible() {
        let node = GraphNode {
            entry_point_tags: vec!["aura_enabled".into()],
            ..mk_fn("fn", Some("public"), "src/a.cls")
        };
        let g = mk_graph(
            vec![mk_file("f", "src/a.cls", false), node],
            vec![("f", "fn", EdgeKind::Contains)],
        );
        let (r, ev) = GenericClassifier::new().classify(&ctx(&g, "fn")).unwrap();
        assert_eq!(r, DeadCodeReason::FrameworkAnnotationUnresolved);
        assert!(ev.contains("aura_enabled"));
    }

    #[test]
    fn callback_target_overrides_fan_in_zero() {
        let node = GraphNode {
            is_callback_target: true,
            ..mk_fn("fn", Some("public"), "src/a.ts")
        };
        let g = mk_graph(
            vec![mk_file("f", "src/a.ts", false), node],
            vec![("f", "fn", EdgeKind::Contains)],
        );
        let (r, _) = GenericClassifier::new().classify(&ctx(&g, "fn")).unwrap();
        assert_eq!(r, DeadCodeReason::CallbackTargetNotTracked);
    }

    #[test]
    fn test_only_reference_when_parent_is_test_file() {
        let g = mk_graph(
            vec![
                mk_file("tf", "tests/a.test.ts", true),
                mk_fn("fn", Some("public"), "tests/a.test.ts"),
            ],
            vec![("tf", "fn", EdgeKind::Contains)],
        );
        let (r, _) = GenericClassifier::new().classify(&ctx(&g, "fn")).unwrap();
        assert_eq!(r, DeadCodeReason::TestOnlyReference);
    }

    // B2 regression: the evidence string must not lie about what the
    // parser actually checked. For a language where attribute-macro
    // extraction is not implemented (e.g. Rust today), the evidence
    // string must explicitly say so, so a consumer cannot read
    // "no framework attribute" as "parser confirmed no framework
    // attribute exists" when really the parser didn't look.
    fn ctx_with_eco<'a>(g: &'a AnalysisGraph, id: &'a str, eco: Ecosystem) -> ClassifyContext<'a> {
        ClassifyContext {
            node_id: id,
            graph: g,
            ecosystem: eco,
            fan_in: g.fan_in(id),
        }
    }

    #[test]
    fn evidence_string_admits_rust_attribute_macros_not_traversed() {
        let g = mk_graph(
            vec![
                mk_file("f", "src/lib.rs", false),
                mk_fn("fn_a", Some("pub"), "src/lib.rs"),
            ],
            vec![("f", "fn_a", EdgeKind::Contains)],
        );
        let (_, ev) = GenericClassifier::new()
            .classify(&ctx_with_eco(&g, "fn_a", Ecosystem::Rust))
            .unwrap();
        assert!(
            ev.contains("attribute macros") || ev.contains("#[tool]"),
            "Rust evidence string must name the missing attribute-macro \
             traversal so consumers know what the parser didn't check; got: {ev}"
        );
        assert!(
            !ev.contains("no framework attribute;") && !ev.ends_with("no framework attribute"),
            "must not use the lying short-form phrasing anymore; got: {ev}"
        );
        assert!(
            ev.contains("no entry_point_tags extracted"),
            "evidence must factually state what was checked (entry_point_tags); got: {ev}"
        );
    }

    #[test]
    fn evidence_string_for_python_names_decorator_gap() {
        let g = mk_graph(
            vec![
                mk_file("f", "app.py", false),
                mk_fn("fn_a", Some("public"), "app.py"),
            ],
            vec![("f", "fn_a", EdgeKind::Contains)],
        );
        let (_, ev) = GenericClassifier::new()
            .classify(&ctx_with_eco(&g, "fn_a", Ecosystem::Python))
            .unwrap();
        assert!(
            ev.contains("Decorators") || ev.contains("@app.route"),
            "Python evidence string must name the decorator extraction gap; got: {ev}"
        );
    }

    #[test]
    fn evidence_string_for_apex_advertises_full_coverage() {
        // Apex is the canonical full-extraction language: when the
        // parser sees no framework marker, that's a real signal, not
        // an extraction gap.
        let g = mk_graph(
            vec![
                mk_file("f", "src/A.cls", false),
                mk_fn("fn_a", Some("public"), "src/A.cls"),
            ],
            vec![("f", "fn_a", EdgeKind::Contains)],
        );
        let (_, ev) = GenericClassifier::new()
            .classify(&ctx_with_eco(&g, "fn_a", Ecosystem::Apex))
            .unwrap();
        assert!(
            ev.contains("parser checked") && ev.contains("@AuraEnabled"),
            "Apex evidence must positively assert what was checked, since coverage is \
             full; got: {ev}"
        );
    }
}
