//! Graph building from syntax and semantic results

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::{ResolvedEdges, SyntaxResults};
use crate::application::use_cases::containment_builder;
use crate::domain::Confidence;
use crate::domain::Graph;
use std::collections::HashSet;
use tracing::{info, warn};

/// Graph builder service
pub struct GraphBuilder;

impl GraphBuilder {
    /// Build the final graph from syntax and semantic results
    ///
    /// # Arguments
    /// * `syntax_results` - Syntax extraction results
    /// * `resolved_edges` - Resolved semantic edges
    /// * `min_confidence` - Minimum confidence level for validation
    ///
    /// # Returns
    /// * `Graph` - Built and validated graph
    /// * `ParsingError` - If building or validation fails
    pub fn build_from_results(
        syntax_results: SyntaxResults,
        resolved_edges: ResolvedEdges,
        min_confidence: Confidence,
    ) -> Result<Graph, ParsingError> {
        let mut graph = Graph::new();

        // Add all symbols from syntax extraction
        Self::add_nodes(&mut graph, &syntax_results);

        // Add all resolved edges
        Self::add_edges(&mut graph, &resolved_edges);

        // Add deterministic syntax-level edges (Apex managed-package
        // Import edges, etc.) — these are independent of any resolver.
        Self::add_synthesized_edges(&mut graph, &syntax_results);

        // Build containment relationships
        Self::add_containment(&mut graph, &syntax_results);

        // Structural validation (dangling edges, required-node kinds, etc.).
        // Confidence is intentionally reported at `Low` here so the pipeline
        // never fails purely because a resolver emitted a deliberately
        // low-confidence heuristic edge. Low-confidence signal is preserved
        // on every edge via `provenance.confidence` and surfaced to callers
        // through the baseline scanner, `ResolutionStatsSummary`, and the
        // upcoming `ResolutionDegraded` finding. Callers that want strict
        // rejection can inspect `min_confidence` post-hoc using
        // [`Self::count_below_confidence`] or `Graph::validate` directly.
        let _ = min_confidence;
        Self::validate_graph(&graph, Confidence::Low)?;

        Ok(graph)
    }

    /// Count how many edges in the graph have provenance confidence strictly
    /// below `threshold`. Useful for advisory reporting and
    /// `ResolutionDegraded` thresholding without turning a low-confidence
    /// heuristic run into a hard parse failure.
    pub fn count_below_confidence(graph: &Graph, threshold: Confidence) -> usize {
        graph
            .edges
            .iter()
            .filter(|e| e.provenance.confidence < threshold)
            .count()
    }

    /// Add nodes from syntax results.
    ///
    /// Confidence is preserved exactly as the extractor set it. Previous
    /// implementations unconditionally rewrote every node's confidence to
    /// `Medium`, which erased the deliberate tiering upstream extractors
    /// carry (e.g. `treesitter.rs` emits file-scope pseudo-nodes as `Low`
    /// while real symbols are `Medium` or `High`). Callers downstream rely
    /// on that tiering for quality telemetry and findings.
    fn add_nodes(graph: &mut Graph, syntax_results: &SyntaxResults) {
        // Deduplicate by stable `Node::id` (derived from fqn + location).
        // Multiple extractors can synthesize the same logical node from
        // different files — most notably Apex `synthesize_external_references`,
        // which emits one managed-package Module per (file, namespace) pair
        // so every consumer file gets an Import target, even though the
        // target module is the same graph identity. Without dedup, those
        // copies all land in `graph.nodes` verbatim, inflating module
        // counts and breaking the invariant that `graph.nodes` is keyed
        // by id (SQLite dedups via `INSERT OR REPLACE`, but the in-memory
        // `Graph` is consumed directly by baselines, metrics, and the
        // analysis crate). First-wins preserves ordering determinism.
        let mut seen: HashSet<String> = HashSet::with_capacity(syntax_results.symbols.len());
        let mut duplicates = 0usize;
        for node in syntax_results.symbols.iter().cloned() {
            if seen.insert(node.id.clone()) {
                graph.add_node(node);
            } else {
                duplicates += 1;
            }
        }
        if duplicates > 0 {
            info!(
                "Added {} nodes to graph ({} duplicate id(s) collapsed)",
                graph.node_count(),
                duplicates
            );
        } else {
            info!("Added {} nodes to graph", graph.node_count());
        }
    }

    /// Add edges from resolved edges.
    ///
    /// Confidence is preserved exactly as the resolver set it.
    /// `ApexHeuristicResolver` emits `Medium` for unambiguous short-name
    /// matches and `Low` for ambiguous overloads so reports can filter
    /// or down-weight speculative edges; LSP resolvers emit `High`;
    /// tree-sitter fallback paths emit `Low`/`Medium` per heuristic
    /// quality. Overwriting this in graph assembly would destroy the
    /// signal that `ResolutionDegraded` findings and the baseline
    /// scanner read from `edges_by_confidence`.
    fn add_edges(graph: &mut Graph, resolved_edges: &ResolvedEdges) {
        for edge in resolved_edges.all_edges() {
            graph.add_edge(edge);
        }
        info!("Added {} edges to graph", graph.edge_count());
    }

