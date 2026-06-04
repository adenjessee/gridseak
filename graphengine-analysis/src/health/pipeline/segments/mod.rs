//! Analysis segment identifiers and callable segment runners (S2-γ).

mod auxiliary_metrics;
mod blast_radius;
mod complexity;
mod cycles;
mod dead_code;
mod fan_metrics;
mod findings_assembly;
mod graph_prep;
mod health_score;
mod module_coupling;

pub use auxiliary_metrics::run as run_auxiliary_metrics;
pub use blast_radius::run as run_blast_radius;
pub use complexity::run as run_complexity;
pub use cycles::run as run_cycles;
pub use dead_code::run as run_dead_code;
pub use fan_metrics::run as run_fan_metrics;
pub use findings_assembly::run as run_findings_assembly;
pub use graph_prep::run as run_graph_prep;
pub use health_score::run as run_health_score;
pub use module_coupling::run as run_module_coupling;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisSegment {
    GraphPrep,
    Cycles,
    FanMetrics,
    ModuleCoupling,
    DeadCode,
    BlastRadius,
    Complexity,
    AuxiliaryMetrics,
    FindingsAssembly,
    HealthScore,
}

impl AnalysisSegment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GraphPrep => "GraphPrep",
            Self::Cycles => "Cycles",
            Self::FanMetrics => "FanMetrics",
            Self::ModuleCoupling => "ModuleCoupling",
            Self::DeadCode => "DeadCode",
            Self::BlastRadius => "BlastRadius",
            Self::Complexity => "Complexity",
            Self::AuxiliaryMetrics => "AuxiliaryMetrics",
            Self::FindingsAssembly => "FindingsAssembly",
            Self::HealthScore => "HealthScore",
        }
    }

    pub fn parallelizable(self) -> bool {
        matches!(self, Self::Complexity)
    }

    pub fn run(
        self,
        ctx: &mut super::session::AnalysisRunContext<'_>,
    ) -> anyhow::Result<Option<super::super::report::HealthReport>> {
        match self {
            Self::GraphPrep => run_graph_prep(ctx),
            Self::Cycles => run_cycles(ctx),
            Self::FanMetrics => run_fan_metrics(ctx),
            Self::ModuleCoupling => run_module_coupling(ctx),
            Self::DeadCode => run_dead_code(ctx),
            Self::BlastRadius => run_blast_radius(ctx),
            Self::Complexity => run_complexity(ctx),
            Self::AuxiliaryMetrics => run_auxiliary_metrics(ctx),
            Self::FindingsAssembly => run_findings_assembly(ctx),
            Self::HealthScore => run_health_score(ctx),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisMode {
    ZeroReuse,
    SegmentedSync,
    Full,
    Background,
}

pub fn all_segment_ids() -> Vec<AnalysisSegment> {
    vec![
        AnalysisSegment::GraphPrep,
        AnalysisSegment::Cycles,
        AnalysisSegment::FanMetrics,
        AnalysisSegment::ModuleCoupling,
        AnalysisSegment::DeadCode,
        AnalysisSegment::BlastRadius,
        AnalysisSegment::Complexity,
        AnalysisSegment::AuxiliaryMetrics,
        AnalysisSegment::FindingsAssembly,
        AnalysisSegment::HealthScore,
    ]
}
