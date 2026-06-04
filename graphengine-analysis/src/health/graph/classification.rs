//! Node-classification predicates that depend on
//! [`super::analysis_graph::AnalysisGraph`].
//!
//! Two kinds of API live here:
//!
//! 1. [`is_synthetic_node`] — a pure free function on `&GraphNode`. No
//!    `AnalysisGraph` required, so the builder can call it before the
//!    graph itself exists. Kept in this module rather than `types.rs`
//!    because it is a *classification predicate*, not part of the
//!    data model.
//! 2. An `impl AnalysisGraph` block exposing [`classification_of`] and
//!    [`is_non_production_node`]. These walk the containment tree and
//!    the file-path index that only exist once the graph is built, so
//!    they cannot live on `GraphNode` alone.
//!
//! Both surfaces feed every Layer-3 metric's "production vs. test"
//! filter. See the parent module's `finalize_production_edges` for the
//! single call-site where the predicates are turned into the
//! `production_structural_edge_indices` set.

use std::collections::HashSet;

use super::analysis_graph::AnalysisGraph;
use super::types::{GraphNode, NodeKind};

/// Returns true for parser-generated synthetic nodes that should not be
/// treated as meaningful user-authored declarations in analysis (they distort
/// dead-code precision, fan-in, cycles, API surface, etc.).
///
/// Two disjoint sources are recognised:
///
/// 1. The **legacy file-scope wrappers** (`__file_scope__`, `__file_module__`).
///    These predate the `properties.synthetic` marker on the parsing side;
///    the name-based check preserves backward compatibility with older
///    parse DBs that do not stamp the flag.
/// 2. Any node the parsing crate explicitly stamped with
///    `properties.synthetic = true`. This currently covers Apex
///    `__trigger__`, `__vf_page__`, R41 field-initializer
///    `<field>.__init__()`, and R39 property-accessor `<prop>.__get__()` /
///    `<prop>.__set__()` wrappers, plus any future language extractor that
///    needs to synthesize an enclosing callable so the heuristic resolver
///    can attribute call sites. Driving the exclusion off a first-class
///    boolean (rather than FQN pattern-matching) keeps the analysis crate
///    agnostic to the extractor's naming conventions — if Python's
///    `__init__` ever shares a suffix with an Apex synthetic, we still
///    classify each correctly because only the synthetic one carries the
///    flag.
pub fn is_synthetic_node(node: &GraphNode) -> bool {
    node.is_synthetic
        || node.name == "__file_scope__"
        || node.name == "__file_module__"
        || node.fqn.ends_with("::__file_scope__")
        || node.fqn.ends_with("::__file_module__")
}

impl AnalysisGraph {
    /// Resolve classification for a node by finding the File node it physically lives in.
    /// Uses the node's `file_path` (from `location.file`) to look up the File node directly,
    /// avoiding the containment tree which may route through Module nodes to the wrong File.
    pub fn classification_of(&self, node_id: &str) -> Option<&GraphNode> {
        let node = self.nodes.get(node_id)?;

        if let Some(ref fp) = node.file_path {
            if let Some(file_node_id) = self.file_path_index.get(fp) {
                if let Some(file_node) = self.nodes.get(file_node_id) {
                    if file_node.path_repo_rel.is_some()
                        || file_node.is_test
                        || file_node.role.is_some()
                    {
                        return Some(file_node);
                    }
                }
            }
        }

        let mut current = node_id.to_string();
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return None;
            }
            if let Some(n) = self.nodes.get(&current) {
                if (n.kind == NodeKind::File || n.kind == NodeKind::Folder)
                    && (n.path_repo_rel.is_some() || n.is_test || n.role.is_some())
                {
                    return Some(n);
                }
            }
            match self.containment_parent.get(&current) {
                Some(parent) => current = parent.clone(),
                None => return None,
            }
        }
    }

    /// Returns `true` if this node belongs to non-production code (test, example,
    /// benchmark, fixture, vendor, generated, etc.).
    ///
    /// Combines three signals:
    /// 1. Parent File node flags (`is_test`, `is_vendor`, `is_generated`, `is_build_output`)
    /// 2. Path heuristic on the parent file's `path_repo_rel` (repo-relative, safe from workspace dirs)
    /// 3. Path heuristic on the node_id itself (covers module-key-style IDs like "examples/resource")
    ///
    /// NOTE: We intentionally use `path_repo_rel` (repo-relative) instead of `file_path`
    /// (absolute) because absolute paths may contain workspace directory names like
    /// `test-repos/` that trigger false positives in the word-boundary matcher.
    pub fn is_non_production_node(&self, node_id: &str) -> bool {
        if let Some(node) = self.nodes.get(node_id) {
            if node.is_test {
                return true;
            }
        }

        if let Some(file_node) = self.classification_of(node_id) {
            if file_node.is_test
                || file_node.is_vendor
                || file_node.is_build_output
                || file_node.is_generated
            {
                return true;
            }
            if let Some(ref rel) = file_node.path_repo_rel {
                if super::super::path_classification::is_non_production_path(rel) {
                    return true;
                }
            }
        }

        if let Some(node) = self.nodes.get(node_id) {
            if let Some(ref rel) = node.path_repo_rel {
                if super::super::path_classification::is_non_production_path(rel) {
                    return true;
                }
            }
        }

        super::super::path_classification::is_non_production_path(node_id)
    }
}
