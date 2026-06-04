//! Hub score (dependency centrality).
//!
//! Identifies function nodes that bridge multiple distinct modules.
//! hub_score = (distinct_source_modules × distinct_target_modules) / total_modules²

use std::collections::{HashMap, HashSet};

use super::graph::AnalysisGraph;

#[derive(Debug, Clone)]
pub struct HubMetrics {
    pub hub_score: f64,
    pub source_modules: usize,
    pub target_modules: usize,
}

pub fn compute_hub_scores(graph: &AnalysisGraph) -> HashMap<String, HubMetrics> {
    let total_folders = graph.total_folder_modules();
    let total_sq = if total_folders > 0 {
        (total_folders * total_folders) as f64
    } else {
        1.0
    };

    let mut results = HashMap::new();

    for id in &graph.function_node_ids {
        let mut source_modules: HashSet<String> = HashSet::new();
        let mut target_modules: HashSet<String> = HashSet::new();

        for edge in graph.structural_incoming(id) {
            if let Some(folder_id) = graph.folder_of(&edge.from_id) {
                source_modules.insert(folder_id.clone());
            }
        }

        for edge in graph.structural_outgoing(id) {
            if let Some(folder_id) = graph.folder_of(&edge.to_id) {
                target_modules.insert(folder_id.clone());
            }
        }

        if let Some(own_folder) = graph.folder_of(id) {
            source_modules.remove(own_folder);
            target_modules.remove(own_folder);
        }

        let src_count = source_modules.len();
        let tgt_count = target_modules.len();
        let score = (src_count * tgt_count) as f64 / total_sq;

        results.insert(
            id.clone(),
            HubMetrics {
                hub_score: score,
                source_modules: src_count,
                target_modules: tgt_count,
            },
        );
    }

    results
}
