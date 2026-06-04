//! Robert C. Martin's Abstractness metric per module.
//!
//! A = Na / Nc where Na = number of abstract types (interfaces, traits, abstract classes)
//! and Nc = total number of types (structs, classes, interfaces, enums) in the module.
//! A value of 0.0 means fully concrete, 1.0 means fully abstract.

use std::collections::HashMap;

use super::graph::{AnalysisGraph, NodeKind};

#[derive(Debug, Clone)]
pub struct ModuleAbstractness {
    pub abstractness: f64,
    pub abstract_types: usize,
    pub total_types: usize,
}

pub fn compute_abstractness(graph: &AnalysisGraph) -> HashMap<String, ModuleAbstractness> {
    let mut results = HashMap::new();

    for module_key in &graph.folder_module_ids {
        let members = match graph.analysis_module_members_of(module_key) {
            Some(m) => m,
            None => continue,
        };

        let mut abstract_count = 0usize;
        let mut total_types = 0usize;

        for member_id in members {
            let node = match graph.nodes.get(member_id.as_str()) {
                Some(n) => n,
                None => continue,
            };

            match node.kind {
                NodeKind::Interface => {
                    total_types += 1;
                    abstract_count += 1;
                }
                NodeKind::Struct | NodeKind::Enum | NodeKind::Type => {
                    total_types += 1;
                }
                _ => {}
            }
        }

        let abstractness = if total_types == 0 {
            0.0
        } else {
            abstract_count as f64 / total_types as f64
        };

        results.insert(
            module_key.clone(),
            ModuleAbstractness {
                abstractness,
                abstract_types: abstract_count,
                total_types,
            },
        );
    }

    results
}
