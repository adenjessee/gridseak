//! API surface ratio per module.
//!
//! Measures encapsulation: what fraction of a module's functions are called from outside.
//! api_surface = exported_functions / total_functions

use std::collections::HashMap;

use super::graph::{is_synthetic_node, AnalysisGraph};

#[derive(Debug, Clone)]
pub struct ApiSurfaceMetrics {
    pub api_surface_ratio: f64,
    pub exported_functions: usize,
    pub total_functions: usize,
}

pub fn compute_api_surface(graph: &AnalysisGraph) -> HashMap<String, ApiSurfaceMetrics> {
    let mut results = HashMap::new();

    for module_key in &graph.folder_module_ids {
        let members = match graph.analysis_module_members_of(module_key) {
            Some(m) => m,
            None => continue,
        };

        let fn_ids: Vec<&str> = members
            .iter()
            .filter(|d| {
                graph
                    .nodes
                    .get(d.as_str())
                    .map(|n| n.kind.is_function_like() && !is_synthetic_node(n))
                    .unwrap_or(false)
            })
            .map(|s| s.as_str())
            .collect();

        let total = fn_ids.len();
        if total == 0 {
            continue;
        }

        let mut exported = 0usize;
        for &fid in &fn_ids {
            let has_external_caller = graph
                .structural_incoming(fid)
                .iter()
                .any(|e| !members.contains(&e.from_id));

            if has_external_caller {
                exported += 1;
            }
        }

        let ratio = exported as f64 / total as f64;

        results.insert(
            module_key.clone(),
            ApiSurfaceMetrics {
                api_surface_ratio: ratio,
                exported_functions: exported,
                total_functions: total,
            },
        );
    }

    results
}
