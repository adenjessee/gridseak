//! Structural test classification using import graph analysis.
//!
//! Implements a 4-tier classification system from the extended spec (Section 2.3):
//!
//! - **Tier 1**: Direct test consumer — file imports a known test framework module
//! - **Tier 2**: Transitive test support — file is only imported by Tier 1/2 files
//! - **Tier 3**: AST-level attributes — parser sets `is_test` on nodes with
//!   `#[test]` / `#[cfg(test)]` (Rust), `@Test` (Java/Kotlin), `[Fact]` (C#).
//!   Files containing any Tier 3 node are classified as test files.
//! - **Tier 4**: Path corroboration — path patterns activate only with structural evidence
//! - **Default**: Production (no signal detected)

use std::collections::{HashMap, HashSet};

use super::graph::{AnalysisGraph, EdgeKind, NodeKind};
use super::path_classification;
use super::test_framework_registry;

/// Classification assigned to a file node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileClassification {
    pub role: FileRole,
    pub tier: ClassificationTier,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    Production,
    Test,
    TestSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClassificationTier {
    Tier1Definitive,
    Tier2Transitive,
    Tier3AstAttribute,
    Tier4PathCorroboration,
    None,
}

/// Classify all file-level module nodes in the graph.
///
/// Returns a map from node ID to classification. Only nodes that are
/// classified as test/test_support are included; everything else is
/// implicitly production.
pub fn classify_files(
    graph: &AnalysisGraph,
    language: &str,
) -> HashMap<String, FileClassification> {
    let mut classifications: HashMap<String, FileClassification> = HashMap::new();

    // --- Tier 1: Direct test framework consumers ---
    let tier1_ids = classify_tier1(graph, language);
    for (id, reason) in &tier1_ids {
        classifications.insert(
            id.clone(),
            FileClassification {
                role: FileRole::Test,
                tier: ClassificationTier::Tier1Definitive,
                reason: reason.clone(),
            },
        );
    }

    // --- Tier 3: AST-level test attributes ---
    // Files containing nodes with is_test=true (e.g., Rust #[test]/#[cfg(test)])
    let tier3_ids = classify_tier3(graph, &classifications);
    for (id, reason) in &tier3_ids {
        classifications.insert(
            id.clone(),
            FileClassification {
                role: FileRole::Test,
                tier: ClassificationTier::Tier3AstAttribute,
                reason: reason.clone(),
            },
        );
    }

    // --- Tier 2: Transitive test support ---
    let tier2_ids = classify_tier2(graph, &classifications);
    for (id, reason) in &tier2_ids {
        classifications.insert(
            id.clone(),
            FileClassification {
                role: FileRole::TestSupport,
                tier: ClassificationTier::Tier2Transitive,
                reason: reason.clone(),
            },
        );
    }

    // --- Tier 4: Path corroboration ---
    let tier4_ids = classify_tier4(graph, &classifications);
    for (id, reason) in tier4_ids {
        classifications.entry(id).or_insert(FileClassification {
            role: FileRole::Test,
            tier: ClassificationTier::Tier4PathCorroboration,
            reason,
        });
    }

    classifications
}

/// Tier 1: Check each file module's `import_sources` property against the
/// test framework registry. If any import source matches, the file is a
/// direct test consumer.
fn classify_tier1(graph: &AnalysisGraph, language: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();

    for (id, node) in &graph.nodes {
        if !is_file_module(node) {
            continue;
        }

        for source in &node.import_sources {
            if test_framework_registry::is_test_framework_import(language, source) {
                results.push((id.clone(), format!("Imports test framework: {}", source)));
                break;
            }
        }
    }

    results
}

/// Tier 3: AST-level test attribute detection.
///
/// For Rust, `#[cfg(test)] mod tests { ... }` lives inside production files,
/// so we do NOT classify the whole file as test here — that would exclude
/// the production code. Instead, individual node-level `is_test` flags are
/// checked by `AnalysisGraph::is_non_production_node`.
///
/// This tier classifies a file as test only when it is a standalone test file
/// (e.g. Java `@Test`, C# `[Fact]`) where *every* function-level node has
/// `is_test=true`. Currently this mainly prepares for Java/C#/Kotlin support.
fn classify_tier3(
    graph: &AnalysisGraph,
    existing: &HashMap<String, FileClassification>,
) -> Vec<(String, String)> {
    let mut results = Vec::new();

    for (id, node) in &graph.nodes {
        if !is_file_module(node) || existing.contains_key(id) {
            continue;
        }

        if let Some(members) = graph.analysis_module_members_of(id) {
            let function_nodes: Vec<&super::graph::GraphNode> = members
                .iter()
                .filter_map(|mid| graph.nodes.get(mid))
                .filter(|n| n.kind == super::graph::NodeKind::Function)
                .collect();

            // Only classify if the file has functions and ALL of them are test
            if !function_nodes.is_empty() && function_nodes.iter().all(|n| n.is_test) {
                results.push((
                    id.clone(),
                    "All functions have test attributes (e.g. #[test], @Test)".to_string(),
                ));
            }
        }
    }

    results
}

