//! Per-metric status assignment.
//!
//! The `HealthReport` data contract treats every metric as self-describing.
//! A raw numeric value is only meaningful when it is accompanied by a
//! `MetricStatus` that tells the consumer whether the underlying graph had
//! enough signal for the number to be interpretable. See
//! [`crate::health::report::MetricStatus`] for the enum.
//!
//! This module concentrates all status-assignment logic in one place so
//! there is a single source of truth for "under what conditions does
//! `cycles_found = 0` mean 'no cycles' vs 'graph too sparse'?". Historically
//! every consumer of the report re-implemented this check (or, more often,
//! didn't), which is how the cycle-ordering bug shipped invisibly.

use super::config::ThresholdConfig;
use super::graph::AnalysisGraph;
use super::report::{MetricStatus, ResolutionQuality, ResolutionTier};

/// Inputs to status assignment. Collected at the end of the pipeline so
/// that all metric statuses are computed from the same consistent view of
/// the graph.
pub struct StatusInputs<'a> {
    pub graph: &'a AnalysisGraph,
    pub thresholds: &'a ThresholdConfig,
    pub ecosystem: Ecosystem,
    pub resolution: Option<&'a ResolutionQuality>,
    pub cycles_computation_ok: bool,
    pub coupling_computation_ok: bool,
    pub depth_computation_ok: bool,
    pub dead_code_computation_ok: bool,
}

/// Minimal ecosystem facade for status logic. We can't depend on
/// `config::Ecosystem` directly without a circular coupling; this mirrors
/// the subset we need and is populated by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    DeclarativeHeavy, // Apex, Django/Python, Java/Spring, Rails, C# MVC
    Standard,         // TypeScript/JavaScript, Rust, Go
    Unknown,
}

impl Ecosystem {
    /// Map from the main config enum. Kept here so callers can pass in the
    /// `config::Ecosystem` they already have without this module depending
    /// on the entire config module graph.
    pub fn classify(eco: super::config::Ecosystem) -> Self {
        use super::config::Ecosystem as E;
        match eco {
            // Languages whose frameworks commonly rely on declarative
            // wiring / reflection-dispatch that static parsing misses.
            // Apex is the canonical example (TDTM + @AuraEnabled), Python
            // because Django/Flask routing is string-based, Java because
            // Spring uses @Component / XML wiring, C# because ASP.NET MVC
            // and attribute-driven DI are pervasive.
            E::Java | E::CSharp | E::Python | E::Apex => Self::DeclarativeHeavy,
            E::Rust | E::Go | E::TypeScript | E::JavaScript => Self::Standard,
            E::Unknown => Self::Unknown,
        }
    }
}

/// Compute the status for the cycles / tangle metric pair. They share a
/// status because tangle_index is derived from cycle membership and would
/// mislead in exactly the same way when the underlying edge set is
/// insufficient.
pub fn cycles_status(inp: &StatusInputs) -> MetricStatus {
    if !inp.cycles_computation_ok {
        return MetricStatus::ComputationFailed;
    }

    let prod_edges = inp.graph.production_structural_edge_indices.len();
    if prod_edges < inp.thresholds.min_edges_for_cycle_metric {
        return MetricStatus::InsufficientEdges;
    }

    if is_framework_invisible(inp) {
        return MetricStatus::FrameworkInvisible;
    }

    MetricStatus::Ok
}

/// Tangle shares cycles' status — same graph, same assumptions.
pub fn tangle_status(inp: &StatusInputs) -> MetricStatus {
    cycles_status(inp)
}

/// Max call depth depends on Call-kind production edges specifically.
/// A graph with lots of Extends/Import edges but few Calls will still
/// produce a depth of 0 even though it has real structure.
pub fn depth_status(inp: &StatusInputs) -> MetricStatus {
    if !inp.depth_computation_ok {
        return MetricStatus::ComputationFailed;
    }

    // `depth` is a call-semantic metric whose threshold is phrased as
    // "minimum call edges" — include framework and declarative dispatch
    // in that count. See DISCOVERY_REPORT.md §8 Decision 5. Pre-T1 this
    // was `== EdgeKind::Call` literally, which under-counted on any
    // Salesforce repo with VF bindings.
    let call_edges = inp
        .graph
        .production_structural_edge_indices
        .iter()
        .filter(|&&ei| inp.graph.edges[ei].kind.is_call_like())
        .count();

    if call_edges < inp.thresholds.min_call_edges_for_depth_metric {
        return MetricStatus::InsufficientEdges;
    }

    if is_framework_invisible(inp) {
        return MetricStatus::FrameworkInvisible;
    }

    MetricStatus::Ok
}

