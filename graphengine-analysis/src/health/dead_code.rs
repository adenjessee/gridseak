//! Dead code detection.
//!
//! Identifies function nodes with zero incoming non-containment edges,
//! excluding entry points (test functions, barrel exports, framework handlers, etc.).
//!
//! # Scope contract
//!
//! `DeadCodeResult` is split into three disjoint slices so every
//! downstream consumer reads the exact bucket it cares about and the
//! type system — not a prose comment — enforces the invariant:
//!
//! - `production` is the list counted by `MetricsReport.dead_code.count`
//!   and summed by `dead_code.reason_breakdown`. The headline number
//!   the user sees.
//! - `test` is dead code inside classes/files marked as tests. These
//!   never run in production and are separated so the aggregate does
//!   not inflate.
//! - `vendor` is dead code inside vendored / generated / build-output
//!   files. Categorically someone else's code; should never appear on
//!   a user-facing dead-code chart.
//!
//! Prior versions returned one flat `Vec<String>` and every consumer
//! re-filtered by production/test/vendor. That put N sites in sync
//! with each other and was the root cause of R20 (count ≠ breakdown
//! sum). Typing the result kills the class of bug outright.

use super::config::DeadCodeConfig;
use super::entry_points::is_entry_point;
use super::graph::AnalysisGraph;

/// Dead code partitioned by lifecycle bucket.
///
/// The three slices are disjoint and their union is every dead
/// function node in the graph. Use `all()` to iterate over the full
/// set when you do not care about scope (e.g., stamping per-node
/// annotations for UI drilldowns).
#[derive(Debug, Default, Clone)]
pub struct DeadCodeResult {
    /// Dead functions in production files. This is the set counted
    /// on `MetricsReport.dead_code.count` and the one that must sum-
    /// match `dead_code.reason_breakdown`.
    pub production: Vec<String>,
    /// Dead functions inside test-classified files. Reported
    /// separately so the headline count stays honest.
    pub test: Vec<String>,
    /// Dead functions inside vendor / generated / build-output files.
    /// These are third-party or derived; not the user's code.
    pub vendor: Vec<String>,
}

impl DeadCodeResult {
    /// Iterate every dead node across all three buckets.
    ///
    /// Useful for consumers that stamp per-node annotations or
    /// classify every dead node (the classifier does this so the UI
    /// drilldown is complete, even though only `production` feeds the
    /// headline chart).
    pub fn all(&self) -> impl Iterator<Item = &String> {
        self.production
            .iter()
            .chain(self.test.iter())
            .chain(self.vendor.iter())
    }

    /// Total count across all buckets.
    pub fn total(&self) -> usize {
        self.production.len() + self.test.len() + self.vendor.len()
    }
}

pub fn detect_dead_code(graph: &AnalysisGraph, dc_config: &DeadCodeConfig) -> DeadCodeResult {
    detect_dead_code_impl(graph, dc_config, false)
}

/// High-confidence-only variant of `detect_dead_code`. Only
/// `Confidence::High` edges count as callers; heuristic edges are
/// ignored. Used by T3 dual-metric emission so the report can show
/// "X dead functions under heuristic resolution vs. Y dead
/// functions under authoritative-only resolution" side by side.
pub fn detect_dead_code_high_only(
    graph: &AnalysisGraph,
    dc_config: &DeadCodeConfig,
) -> DeadCodeResult {
    detect_dead_code_impl(graph, dc_config, true)
}

fn detect_dead_code_impl(
    graph: &AnalysisGraph,
    dc_config: &DeadCodeConfig,
    high_only: bool,
) -> DeadCodeResult {
    let mut production = Vec::new();
    let mut test = Vec::new();
    let mut vendor = Vec::new();

    for id in &graph.function_node_ids {
        let fi = if high_only {
            graph.fan_in_high_only(id)
        } else {
            graph.fan_in(id)
        };

        if fi > 0 {
            continue;
        }

        if is_entry_point(graph, id, dc_config) {
            continue;
        }

        // Bucket disjointly by lifecycle. Test is checked before
        // vendor because a file can in principle carry both flags
        // (a vendored test harness) and the test interpretation is
        // the more useful one for a reviewer.
        if graph.is_non_production_node(id) {
            if is_in_test_file(graph, id) {
                test.push(id.clone());
            } else {
                vendor.push(id.clone());
            }
        } else {
            production.push(id.clone());
        }
    }

    production.sort();
    test.sort();
    vendor.sort();

    DeadCodeResult {
        production,
        test,
        vendor,
    }
}

