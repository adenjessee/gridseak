//! Robert C. Martin's Instability metric per module.
//!
//! I = Ce / (Ca + Ce) where Ca = afferent (incoming) module-level coupling,
//! Ce = efferent (outgoing) module-level coupling.

use std::collections::{HashMap, HashSet};

use super::graph::AnalysisGraph;

#[derive(Debug, Clone)]
pub struct ModuleInstability {
    pub instability: f64,
    pub afferent_coupling: usize,
    pub efferent_coupling: usize,
}

pub fn compute_instability(graph: &AnalysisGraph) -> HashMap<String, ModuleInstability> {
    compute_instability_impl(graph, false)
}

/// High-confidence-only variant used by T3 dual-metric emission.
/// Only `Confidence::High` edges contribute to afferent/efferent
/// module coupling counts. Modules whose fan-out was inflated by
/// heuristic call edges will see their instability drop.
pub fn compute_instability_high_only(graph: &AnalysisGraph) -> HashMap<String, ModuleInstability> {
    compute_instability_impl(graph, true)
}

fn compute_instability_impl(
    graph: &AnalysisGraph,
    high_only: bool,
) -> HashMap<String, ModuleInstability> {
    let mut results = HashMap::new();

    for module_key in &graph.folder_module_ids {
        let members = match graph.analysis_module_members_of(module_key) {
            Some(m) => m,
            None => continue,
        };

        let mut ca_modules: HashSet<String> = HashSet::new();
        let mut ce_modules: HashSet<String> = HashSet::new();

        for &ei in &graph.structural_edge_indices {
            let edge = &graph.edges[ei];
            if high_only && !edge.confidence.is_high() {
                continue;
            }
            let from_in = members.contains(&edge.from_id);
            let to_in = members.contains(&edge.to_id);

            if from_in && !to_in {
                if let Some(target_module) = graph.folder_of(&edge.to_id) {
                    if target_module != module_key {
                        ce_modules.insert(target_module.clone());
                    }
                }
            } else if !from_in && to_in {
                if let Some(source_module) = graph.folder_of(&edge.from_id) {
                    if source_module != module_key {
                        ca_modules.insert(source_module.clone());
                    }
                }
            }
        }

        let ca = ca_modules.len();
        let ce = ce_modules.len();
        let total = ca + ce;

        let instability = if total == 0 {
            0.0
        } else {
            ce as f64 / total as f64
        };

        results.insert(
            module_key.clone(),
            ModuleInstability {
                instability,
                afferent_coupling: ca,
                efferent_coupling: ce,
            },
        );
    }

    results
}