/// Dead-code is status-sensitive to framework invisibility but not to
/// graph sparsity — a legitimately tiny graph can still have legitimate
/// dead code. The only concern is that framework-routed entry points
/// (Django views, Apex @AuraEnabled methods) look dead without resolver
/// support.
pub fn dead_code_status(inp: &StatusInputs) -> MetricStatus {
    if !inp.dead_code_computation_ok {
        return MetricStatus::ComputationFailed;
    }
    if is_framework_invisible(inp) {
        return MetricStatus::FrameworkInvisible;
    }
    MetricStatus::Ok
}

/// Coupling is sensitive to module density. Repos with few modules produce
/// non-representative averages regardless of framework.
pub fn coupling_status(inp: &StatusInputs, modules_measured: usize) -> MetricStatus {
    if !inp.coupling_computation_ok {
        return MetricStatus::ComputationFailed;
    }
    // 3 is an absolute floor: you cannot reason about "inter-module coupling"
    // with fewer than three modules. This is an architectural constant, not
    // a tunable threshold.
    if modules_measured < 3 {
        return MetricStatus::InsufficientEdges;
    }
    if is_framework_invisible(inp) {
        return MetricStatus::FrameworkInvisible;
    }
    MetricStatus::Ok
}

/// Shared predicate: true when the codebase is declarative-framework-heavy
/// AND a significant fraction of resolution came from heuristic fallback
/// (no LSP / low-confidence import matching). Under these conditions, the
/// parsed call graph is almost certainly missing the dispatch edges that
/// the framework routes through at runtime.
fn is_framework_invisible(inp: &StatusInputs) -> bool {
    if inp.ecosystem != Ecosystem::DeclarativeHeavy {
        return false;
    }

    let Some(res) = inp.resolution else {
        // No resolution report — can't judge quality. Assume Ok rather than
        // surfacing a warning we can't substantiate.
        return false;
    };

    match res.resolution_tier {
        ResolutionTier::None | ResolutionTier::HeuristicOnly => true,
        ResolutionTier::Full => {
            // Even with LSP, if the import_edges count is zero we have no
            // cross-file resolution at all. Unlikely but not impossible.
            res.import_edges_total == 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_graph() -> AnalysisGraph {
        AnalysisGraph::build(Default::default(), Default::default())
    }

    #[test]
    fn insufficient_edges_triggers_on_small_graph() {
        let g = empty_graph();
        let cfg = ThresholdConfig::default();
        let inp = StatusInputs {
            graph: &g,
            thresholds: &cfg,
            ecosystem: Ecosystem::Standard,
            resolution: None,
            cycles_computation_ok: true,
            coupling_computation_ok: true,
            depth_computation_ok: true,
            dead_code_computation_ok: true,
        };
        assert_eq!(cycles_status(&inp), MetricStatus::InsufficientEdges);
        assert_eq!(depth_status(&inp), MetricStatus::InsufficientEdges);
    }

    #[test]
    fn framework_invisible_gates_on_resolution() {
        let g = empty_graph();
        let cfg = ThresholdConfig::default();
        let res = ResolutionQuality {
            import_edges_total: 10,
            resolution_tier: ResolutionTier::HeuristicOnly,
            measured_fidelity: crate::health::report::MeasuredFidelity {
                tier: crate::health::report::MeasuredFidelityTier::Unknown,
                high_ratio_on_calls: None,
                call_edges_by_confidence: Default::default(),
                all_edges_by_confidence: Default::default(),
            },
            recommendation: None,
        };
        let inp = StatusInputs {
            graph: &g,
            thresholds: &cfg,
            ecosystem: Ecosystem::DeclarativeHeavy,
            resolution: Some(&res),
            cycles_computation_ok: true,
            coupling_computation_ok: true,
            depth_computation_ok: true,
            dead_code_computation_ok: true,
        };
        // Empty graph → InsufficientEdges wins over FrameworkInvisible
        // because edge sparsity is the more specific diagnostic.
        assert_eq!(cycles_status(&inp), MetricStatus::InsufficientEdges);
        // Dead-code doesn't have an edge-count floor, so FrameworkInvisible
        // surfaces.
        assert_eq!(dead_code_status(&inp), MetricStatus::FrameworkInvisible);
    }

    #[test]
    fn computation_failed_is_sticky() {
        let g = empty_graph();
        let cfg = ThresholdConfig::default();
        let inp = StatusInputs {
            graph: &g,
            thresholds: &cfg,
            ecosystem: Ecosystem::Standard,
            resolution: None,
            cycles_computation_ok: false,
            coupling_computation_ok: false,
            depth_computation_ok: false,
            dead_code_computation_ok: false,
        };
        assert_eq!(cycles_status(&inp), MetricStatus::ComputationFailed);
        assert_eq!(depth_status(&inp), MetricStatus::ComputationFailed);
        assert_eq!(dead_code_status(&inp), MetricStatus::ComputationFailed);
        assert_eq!(coupling_status(&inp, 100), MetricStatus::ComputationFailed);
    }
}