/// Tier 2: Iterative — files whose ONLY importers are Tier 1 or Tier 2 files.
/// Iterate to a fixed point (files that support tests but don't import frameworks).
fn classify_tier2(
    graph: &AnalysisGraph,
    existing: &HashMap<String, FileClassification>,
) -> Vec<(String, String)> {
    let file_module_ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|(_, n)| is_file_module(n))
        .map(|(id, _)| id.as_str())
        .collect();

    // Build a reverse import map: file_module_id -> set of file_module_ids that import it
    let mut importers_of: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind != EdgeKind::Import {
            continue;
        }
        let from_file = find_file_module_for(graph, &edge.from_id);
        let to_file = find_file_module_for(graph, &edge.to_id);

        if let (Some(from), Some(to)) = (from_file, to_file) {
            if from != to && file_module_ids.contains(from) && file_module_ids.contains(to) {
                importers_of.entry(to).or_default().insert(from);
            }
        }
    }

    let mut test_set: HashSet<&str> = existing.keys().map(|s| s.as_str()).collect();
    let mut newly_classified: Vec<(String, String)> = Vec::new();
    let mut changed = true;

    while changed {
        changed = false;
        for &file_id in &file_module_ids {
            if test_set.contains(file_id) {
                continue;
            }

            if let Some(importers) = importers_of.get(file_id) {
                if importers.is_empty() {
                    continue;
                }
                let all_importers_are_test = importers.iter().all(|imp| test_set.contains(*imp));
                if all_importers_are_test {
                    // Guard: only classify as test-support if the file's
                    // repo-relative path already looks like test/auxiliary code.
                    // Production paths (e.g. lib/, src/) must stay production
                    // even when the only in-repo consumers are tests (common in
                    // library repos). We use resolve_path_for to get the
                    // repo-relative path, avoiding false positives from absolute
                    // paths that may contain "test" in their clone directory.
                    let rel_path = graph.resolve_path_for(file_id).unwrap_or_default();
                    if !rel_path.is_empty()
                        && !path_classification::is_non_production_path(&rel_path)
                    {
                        continue;
                    }

                    test_set.insert(file_id);
                    newly_classified.push((
                        file_id.to_string(),
                        "Only imported by test/test-support files".to_string(),
                    ));
                    changed = true;
                }
            }
        }
    }

    newly_classified
}

/// Tier 4: Path patterns as corroboration only.
/// Activates only when a file has no import edges AND is in a directory
/// containing at least one Tier 1 confirmed test file.
fn classify_tier4(
    graph: &AnalysisGraph,
    existing: &HashMap<String, FileClassification>,
) -> Vec<(String, String)> {
    let mut results = Vec::new();

    // Collect directories that contain at least one Tier 1 test file
    let tier1_dirs: HashSet<String> = existing
        .iter()
        .filter(|(_, c)| c.tier == ClassificationTier::Tier1Definitive)
        .filter_map(|(id, _)| {
            let node = graph.nodes.get(id)?;
            let path = node
                .file_path
                .as_deref()
                .or(node.path_repo_rel.as_deref())?;
            path.rsplit_once('/').map(|(dir, _)| dir.to_string())
        })
        .collect();

    for (id, node) in &graph.nodes {
        if !is_file_module(node) || existing.contains_key(id) {
            continue;
        }

        // Only apply if the file has no import edges at all
        let has_import_edges = node.import_sources.is_empty()
            && !graph
                .outgoing
                .get(id)
                .map(|indices| {
                    indices
                        .iter()
                        .any(|&idx| graph.edges[idx].kind == EdgeKind::Import)
                })
                .unwrap_or(false);

        if !has_import_edges {
            continue;
        }

        let path = node
            .file_path
            .as_deref()
            .or(node.path_repo_rel.as_deref())
            .unwrap_or("");

        if path.is_empty() {
            continue;
        }

        // Check if the path matches test patterns AND is in a directory with Tier 1 files
        if path_classification::is_test_path(path) {
            if let Some((dir, _)) = path.rsplit_once('/') {
                if tier1_dirs.contains(dir) {
                    results.push((
                        id.clone(),
                        format!("Path pattern corroborated by Tier 1 sibling in {}", dir),
                    ));
                }
            }
        }
    }

    results
}