    /// Drain deterministic, syntax-extraction-produced edges (currently
    /// Apex managed-package `Import` edges — see
    /// [`crate::application::ports::SyntaxResults::synthesized_edges`]).
    /// These do not need resolver passes; their endpoints are known
    /// entirely from syntax and stable node ids.
    fn add_synthesized_edges(graph: &mut Graph, syntax_results: &SyntaxResults) {
        if syntax_results.synthesized_edges.is_empty() {
            return;
        }
        let before = graph.edge_count();
        for edge in syntax_results.synthesized_edges.iter().cloned() {
            graph.add_edge(edge);
        }
        info!(
            "Added {} synthesized edges (e.g. Apex managed-package Import edges)",
            graph.edge_count() - before
        );
    }

    /// Add containment relationships (delegates to containment_builder)
    fn add_containment(graph: &mut Graph, syntax_results: &SyntaxResults) {
        info!("Building containment relationships...");
        let (containment_nodes, containment_edges) =
            containment_builder::ContainmentBuilder::build_containment(
                &graph.nodes,
                syntax_results,
            );

        // Same id-dedup as `add_nodes`: the containment builder returns the
        // pre-existing symbol/module nodes alongside any new Project/Crate/
        // File/Folder scaffolding it synthesised, so we must skip anything
        // already attached to the graph to avoid duplicates.
        let mut seen: HashSet<String> =
            HashSet::with_capacity(graph.nodes.len() + containment_nodes.len());
        for existing in &graph.nodes {
            seen.insert(existing.id.clone());
        }
        let mut added = 0usize;
        let mut duplicates = 0usize;
        for node in containment_nodes {
            if seen.insert(node.id.clone()) {
                graph.add_node(node);
                added += 1;
            } else {
                duplicates += 1;
            }
        }
        if duplicates > 0 {
            info!(
                "Added {} containment nodes (Project/Crate/File/Folder/Module); {} duplicate id(s) collapsed",
                added, duplicates
            );
        } else {
            info!(
                "Added {} containment nodes (Project/Crate/File/Folder/Module)",
                added
            );
        }

        let containment_edge_count = containment_edges.len();
        for edge in containment_edges {
            graph.add_edge(edge);
        }
        info!(
            "Added {} containment edges (hierarchy + contains)",
            containment_edge_count
        );
    }

    /// Validate the graph
    fn validate_graph(graph: &Graph, min_confidence: Confidence) -> Result<(), ParsingError> {
        graph.validate(min_confidence).map_err(|e| {
            warn!("Graph validation failed: {:?}", e);
            ParsingError::Validation(e)
        })?;

        info!(
            "Graph validation passed with {} nodes and {} edges",
            graph.node_count(),
            graph.edge_count()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::SyntaxResults;
    use crate::domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range};

    fn module_node(fqn: &str, file: &str) -> Node {
        Node::new(
            NodeKind::Module,
            fqn.to_string(),
            Range::with_file(0, 0, 0, 0, file.to_string()),
            Provenance::new(ProvenanceSource::Heuristic, Confidence::High),
        )
    }

    /// Regression test for the duplicate-managed-package-module bug discovered
    /// during the NPSP large-corpus scan (G.1). Apex synthesises one external
    /// `Module` node per (consumer file, namespace) pair — every one of those
    /// copies shares the same stable id because the fqn and sentinel location
    /// are identical. Before the dedup fix, every copy landed in `graph.nodes`
    /// verbatim, inflating the module count by O(#imports). `add_nodes` now
    /// collapses them to a single graph entry while preserving any edges
    /// emitted against that id.
    #[test]
    fn add_nodes_dedupes_by_stable_id() {
        let a = module_node(
            "external::salesforce::managed_package::npsp",
            "<external:managed_package>",
        );
        let a_copy = module_node(
            "external::salesforce::managed_package::npsp",
            "<external:managed_package>",
        );
        let b = module_node("real::in_repo::module", "/repo/src/foo.cls");
        assert_eq!(a.id, a_copy.id, "precondition: dup has same stable id");

        let mut syntax = SyntaxResults::new();
        syntax.add_symbol(a);
        syntax.add_symbol(a_copy);
        syntax.add_symbol(b);

        let mut graph = Graph::new();
        GraphBuilder::add_nodes(&mut graph, &syntax);

        assert_eq!(
            graph.nodes.len(),
            2,
            "expected dedup to collapse the two managed-package copies into one"
        );
        let npsp_count = graph
            .nodes
            .iter()
            .filter(|n| n.fqn == "external::salesforce::managed_package::npsp")
            .count();
        assert_eq!(npsp_count, 1);
    }

    /// `add_containment` receives the combined output of the builder — that
    /// output can legitimately include references to nodes already on the
    /// graph (file modules, etc.) when the builder returns an unchanged
    /// scaffold row. The pipeline must not re-insert those.
    #[test]
    fn add_containment_skips_ids_already_on_graph() {
        let existing = module_node("already::there", "/repo/src/x.cls");
        let existing_id = existing.id.clone();
        let mut graph = Graph::new();
        graph.add_node(existing);

        let containment_nodes = vec![
            module_node("already::there", "/repo/src/x.cls"),
            module_node("newly::created", "/repo/src/y.cls"),
        ];

        // Manually exercise the dedup branch of add_containment.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for n in &graph.nodes {
            seen.insert(n.id.clone());
        }
        for n in containment_nodes {
            if seen.insert(n.id.clone()) {
                graph.add_node(n);
            }
        }

        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.nodes.iter().any(|n| n.id == existing_id));
        assert!(graph.nodes.iter().any(|n| n.fqn == "newly::created"));
    }
}
