//! Shared analysis run context (S2-γ segment extraction).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use rusqlite::Connection;

use crate::validation::overrides::ValidationOverrides;

use super::super::abstractness::ModuleAbstractness;
use super::super::api_surface::ApiSurfaceMetrics;
use super::super::blast_radius::BlastRadiusResult;
use super::super::cohesion::CohesionResult;
use super::super::complexity::ComplexityResult;
use super::super::config::{AnalysisConfig, DeadCodeConfig};
use super::super::coupling::CouplingResult;
use super::super::cycles::CycleResult;
use super::super::dead_code::DeadCodeResult;
use super::super::dead_code_classifier;
use super::super::depth::DepthResult;
use super::super::distance_from_main_sequence::ModuleDistance;
use super::super::fan_metrics::FanResult;
use super::super::graph::AnalysisGraph;
use super::super::hub_score::HubMetrics;
use super::super::information_flow::IfcResult;
use super::super::instability::ModuleInstability;
use super::super::layers::LayerResult;
use super::super::repo_classification::RepoType;
use super::super::report::{
    AnalysisError, EdgesByConfidence, Finding, HealthReport, MeasuredFidelity,
    MeasuredFidelityTier, ModuleAnnotation, NodeAnnotation, ResolutionQuality, ResolutionTier,
};
use super::super::temporal_coupling::TemporalCouplingResult;

/// Mutable state threaded through segment runners during a full or partial analysis.
pub struct AnalysisRunContext<'a> {
    pub conn: Connection,
    pub start: Instant,
    pub db_path: String,
    pub norms_path: Option<&'a str>,
    pub git_dir: Option<&'a str>,
    pub overrides: Option<&'a ValidationOverrides>,
    pub bootstrap_config: Option<AnalysisConfig>,
    pub config: AnalysisConfig,
    pub graph: AnalysisGraph,
    pub stale_parse_db: bool,
    pub resolution_quality: ResolutionQuality,
    pub override_entry_point_ids: HashSet<String>,
    pub findings: Vec<Finding>,
    pub analysis_errors: Vec<AnalysisError>,
    pub node_annotations: BTreeMap<String, NodeAnnotation>,
    pub module_annotations: BTreeMap<String, ModuleAnnotation>,
    pub cycle_result: Option<CycleResult>,
    pub cycle_node_set: HashSet<String>,
    pub total_cycle_nodes: usize,
    pub edges_in_cycles: usize,
    pub fan_result: Option<FanResult>,
    pub coupling_result: Option<CouplingResult>,
    pub repo_type: RepoType,
    pub dc_cfg: DeadCodeConfig,
    pub dead_result: Option<DeadCodeResult>,
    pub dead_code_reason_breakdown: BTreeMap<String, usize>,
    pub dead_code_verdicts_all: Vec<dead_code_classifier::DeadCodeVerdict>,
    pub blast_result: Option<BlastRadiusResult>,
    pub depth_result: Option<DepthResult>,
    pub layer_result: Option<LayerResult>,
    pub complexity_result: Option<ComplexityResult>,
    pub fn_loc: HashMap<String, usize>,
    pub cohesion_result: Option<CohesionResult>,
    pub instability_result: Option<HashMap<String, ModuleInstability>>,
    pub abstractness_result: Option<HashMap<String, ModuleAbstractness>>,
    pub distance_result: Option<BTreeMap<String, ModuleDistance>>,
    pub tangle_idx: f64,
    pub ifc_result: Option<IfcResult>,
    pub hub_result: Option<HashMap<String, HubMetrics>>,
    pub api_result: Option<HashMap<String, ApiSurfaceMetrics>>,
    pub temporal_result: Option<TemporalCouplingResult>,
    pub report: Option<HealthReport>,
}

impl<'a> AnalysisRunContext<'a> {
    pub fn new(
        db_path: String,
        bootstrap_config: Option<AnalysisConfig>,
        norms_path: Option<&'a str>,
        git_dir: Option<&'a str>,
        overrides: Option<&'a ValidationOverrides>,
    ) -> Self {
        Self {
            conn: Connection::open_in_memory().expect("in-memory sqlite for bootstrap"),
            start: Instant::now(),
            db_path,
            norms_path,
            git_dir,
            overrides,
            bootstrap_config,
            config: AnalysisConfig::default(),
            graph: AnalysisGraph::empty(),
            stale_parse_db: false,
            resolution_quality: ResolutionQuality {
                import_edges_total: 0,
                resolution_tier: ResolutionTier::None,
                measured_fidelity: MeasuredFidelity {
                    tier: MeasuredFidelityTier::Unknown,
                    high_ratio_on_calls: None,
                    call_edges_by_confidence: EdgesByConfidence::default(),
                    all_edges_by_confidence: EdgesByConfidence::default(),
                },
                recommendation: None,
            },
            override_entry_point_ids: HashSet::new(),
            findings: Vec::new(),
            analysis_errors: Vec::new(),
            node_annotations: BTreeMap::new(),
            module_annotations: BTreeMap::new(),
            cycle_result: None,
            cycle_node_set: HashSet::new(),
            total_cycle_nodes: 0,
            edges_in_cycles: 0,
            fan_result: None,
            coupling_result: None,
            repo_type: RepoType::Application,
            dc_cfg: DeadCodeConfig::default(),
            dead_result: None,
            dead_code_reason_breakdown: dead_code_classifier::empty_reason_breakdown(),
            dead_code_verdicts_all: Vec::new(),
            blast_result: None,
            depth_result: None,
            layer_result: None,
            complexity_result: None,
            fn_loc: HashMap::new(),
            cohesion_result: None,
            instability_result: None,
            abstractness_result: None,
            distance_result: None,
            tangle_idx: 0.0,
            ifc_result: None,
            hub_result: None,
            api_result: None,
            temporal_result: None,
            report: None,
        }
    }
}
