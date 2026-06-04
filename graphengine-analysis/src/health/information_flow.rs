//! Henry & Kafura Information Flow Complexity (IFC).
//!
//! IFC = (fan_in × fan_out)² per function node.
//! Catches information flow bottlenecks that neither fan-in nor fan-out alone reveals.

use std::collections::HashMap;

use super::fan_metrics::FanResult;
use super::graph::AnalysisGraph;

#[derive(Debug)]
pub struct IfcResult {
    pub scores: HashMap<String, usize>,
}

pub fn compute_ifc(graph: &AnalysisGraph, fan: &FanResult) -> IfcResult {
    let mut scores = HashMap::new();

    for id in &graph.function_node_ids {
        if let Some(m) = fan.metrics.get(id) {
            let product = m.fan_in * m.fan_out;
            let ifc = product * product; // (fan_in * fan_out)²
            scores.insert(id.clone(), ifc);
        }
    }

    IfcResult { scores }
}