/// True when the node's parent file is classified as a test file.
/// Used to split non-production dead code into the `test` vs `vendor`
/// buckets. The `is_non_production_node` helper does not distinguish
/// between the two — it only tells us "not production".
fn is_in_test_file(graph: &AnalysisGraph, id: &str) -> bool {
    if let Some(node) = graph.nodes.get(id) {
        if node.is_test {
            return true;
        }
    }
    if let Some(parent) = graph.classification_of(id) {
        return parent.is_test;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_graph_with_file(
        nodes: Vec<(&str, NodeKind, Option<&str>)>,
        edges: Vec<(&str, &str, EdgeKind)>,
    ) -> AnalysisGraph {
        let mut node_map = BTreeMap::new();
        for (id, kind, path) in &nodes {
            node_map.insert(
                id.to_string(),
                GraphNode {
                    id: id.to_string(),
                    kind: *kind,
                    fqn: format!("test::{id}"),
                    name: id.to_string(),
                    file_path: path.map(|p| p.to_string()),
                    start_line: None,
                    end_line: None,
                    path_repo_rel: path.map(|p| p.to_string()),
                    role: Some("source".to_string()),
                    is_test: false,
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
                },
            );
        }
        let edges: Vec<GraphEdge> = edges
            .into_iter()
            .map(|(f, t, k)| GraphEdge {
                from_id: f.to_string(),
                to_id: t.to_string(),
                kind: k,
                confidence: crate::health::graph::Confidence::High,
            })
            .collect();
        let mut g = AnalysisGraph::build(node_map, edges);
        g.compute_module_membership();
        g
    }

    fn mk_file(id: &str, path: &str, is_test: bool, is_vendor: bool) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: NodeKind::File,
            fqn: format!("test::{id}"),
            name: id.to_string(),
            file_path: Some(path.to_string()),
            start_line: None,
            end_line: None,
            path_repo_rel: Some(path.to_string()),
            role: Some("source".to_string()),
            is_test,
            is_vendor,
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

    fn mk_fn(id: &str, path: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: NodeKind::Function,
            fqn: format!("test::{id}"),
            name: id.to_string(),
            file_path: Some(path.to_string()),
            start_line: None,
            end_line: None,
            path_repo_rel: Some(path.to_string()),
            role: None,
            is_test: false,
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

    fn build(nodes: Vec<GraphNode>, edges: Vec<(&str, &str, EdgeKind)>) -> AnalysisGraph {
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

    #[test]
    fn called_function_not_dead() {
        let g = make_graph_with_file(
            vec![
                ("file", NodeKind::File, Some("src/utils.ts")),
                ("A", NodeKind::Function, None),
                ("B", NodeKind::Function, None),
            ],
            vec![
                ("file", "A", EdgeKind::Contains),
                ("file", "B", EdgeKind::Contains),
                ("A", "B", EdgeKind::Call),
            ],
        );
        let result = detect_dead_code(&g, &DeadCodeConfig::default());
        let all: Vec<&String> = result.all().collect();
        assert!(!all.iter().any(|id| *id == "B"));
    }

    #[test]
    fn uncalled_function_flagged() {
        let g = make_graph_with_file(
            vec![
                ("file", NodeKind::File, Some("src/utils/deprecated.ts")),
                ("orphan", NodeKind::Function, None),
            ],
            vec![("file", "orphan", EdgeKind::Contains)],
        );
        let result = detect_dead_code(&g, &DeadCodeConfig::default());
        assert!(result.production.contains(&"orphan".to_string()));
    }

    #[test]
    fn test_file_dead_code_never_leaves_entry_point_filter() {
        // Documents the current pipeline: functions in a file flagged
        // `is_test=true` are exempted at the entry_points layer
        // (`ParentFileIsTest`) long before the dead-code bucketer
        // runs. They should never appear in ANY bucket. This test
        // pins that behaviour so we notice if a future change lets
        // test functions leak past the filter.
        let g = build(
            vec![
                mk_file("tf", "src/foo_test.go", true, false),
                mk_fn("orphan", "src/foo_test.go"),
            ],
            vec![("tf", "orphan", EdgeKind::Contains)],
        );
        let result = detect_dead_code(&g, &DeadCodeConfig::default());
        let all: Vec<&String> = result.all().collect();
        assert!(
            !all.iter().any(|id| *id == "orphan"),
            "test-file functions must be exempt at the entry_points layer"
        );
    }

    #[test]
    fn non_prod_path_on_node_id_goes_to_test_or_vendor_bucket() {
        // Reproduces the rare path where `is_non_production_node`
        // returns true via the node-id path heuristic but no parent
        // File node is flagged (so entry_points doesn't exempt).
        // This is the *only* way a non-production dead node can
        // leak through today — and the typed buckets must route it
        // correctly rather than inflate `production`.
        //
        // Using a node whose own path_repo_rel includes `test-repos/`
        // (a pattern recognised by `is_non_production_path`).
        let mut node = mk_fn("orphan", "test-repos/foo/bar.rs");
        node.path_repo_rel = Some("test-repos/foo/bar.rs".into());
        let g = build(
            vec![mk_file("f", "test-repos/foo/bar.rs", false, false), node],
            vec![("f", "orphan", EdgeKind::Contains)],
        );
        let r = detect_dead_code(&g, &DeadCodeConfig::default());
        // Either test or vendor bucket must claim the node; production must not.
        let in_nonprod =
            r.test.contains(&"orphan".to_string()) || r.vendor.contains(&"orphan".to_string());
        assert!(
            in_nonprod,
            "non-production dead node must land in test or vendor bucket, not production"
        );
        assert!(!r.production.contains(&"orphan".to_string()));
    }

    #[test]
    fn buckets_are_disjoint_and_sum_to_all() {
        // Property test for the scope contract:
        //   all() = production ∪ test ∪ vendor
        //   the three slices share no element
        //   total() equals the sum of slice lengths
        //
        // Uses a production-only fixture because entry_points exempts
        // dead functions living in classified test/vendor files — so
        // those buckets are typically empty in practice. The union
        // invariant holds regardless of population distribution.
        let g = build(
            vec![
                mk_file("pf", "src/prod.rs", false, false),
                mk_fn("p1", "src/prod.rs"),
                mk_fn("p2", "src/prod.rs"),
                mk_fn("p3", "src/prod.rs"),
            ],
            vec![
                ("pf", "p1", EdgeKind::Contains),
                ("pf", "p2", EdgeKind::Contains),
                ("pf", "p3", EdgeKind::Contains),
            ],
        );
        let r = detect_dead_code(&g, &DeadCodeConfig::default());

        let all: std::collections::BTreeSet<&String> = r.all().collect();
        let prod_set: std::collections::BTreeSet<&String> = r.production.iter().collect();
        let test_set: std::collections::BTreeSet<&String> = r.test.iter().collect();
        let vendor_set: std::collections::BTreeSet<&String> = r.vendor.iter().collect();

        // Disjoint
        assert!(prod_set.is_disjoint(&test_set));
        assert!(prod_set.is_disjoint(&vendor_set));
        assert!(test_set.is_disjoint(&vendor_set));

        // Union equals all()
        let union: std::collections::BTreeSet<&String> = prod_set
            .union(&test_set)
            .chain(vendor_set.iter())
            .copied()
            .collect();
        assert_eq!(union, all);

        // Total consistent
        assert_eq!(
            r.total(),
            r.production.len() + r.test.len() + r.vendor.len()
        );
        assert_eq!(r.total(), all.len());
        // In this production-only fixture, all dead code lands in production.
        assert_eq!(r.production.len(), 3);
        assert!(r.test.is_empty());
        assert!(r.vendor.is_empty());
    }

    #[test]
    fn handler_exempt() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file".to_string(),
            GraphNode {
                id: "file".to_string(),
                kind: NodeKind::File,
                fqn: "test::file".to_string(),
                name: "file".to_string(),
                file_path: Some("src/api/users.ts".to_string()),
                start_line: None,
                end_line: None,
                path_repo_rel: Some("src/api/users.ts".to_string()),
                role: Some("source".to_string()),
                is_test: false,
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
            },
        );
        nodes.insert(
            "getHandler".to_string(),
            GraphNode {
                id: "getHandler".to_string(),
                kind: NodeKind::Function,
                fqn: "test::getHandler".to_string(),
                name: "getHandler".to_string(),
                file_path: None,
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
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
            },
        );
        let edges = vec![GraphEdge {
            from_id: "file".to_string(),
            to_id: "getHandler".to_string(),
            kind: EdgeKind::Contains,
            confidence: crate::health::graph::Confidence::High,
        }];
        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();

        let result = detect_dead_code(&g, &DeadCodeConfig::default());
        let all: Vec<&String> = result.all().collect();
        assert!(
            !all.iter().any(|id| *id == "getHandler"),
            "Framework handlers should be exempt"
        );
    }

    #[test]
    fn exported_public_function_exempt_when_enabled() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file".into(),
            GraphNode {
                id: "file".into(),
                kind: NodeKind::File,
                fqn: "test::file".into(),
                name: "file".into(),
                file_path: Some("src/router.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: Some("src/router.rs".into()),
                role: Some("source".into()),
                is_test: false,
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
            },
        );
        nodes.insert(
            "pub_fn".into(),
            GraphNode {
                id: "pub_fn".into(),
                kind: NodeKind::Function,
                fqn: "test::public_api".into(),
                name: "public_api".into(),
                file_path: Some("src/router.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
                is_vendor: false,
                is_build_output: false,
                is_generated: false,
                cyclomatic_complexity: None,
                cognitive_complexity: None,
                visibility: Some("public".into()),
                import_sources: vec![],
                is_trait_impl: false,
                trait_name: None,
                is_attribute_invoked: false,
                is_callback_target: false,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        );
        nodes.insert(
            "priv_fn".into(),
            GraphNode {
                id: "priv_fn".into(),
                kind: NodeKind::Function,
                fqn: "test::internal_helper".into(),
                name: "internal_helper".into(),
                file_path: Some("src/router.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
                is_vendor: false,
                is_build_output: false,
                is_generated: false,
                cyclomatic_complexity: None,
                cognitive_complexity: None,
                visibility: Some("private".into()),
                import_sources: vec![],
                is_trait_impl: false,
                trait_name: None,
                is_attribute_invoked: false,
                is_callback_target: false,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        );
        let edges = vec![
            GraphEdge {
                from_id: "file".into(),
                to_id: "pub_fn".into(),
                kind: EdgeKind::Contains,
                confidence: crate::health::graph::Confidence::High,
            },
            GraphEdge {
                from_id: "file".into(),
                to_id: "priv_fn".into(),
                kind: EdgeKind::Contains,
                confidence: crate::health::graph::Confidence::High,
            },
        ];
        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();

        let cfg = DeadCodeConfig {
            exported_symbols: true,
            ..DeadCodeConfig::default()
        };
        let result = detect_dead_code(&g, &cfg);
        let all: Vec<&String> = result.all().collect();

        assert!(
            !all.iter().any(|id| *id == "pub_fn"),
            "Public/exported functions should be exempt when exported_symbols is enabled"
        );
        assert!(
            all.iter().any(|id| *id == "priv_fn"),
            "Private functions with no callers should still be flagged"
        );
    }

    #[test]
    fn exported_symbols_disabled_flags_both() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file".into(),
            GraphNode {
                id: "file".into(),
                kind: NodeKind::File,
                fqn: "test::file".into(),
                name: "file".into(),
                file_path: Some("src/mod.ts".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: Some("src/mod.ts".into()),
                role: Some("source".into()),
                is_test: false,
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
            },
        );
        nodes.insert(
            "exported_fn".into(),
            GraphNode {
                id: "exported_fn".into(),
                kind: NodeKind::Function,
                fqn: "test::myFunc".into(),
                name: "myFunc".into(),
                file_path: Some("src/mod.ts".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
                is_vendor: false,
                is_build_output: false,
                is_generated: false,
                cyclomatic_complexity: None,
                cognitive_complexity: None,
                visibility: Some("exported".into()),
                import_sources: vec![],
                is_trait_impl: false,
                trait_name: None,
                is_attribute_invoked: false,
                is_callback_target: false,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        );
        let edges = vec![GraphEdge {
            from_id: "file".into(),
            to_id: "exported_fn".into(),
            kind: EdgeKind::Contains,
            confidence: crate::health::graph::Confidence::High,
        }];
        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();

        let cfg = DeadCodeConfig {
            exported_symbols: false,
            ..DeadCodeConfig::default()
        };
        let result = detect_dead_code(&g, &cfg);
        let all: Vec<&String> = result.all().collect();

        assert!(
            all.iter().any(|id| *id == "exported_fn"),
            "When exported_symbols rule is disabled, exported functions with no callers are flagged"
        );
    }

    #[test]
    fn trait_impl_exempt_when_enabled() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file".into(),
            GraphNode {
                id: "file".into(),
                kind: NodeKind::File,
                fqn: "test::file".into(),
                name: "file".into(),
                file_path: Some("src/session.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: Some("src/session.rs".into()),
                role: Some("source".into()),
                is_test: false,
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
            },
        );
        nodes.insert(
            "trait_fn".into(),
            GraphNode {
                id: "trait_fn".into(),
                kind: NodeKind::Function,
                fqn: "test::close_document".into(),
                name: "close_document".into(),
                file_path: Some("src/session.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
                is_vendor: false,
                is_build_output: false,
                is_generated: false,
                cyclomatic_complexity: None,
                cognitive_complexity: None,
                visibility: Some("public".into()),
                import_sources: vec![],
                is_trait_impl: true,
                trait_name: Some("LspSession".into()),
                is_attribute_invoked: false,
                is_callback_target: false,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        );
        nodes.insert(
            "regular_fn".into(),
            GraphNode {
                id: "regular_fn".into(),
                kind: NodeKind::Function,
                fqn: "test::unused_helper".into(),
                name: "unused_helper".into(),
                file_path: Some("src/session.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
                is_vendor: false,
                is_build_output: false,
                is_generated: false,
                cyclomatic_complexity: None,
                cognitive_complexity: None,
                visibility: Some("private".into()),
                import_sources: vec![],
                is_trait_impl: false,
                trait_name: None,
                is_attribute_invoked: false,
                is_callback_target: false,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        );
        let edges = vec![
            GraphEdge {
                from_id: "file".into(),
                to_id: "trait_fn".into(),
                kind: EdgeKind::Contains,
                confidence: crate::health::graph::Confidence::High,
            },
            GraphEdge {
                from_id: "file".into(),
                to_id: "regular_fn".into(),
                kind: EdgeKind::Contains,
                confidence: crate::health::graph::Confidence::High,
            },
        ];
        let mut g = AnalysisGraph::build(nodes, edges);
        g.compute_module_membership();

        let cfg = DeadCodeConfig {
            trait_impls: true,
            exported_symbols: false,
            ..DeadCodeConfig::default()
        };
        let result = detect_dead_code(&g, &cfg);
        let all: Vec<&String> = result.all().collect();

        assert!(
            !all.iter().any(|id| *id == "trait_fn"),
            "Trait impl functions should be exempt when trait_impls is enabled"
        );
        assert!(
            all.iter().any(|id| *id == "regular_fn"),
            "Non-trait functions with no callers should still be flagged"
        );
    }

    #[test]
    fn callback_target_exempt() {
        let g = make_graph_with_file(
            vec![
                ("file", NodeKind::File, Some("src/db.rs")),
                ("cb", NodeKind::Function, None),
            ],
            vec![("file", "cb", EdgeKind::Contains)],
        );

        let result = detect_dead_code(&g, &DeadCodeConfig::default());
        let all: Vec<&String> = result.all().collect();
        assert!(all.iter().any(|id| *id == "cb"));

        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file".into(),
            GraphNode {
                id: "file".into(),
                kind: NodeKind::File,
                fqn: "test::file".into(),
                name: "file".into(),
                file_path: Some("src/db.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: Some("src/db.rs".into()),
                role: Some("source".into()),
                is_test: false,
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
            },
        );
        nodes.insert(
            "row_to_node".into(),
            GraphNode {
                id: "row_to_node".into(),
                kind: NodeKind::Function,
                fqn: "test::row_to_node".into(),
                name: "row_to_node".into(),
                file_path: Some("src/db.rs".into()),
                start_line: None,
                end_line: None,
                path_repo_rel: None,
                role: None,
                is_test: false,
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
                is_callback_target: true,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        );
        let edges = vec![GraphEdge {
            from_id: "file".into(),
            to_id: "row_to_node".into(),
            kind: EdgeKind::Contains,
            confidence: crate::health::graph::Confidence::High,
        }];
        let mut g2 = AnalysisGraph::build(nodes, edges);
        g2.compute_module_membership();

        let result2 = detect_dead_code(&g2, &DeadCodeConfig::default());
        let all2: Vec<&String> = result2.all().collect();
        assert!(
            !all2.iter().any(|id| *id == "row_to_node"),
            "Callback target functions should be exempt from dead code"
        );
    }
}