fn is_file_module(node: &super::graph::GraphNode) -> bool {
    node.kind == NodeKind::Module && node.name == "__file_module__"
}

/// Given a node ID, find the file-module that contains it.
/// For `__file_module__` nodes, return self. For others, walk containment.
fn find_file_module_for<'a>(graph: &'a AnalysisGraph, node_id: &'a str) -> Option<&'a str> {
    if let Some(node) = graph.nodes.get(node_id) {
        if is_file_module(node) {
            return Some(node_id);
        }
    }

    // Walk containment tree up to find the file module
    let mut current = node_id;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current) {
            break;
        }
        if let Some(parent) = graph.containment_parent.get(current) {
            if let Some(pnode) = graph.nodes.get(parent.as_str()) {
                if is_file_module(pnode) {
                    return Some(parent.as_str());
                }
            }
            current = parent.as_str();
        } else {
            break;
        }
    }

    None
}

/// Check if a specific file node is structurally classified as test.
pub fn is_structurally_test(
    classifications: &HashMap<String, FileClassification>,
    node_id: &str,
) -> bool {
    classifications
        .get(node_id)
        .map(|c| matches!(c.role, FileRole::Test | FileRole::TestSupport))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::*;
    use std::collections::BTreeMap;

    fn make_file_module(id: &str, path: &str, import_sources: Vec<String>) -> (String, GraphNode) {
        (
            id.to_string(),
            GraphNode {
                id: id.to_string(),
                kind: NodeKind::Module,
                fqn: format!("{}::__file_module__", path),
                name: "__file_module__".to_string(),
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
                import_sources,
                is_trait_impl: false,
                trait_name: None,
                is_attribute_invoked: false,
                is_callback_target: false,
                entry_point_tags: vec![],
                language: None,
                frameworks: vec![],
                is_synthetic: false,
            },
        )
    }

    #[test]
    fn tier1_jest_import_classifies_as_test() {
        let mut nodes = BTreeMap::new();
        let (id, node) = make_file_module(
            "test_file",
            "src/__tests__/auth.test.ts",
            vec!["jest".into(), "./auth".into()],
        );
        nodes.insert(id, node);

        let graph = AnalysisGraph::build(nodes, vec![]);
        let result = classify_files(&graph, "typescript");

        assert_eq!(result.get("test_file").unwrap().role, FileRole::Test);
        assert_eq!(
            result.get("test_file").unwrap().tier,
            ClassificationTier::Tier1Definitive
        );
    }

    #[test]
    fn tier1_pytest_import_classifies_as_test() {
        let mut nodes = BTreeMap::new();
        let (id, node) = make_file_module(
            "test_file",
            "tests/test_auth.py",
            vec!["pytest".into(), "myapp.auth".into()],
        );
        nodes.insert(id, node);

        let graph = AnalysisGraph::build(nodes, vec![]);
        let result = classify_files(&graph, "python");

        assert_eq!(result.get("test_file").unwrap().role, FileRole::Test);
    }

    #[test]
    fn production_file_not_classified() {
        let mut nodes = BTreeMap::new();
        let (id, node) = make_file_module(
            "prod_file",
            "src/auth.ts",
            vec!["express".into(), "./db".into()],
        );
        nodes.insert(id, node);

        let graph = AnalysisGraph::build(nodes, vec![]);
        let result = classify_files(&graph, "typescript");

        assert!(!result.contains_key("prod_file"));
    }

    #[test]
    fn tier2_transitive_test_support() {
        let mut nodes = BTreeMap::new();

        let (id1, n1) = make_file_module(
            "test_file",
            "tests/test_auth.ts",
            vec!["jest".into(), "./test_helpers".into()],
        );
        nodes.insert(id1, n1);

        let (id2, n2) = make_file_module(
            "helper_file",
            "tests/test_helpers.ts",
            vec!["./utils".into()],
        );
        nodes.insert(id2, n2);

        let edges = vec![GraphEdge {
            from_id: "test_file".into(),
            to_id: "helper_file".into(),
            kind: EdgeKind::Import,
            confidence: crate::health::graph::Confidence::High,
        }];

        let mut graph = AnalysisGraph::build(nodes, edges);
        graph.compute_module_membership();

        let result = classify_files(&graph, "typescript");

        assert_eq!(result.get("test_file").unwrap().role, FileRole::Test);
        assert_eq!(
            result.get("helper_file").unwrap().role,
            FileRole::TestSupport
        );
        assert_eq!(
            result.get("helper_file").unwrap().tier,
            ClassificationTier::Tier2Transitive
        );
    }

    #[test]
    fn test_framework_repo_production_code_not_misclassified() {
        let mut nodes = BTreeMap::new();

        // This is a file in a test framework repo that DEFINES test APIs
        // but doesn't import any test framework itself.
        let (id, node) = make_file_module(
            "framework_core",
            "src/test_runner.py",
            vec!["os".into(), "sys".into()],
        );
        nodes.insert(id, node);

        let graph = AnalysisGraph::build(nodes, vec![]);
        let result = classify_files(&graph, "python");

        assert!(
            !result.contains_key("framework_core"),
            "Framework code that doesn't import test frameworks should stay production"
        );
    }

    #[test]
    fn go_testing_import_detected() {
        let mut nodes = BTreeMap::new();
        let (id, node) = make_file_module(
            "test_file",
            "router_test.go",
            vec!["testing".into(), "net/http".into()],
        );
        nodes.insert(id, node);

        let graph = AnalysisGraph::build(nodes, vec![]);
        let result = classify_files(&graph, "go");

        assert_eq!(result.get("test_file").unwrap().role, FileRole::Test);
    }

    #[test]
    fn library_code_only_imported_by_tests_stays_production() {
        let mut nodes = BTreeMap::new();

        let (id1, n1) = make_file_module(
            "test_file",
            "__tests__/app.test.js",
            vec!["jest".into(), "../lib/application".into()],
        );
        nodes.insert(id1, n1);

        let (id2, n2) = make_file_module(
            "lib_file",
            "lib/application.js",
            vec!["http".into(), "events".into()],
        );
        nodes.insert(id2, n2);

        let edges = vec![GraphEdge {
            from_id: "test_file".into(),
            to_id: "lib_file".into(),
            kind: EdgeKind::Import,
            confidence: crate::health::graph::Confidence::High,
        }];

        let mut graph = AnalysisGraph::build(nodes, edges);
        graph.compute_module_membership();

        let result = classify_files(&graph, "javascript");

        assert_eq!(result.get("test_file").unwrap().role, FileRole::Test);
        assert!(
            !result.contains_key("lib_file"),
            "Library code in lib/ should stay production even when only imported by tests"
        );
    }

    #[test]
    fn test_helper_in_test_dir_only_imported_by_tests_becomes_support() {
        let mut nodes = BTreeMap::new();

        let (id1, n1) = make_file_module(
            "test_file",
            "__tests__/auth.test.js",
            vec!["jest".into(), "../test-helpers/setup".into()],
        );
        nodes.insert(id1, n1);

        let (id2, n2) = make_file_module(
            "helper_file",
            "test-helpers/setup.js",
            vec!["./utils".into()],
        );
        nodes.insert(id2, n2);

        let edges = vec![GraphEdge {
            from_id: "test_file".into(),
            to_id: "helper_file".into(),
            kind: EdgeKind::Import,
            confidence: crate::health::graph::Confidence::High,
        }];

        let mut graph = AnalysisGraph::build(nodes, edges);
        graph.compute_module_membership();

        let result = classify_files(&graph, "javascript");

        assert_eq!(result.get("test_file").unwrap().role, FileRole::Test);
        assert_eq!(
            result.get("helper_file").unwrap().role,
            FileRole::TestSupport,
            "Test helper in test-helpers/ dir should become test support"
        );
    }

    #[test]
    fn helper_with_production_importers_stays_production() {
        let mut nodes = BTreeMap::new();

        let (id1, n1) = make_file_module(
            "test_file",
            "tests/test_auth.ts",
            vec!["jest".into(), "./shared_utils".into()],
        );
        nodes.insert(id1, n1);

        let (id2, n2) = make_file_module(
            "prod_file",
            "src/auth.ts",
            vec!["express".into(), "./shared_utils".into()],
        );
        nodes.insert(id2, n2);

        // shared_utils is imported by BOTH a test and a production file
        let (id3, n3) =
            make_file_module("shared_utils", "src/shared_utils.ts", vec!["lodash".into()]);
        nodes.insert(id3, n3);

        let edges = vec![
            GraphEdge {
                from_id: "test_file".into(),
                to_id: "shared_utils".into(),
                kind: EdgeKind::Import,
                confidence: crate::health::graph::Confidence::High,
            },
            GraphEdge {
                from_id: "prod_file".into(),
                to_id: "shared_utils".into(),
                kind: EdgeKind::Import,
                confidence: crate::health::graph::Confidence::High,
            },
        ];

        let mut graph = AnalysisGraph::build(nodes, edges);
        graph.compute_module_membership();

        let result = classify_files(&graph, "typescript");

        assert!(
            !result.contains_key("shared_utils"),
            "File imported by both test and production should stay production"
        );
    }
}
