//! Structural health analysis orchestrator.
//!
//! Calls each analysis algorithm independently, collects results, merges annotations,
//! computes the composite health score, and builds the final JSON report.
//! Individual algorithm failures are caught and reported without crashing the pipeline.

pub mod abstractness;
pub mod api_surface;
pub mod blast_radius;
pub mod cohesion;
pub mod complexity;
pub mod config;
pub mod coupling;
pub mod coverage_attach;
pub mod cycles;
pub mod dead_code;
pub mod dead_code_classifier;
pub mod dead_code_confidence;
pub mod depth;
pub mod distance_from_main_sequence;
pub mod entry_points;
pub mod fan_metrics;
pub mod git_signals_attach;
pub mod graph;
pub mod health_score;
pub mod hub_score;
pub mod information_flow;
pub mod instability;
pub mod layers;
pub mod loc;
pub mod managed_package_coupling_concentration;
pub mod metric_status;
pub mod multiple_triggers_per_sobject;
pub mod norms;
pub mod path_classification;
pub mod progress;
pub mod repo_classification;
pub mod report;
pub mod resolution_degraded;
pub mod structural_classification;
pub mod tangle;
pub mod temporal_coupling;
pub mod test_framework_registry;

pub mod incremental_fast_path;
pub mod pipeline;

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use rusqlite::Connection;

use config::AnalysisConfig;
use graph::AnalysisGraph;
use report::*;

use crate::validation::overrides::ValidationOverrides;

/// Run the full analysis pipeline on a SQLite database with default config.
pub fn run_analysis(db_path: &str) -> Result<HealthReport> {
    run_analysis_with_config(db_path, None, None, None, None)
}

/// Run the full analysis pipeline on a SQLite database with an explicit config.
/// If `config` is None, the ecosystem is auto-detected from the database and
/// the corresponding profile defaults are used.
/// If `norms_path` is Some, percentile rankings are computed against the population DB.
/// If `git_dir` is Some, temporal coupling analysis is run against the git history.
/// If `overrides` is Some, user corrections are applied before analysis begins.
pub fn run_analysis_with_config(
    db_path: &str,
    config: Option<AnalysisConfig>,
    norms_path: Option<&str>,
    git_dir: Option<&str>,
    overrides: Option<&ValidationOverrides>,
) -> Result<HealthReport> {
    pipeline::full::run_full(db_path, config, norms_path, git_dir, overrides, None)
}

/// Open the graph DB read-write and update the Project node's
/// `properties.language` to `lang`. Idempotent: if the property is
/// already equal we still rewrite the row (cheaper than reading,
/// parsing, comparing, then rewriting on miss).
///
/// Kept separate from `run_analysis_with_config` so the main path
/// can stay on a read-only handle (`SQLITE_OPEN_READ_ONLY`) and only
/// pay the read-write open cost when we actually have something to
/// write back.
pub(crate) fn persist_project_language(db_path: &str, lang: &str) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let project_row: Option<(String, String)> = conn
        .query_row(
            "SELECT id, properties FROM nodes WHERE kind = 'Project' LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok();
    let Some((project_id, properties_raw)) = project_row else {
        // No Project node — nothing to write. This is a non-error case
        // for very small fixtures used in unit tests.
        return Ok(());
    };
    let mut properties: serde_json::Value =
        serde_json::from_str(&properties_raw).unwrap_or_else(|_| serde_json::json!({}));
    if !properties.is_object() {
        properties = serde_json::json!({});
    }
    properties["language"] = serde_json::Value::String(lang.to_string());
    let serialised = serde_json::to_string(&properties).unwrap_or_else(|_| "{}".into());
    conn.execute(
        "UPDATE nodes SET properties = ?1 WHERE id = ?2",
        rusqlite::params![serialised, project_id],
    )?;
    // The parser opens the scratch DB in WAL journal mode (the
    // sqlite_repository default) so the UPDATE above lands in
    // `<db_path>-wal` rather than the main file. `complete_scan`
    // copies only `<db_path>` to the durable graphs directory, so
    // without a checkpoint the canonical language would silently
    // vanish from the durable copy even though the in-memory
    // HealthReport and `scan_runs.primary_language` both have it.
    //
    // `wal_checkpoint(TRUNCATE)` flushes any pending WAL frames into
    // the main file and resets the WAL to zero length — the
    // strongest variant, equivalent to a quiesced database. We
    // tolerate failure (read-only mount, exotic locking) with the
    // same best-effort posture as the rest of this function: the
    // HealthReport still carries the canonical value, and downstream
    // consumers that prefer the report over the graph DB stay
    // correct.
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: run an algorithm safely, catching panics
// ---------------------------------------------------------------------------

pub(crate) fn run_safe<T, F: FnOnce() -> T + std::panic::UnwindSafe>(
    name: &str,
    errors: &mut Vec<AnalysisError>,
    f: F,
) -> Option<T> {
    match std::panic::catch_unwind(f) {
        Ok(result) => Some(result),
        Err(_) => {
            errors.push(AnalysisError {
                algorithm: name.into(),
                error: format!("Panicked during {name}"),
                nodes_affected: None,
            });
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Finding builders
// ---------------------------------------------------------------------------

pub(crate) fn build_cycle_finding(
    cycle: &cycles::Cycle,
    graph: &AnalysisGraph,
    thresholds: &config::ThresholdConfig,
) -> Finding {
    let names: Vec<String> = cycle
        .node_ids
        .iter()
        .filter_map(|id| graph.nodes.get(id).map(|n| n.display_name()))
        .collect();

    let description = if names.is_empty() {
        format!("Cycle of {} nodes", cycle.node_ids.len())
    } else {
        let display_names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut desc = display_names.join(" → ");
        if let Some(first) = display_names.first() {
            desc.push_str(" → ");
            desc.push_str(first);
        }
        desc
    };

    let len = cycle.node_ids.len();
    let severity = if len >= thresholds.cycle_critical_length {
        Severity::Critical
    } else if len >= thresholds.cycle_high_length {
        Severity::High
    } else {
        Severity::Warning
    };

    Finding {
        id: cycle.id.clone(),
        finding_type: FindingType::CircularDependency,
        severity,
        description,
        detail: Some(format!(
            "{len}-node circular dependency chain"
        )),
        node_ids: cycle.node_ids.clone(),
        edge_ids: None,
        primary_node_id: cycle.node_ids.first().cloned(),
        metric_name: Some("cycle_length".into()),
        metric_value: Some(len as f64),
        impact: None,
        blast_radius: None,
        recommendation: Some(
            "Break the cycle by extracting shared dependencies into a separate module, or invert one dependency direction using dependency injection."
                .into(),
        ),
        cycle_length: Some(len),
        fan_in: None,
        coupling_score: None,
        internal_edges: None,
        external_edges: None,
        count: None,
        hub_score: None,
        file_a: None,
        file_b: None,
        co_change_count: None,
        temporal_coupling_score: None,
        has_import_edge: None,
        confidence: None,
    }
}

pub(crate) fn build_coupling_finding(
    module_key: &str,
    mc: &coupling::ModuleCoupling,
    thresholds: &config::ThresholdConfig,
) -> Finding {
    let severity = if mc.coupling_score > thresholds.coupling_critical {
        Severity::Critical
    } else if mc.coupling_score > thresholds.coupling_high {
        Severity::High
    } else if mc.coupling_score > thresholds.coupling_warning {
        Severity::Warning
    } else {
        Severity::Info
    };

    Finding {
        id: format!("coupling-{module_key}"),
        finding_type: FindingType::HighCoupling,
        severity,
        description: format!(
            "{module_key}: coupling {:.2} ({} external vs {} internal edges)",
            mc.coupling_score, mc.external_edges, mc.internal_edges
        ),
        detail: Some(
            "High coupling means a module depends heavily on external modules relative to its internal structure. \
             Changes to this module or its dependencies propagate widely, increasing change risk."
                .into(),
        ),
        node_ids: vec![module_key.to_string()],
        edge_ids: None,
        primary_node_id: Some(module_key.to_string()),
        metric_name: Some("coupling_score".into()),
        metric_value: Some(mc.coupling_score),
        impact: None,
        blast_radius: None,
        recommendation: Some(
            "Reduce external dependencies by accepting interfaces instead of importing concrete implementations."
                .into(),
        ),
        cycle_length: None,
        fan_in: None,
        coupling_score: Some(mc.coupling_score),
        internal_edges: Some(mc.internal_edges),
        external_edges: Some(mc.external_edges),
        count: None,
        hub_score: None,
        file_a: None,
        file_b: None,
        co_change_count: None,
        temporal_coupling_score: None,
        has_import_edge: None,
        confidence: None,
    }
}

pub(crate) fn build_hotspot_findings(
    graph: &AnalysisGraph,
    fan: &fan_metrics::FanResult,
    blast: &blast_radius::BlastRadiusResult,
) -> Vec<Finding> {
    let mut results = Vec::new();
    let mut idx = 0;

    let mut hotspot_entries: Vec<_> = fan.metrics.iter().filter(|(_, m)| m.is_hotspot).collect();
    hotspot_entries.sort_by(|a, b| b.1.fan_in.cmp(&a.1.fan_in).then_with(|| a.0.cmp(b.0)));

    for (id, fm) in hotspot_entries {
        if graph.is_non_production_node(id) {
            continue;
        }
        idx += 1;
        let radius = blast.radii.get(id).copied().unwrap_or(0);
        let name = graph
            .nodes
            .get(id.as_str())
            .map(|n| n.display_name())
            .unwrap_or_else(|| "unknown".to_string());

        let severity = if radius > 0 && fm.fan_in > 0 {
            Severity::High
        } else {
            Severity::Warning
        };

        results.push(Finding {
            id: format!("hotspot-{idx}"),
            finding_type: FindingType::BlastRadiusHotspot,
            severity,
            description: format!(
                "{name}: called by {} paths, affects {} nodes",
                fm.fan_in, radius
            ),
            detail: Some(
                "Hotspots are functions in the top percentile of fan-in that also have high blast radius. \
                 They are critical dependency points — changes here ripple through many downstream nodes."
                    .into(),
            ),
            node_ids: vec![id.clone()],
            edge_ids: None,
            primary_node_id: Some(id.clone()),
            metric_name: Some("fan_in".into()),
            metric_value: Some(fm.fan_in as f64),
            impact: Some(format!("{radius} downstream nodes affected")),
            blast_radius: Some(radius),
            recommendation: Some(
                "Monitor for growth. If this function changes frequently, consider breaking it into smaller units."
                    .into(),
            ),
            cycle_length: None,
            fan_in: Some(fm.fan_in),
            coupling_score: None,
            internal_edges: None,
            external_edges: None,
            count: None,
            hub_score: None,
            file_a: None,
            file_b: None,
            co_change_count: None,
            temporal_coupling_score: None,
            has_import_edge: None,
            confidence: None,
        });
    }

    results
}

// ---------------------------------------------------------------------------
// Risk level assignment
// ---------------------------------------------------------------------------

/// When the graph has zero Import edges, cross-file-dependent findings cannot
/// Tag finding confidence based on the resolution tier:
/// - Full (LSP): all cross-file findings get High confidence
/// - HeuristicOnly: cross-file findings get Medium confidence
/// - None: cross-file findings get Low confidence
pub(crate) fn tag_confidence_from_resolution(findings: &mut [Finding], rq: &ResolutionQuality) {
    use report::Confidence;

    const CROSS_FILE_DEPENDENT_TYPES: &[FindingType] = &[
        FindingType::PotentiallyUnreachable,
        FindingType::HighCoupling,
        FindingType::BlastRadiusHotspot,
        FindingType::InformationFlowBottleneck,
        FindingType::HubNode,
    ];

    let confidence = match rq.resolution_tier {
        ResolutionTier::Full => Confidence::High,
        ResolutionTier::HeuristicOnly => Confidence::Medium,
        ResolutionTier::None => Confidence::Low,
    };

    for finding in findings.iter_mut() {
        if CROSS_FILE_DEPENDENT_TYPES.contains(&finding.finding_type) {
            finding.confidence = Some(confidence);
        }
    }
}

pub(crate) fn assign_node_risk_levels(
    findings: &[Finding],
    annotations: &mut BTreeMap<String, NodeAnnotation>,
) {
    // Count finding severity per node
    let mut node_finding_counts: HashMap<String, Vec<Severity>> = HashMap::new();

    for f in findings {
        for nid in &f.node_ids {
            node_finding_counts
                .entry(nid.clone())
                .or_default()
                .push(f.severity);
        }
    }

    for (nid, severities) in &node_finding_counts {
        if let Some(ann) = annotations.get_mut(nid) {
            let critical_or_high = severities
                .iter()
                .filter(|s| matches!(s, Severity::Critical | Severity::High))
                .count();
            let high_or_warning = severities
                .iter()
                .filter(|s| matches!(s, Severity::High | Severity::Warning))
                .count();
            let has_critical = severities.contains(&Severity::Critical);
            let only_info = severities.iter().all(|s| *s == Severity::Info);

            ann.risk_level = if critical_or_high >= 2 {
                RiskLevel::Critical
            } else if has_critical || high_or_warning >= 2 {
                RiskLevel::High
            } else if severities
                .iter()
                .any(|s| matches!(s, Severity::High | Severity::Warning))
            {
                RiskLevel::Warning
            } else if only_info {
                RiskLevel::Info
            } else {
                RiskLevel::Healthy
            };
        }
    }
}

pub(crate) fn assign_module_risk_levels(
    findings: &[Finding],
    annotations: &mut BTreeMap<String, ModuleAnnotation>,
) {
    let mut module_severities: HashMap<String, Vec<Severity>> = HashMap::new();
    for f in findings {
        for nid in &f.node_ids {
            module_severities
                .entry(nid.clone())
                .or_default()
                .push(f.severity);
        }
    }

    for (mid, ann) in annotations.iter_mut() {
        if let Some(severities) = module_severities.get(mid) {
            let has_critical = severities.contains(&Severity::Critical);
            let has_high = severities.contains(&Severity::High);
            let has_warning = severities.contains(&Severity::Warning);

            ann.risk_level = if has_critical {
                RiskLevel::Critical
            } else if has_high {
                RiskLevel::High
            } else if has_warning {
                RiskLevel::Warning
            } else {
                RiskLevel::Info
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

pub(crate) fn node_annotation_for(node: &graph::GraphNode) -> NodeAnnotation {
    NodeAnnotation {
        fqn: node.fqn.clone(),
        display_name: node.display_name(),
        file_path: node.file_path.clone(),
        start_line: node.start_line,
        fan_in: 0,
        fan_out: 0,
        blast_radius: 0,
        depth_from_root: 0,
        information_flow_complexity: 0,
        is_hotspot: false,
        is_dead: false,
        dead_code_reason: None,
        dead_code_evidence: None,
        dead_code_classifier: None,
        dead_code_confidence: None,
        is_test: node.is_test,
        cycle_member: false,
        cycle_ids: vec![],
        hub_score: 0.0,
        cyclomatic_complexity: None,
        cognitive_complexity: None,
        loc: 0,
        inferred_layer: None,
        layer_label: None,
        risk_level: RiskLevel::Healthy,
    }
}

/// Assign a semantic layer label based on BFS depth and max depth.
/// Labels are relative: 0 is always "entry", the deepest is "infrastructure".
pub(crate) fn layer_label_for_depth(depth: usize, max_depth: usize) -> &'static str {
    if max_depth == 0 {
        return "entry";
    }
    let normalized = depth as f64 / max_depth as f64;
    if depth == 0 {
        "entry"
    } else if normalized <= 0.25 {
        "controller"
    } else if normalized <= 0.60 {
        "service"
    } else {
        "infrastructure"
    }
}

/// Cap findings to at most `max_per_type` per finding type.
/// Excess findings are summarized into a single aggregate finding per type.
pub(crate) fn cap_findings(findings: Vec<Finding>, max_per_type: usize) -> Vec<Finding> {
    let mut by_type: HashMap<String, Vec<Finding>> = HashMap::new();
    for f in findings {
        let type_key = format!("{:?}", f.finding_type);
        by_type.entry(type_key).or_default().push(f);
    }

    let mut result = Vec::new();
    for (_type_key, mut type_findings) in by_type {
        // Already sorted by severity from the parent sort
        if type_findings.len() <= max_per_type {
            result.append(&mut type_findings);
        } else {
            let overflow_count = type_findings.len() - max_per_type;
            let finding_type = type_findings[0].finding_type;
            let kept: Vec<Finding> = type_findings.drain(..max_per_type).collect();
            result.extend(kept);
            result.push(Finding {
                id: format!("{:?}-overflow", finding_type).to_lowercase(),
                finding_type,
                severity: Severity::Info,
                description: format!(
                    "{overflow_count} additional {finding_type:?} findings not shown (top {max_per_type} displayed above)"
                ),
                detail: Some(
                    "Findings are capped per type for readability. This entry summarizes the count of additional findings not displayed."
                        .into(),
                ),
                node_ids: vec![],
                edge_ids: None,
                primary_node_id: None,
                metric_name: None,
                metric_value: Some(overflow_count as f64),
                impact: None,
                blast_radius: None,
                recommendation: None,
                cycle_length: None,
                fan_in: None,
                coupling_score: None,
                internal_edges: None,
                external_edges: None,
                count: Some(overflow_count),
                hub_score: None,
                file_a: None,
                file_b: None,
                co_change_count: None,
                temporal_coupling_score: None,
                has_import_edge: None,
                confidence: None,
            });
        }
    }

    // Re-sort after capping
    result.sort_by(|a, b| {
        b.severity
            .rank()
            .cmp(&a.severity.rank())
            .then_with(|| a.id.cmp(&b.id))
    });

    result
}

/// Compute the Nth percentile of a set of values.
/// Returns at least `floor` to prevent degenerate thresholds in small/uniform distributions.
pub(crate) fn percentile_threshold_usize(
    mut values: Vec<usize>,
    percentile: usize,
    floor: usize,
) -> usize {
    if values.is_empty() {
        return floor;
    }
    values.sort_unstable();
    let idx = ((percentile as f64 / 100.0) * values.len() as f64).ceil() as usize;
    let idx = idx.min(values.len()).saturating_sub(1);
    values[idx].max(floor)
}

pub(crate) fn build_classifications(
    graph: &AnalysisGraph,
    cfg: &config::AnalysisConfig,
) -> BTreeMap<String, report::ModuleClassification> {
    let mut out = BTreeMap::new();

    let emit =
        |role: &str, confidence: &str, reason: &str, source: &str| report::ModuleClassification {
            counts_toward_score: role == "production",
            role: role.into(),
            confidence: confidence.into(),
            reason: reason.into(),
            source: source.into(),
        };

    for module_key in &graph.folder_module_ids {
        // Layer 3: user config overrides (highest priority)
        if let Some(ref overrides) = cfg.classification_overrides {
            if overrides
                .production_paths
                .iter()
                .any(|p| module_key.starts_with(p))
            {
                out.insert(
                    module_key.clone(),
                    emit(
                        "production",
                        "definitive",
                        "user override via config",
                        "user_config",
                    ),
                );
                continue;
            }
            if overrides
                .non_production_paths
                .iter()
                .any(|p| module_key.starts_with(p))
            {
                out.insert(
                    module_key.clone(),
                    emit(
                        "non_production",
                        "definitive",
                        "user override via config",
                        "user_config",
                    ),
                );
                continue;
            }
        }

        // Layer 2: structural detection (from parser's is_test flags)
        let structural_signal = graph.descendants_of(module_key).iter().find_map(|nid| {
            let n = graph.nodes.get(nid.as_str())?;
            if n.is_test {
                Some(("test", "structural: contains test-flagged nodes"))
            } else if n.is_vendor {
                Some(("vendor", "structural: contains vendor-flagged nodes"))
            } else if n.is_generated {
                Some(("generated", "structural: contains generated-flagged nodes"))
            } else if n.is_build_output {
                Some(("config", "structural: contains build output nodes"))
            } else {
                None
            }
        });

        if let Some((role, reason)) = structural_signal {
            out.insert(module_key.clone(), emit(role, "high", reason, "structural"));
            continue;
        }

        // Layer 1: path heuristic
        if let Some((role, reason)) = path_classification::classify_path(module_key) {
            let confidence = if role == "test" { "high" } else { "medium" };
            out.insert(
                module_key.clone(),
                emit(role, confidence, reason, "path_heuristic"),
            );
            continue;
        }

        // Default: production — but check if the module contains co-located test files
        let test_file_count = graph
            .analysis_module_members_of(module_key)
            .map(|members| {
                members
                    .iter()
                    .filter(|nid| graph.nodes.get(nid.as_str()).is_some_and(|n| n.is_test))
                    .count()
            })
            .unwrap_or(0);

        if test_file_count > 0 {
            out.insert(
                module_key.clone(),
                emit(
                    "production",
                    "high",
                    &format!(
                        "production (contains {} co-located test file{} excluded from scoring)",
                        test_file_count,
                        if test_file_count == 1 { "" } else { "s" },
                    ),
                    "path_heuristic",
                ),
            );
        } else {
            out.insert(
                module_key.clone(),
                emit(
                    "production",
                    "high",
                    "no test signals detected",
                    "path_heuristic",
                ),
            );
        }
    }

    eprintln!(
        "[ge-analyze] Classified {} modules ({} production, {} non-production).",
        out.len(),
        out.values().filter(|c| c.counts_toward_score).count(),
        out.values().filter(|c| !c.counts_toward_score).count(),
    );

    out
}

pub(crate) fn empty_report(
    db_path: &str,
    elapsed_ms: u64,
    weights: &config::ScoreWeights,
) -> HealthReport {
    HealthReport {
        version: "1.0.0".into(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        analysis_duration_ms: elapsed_ms,
        db_path: db_path.to_string(),
        health_score: Some(100),
        health_score_components: HealthScoreComponents {
            cycle_severity: ScoreComponent {
                score: 100,
                weight: weights.cycle_severity,
            },
            coupling_health: ScoreComponent {
                score: 100,
                weight: weights.coupling_health,
            },
            hotspot_concentration: ScoreComponent {
                score: 100,
                weight: weights.hotspot_concentration,
            },
            dead_code_ratio: ScoreComponent {
                score: 100,
                weight: weights.dead_code_ratio,
            },
            depth_complexity: ScoreComponent {
                score: 100,
                weight: weights.depth_complexity,
            },
            complexity: ScoreComponent {
                score: 100,
                weight: weights.complexity,
            },
            cohesion: ScoreComponent {
                score: 100,
                weight: weights.cohesion,
            },
            distance: ScoreComponent {
                score: 100,
                weight: weights.distance,
            },
            temporal_coupling: ScoreComponent {
                score: 100,
                weight: weights.temporal_coupling,
            },
        },
        metrics: MetricsReport {
            cycles: MetricDetail {
                count: 0,
                total: 0,
                ratio: 0.0,
                description: "No nodes in graph".into(),
                status: MetricStatus::NotApplicable,
                // Empty-graph fallback: no edges exist, no gap to measure.
                fidelity: None,
            },
            coupling: CouplingMetricDetail {
                modules_measured: 0,
                modules_above_070: 0,
                modules_above_050: 0,
                avg_coupling: 0.0,
                description: "No modules in graph".into(),
                status: MetricStatus::NotApplicable,
                fidelity: None,
            },
            hotspot_concentration: MetricDetail {
                count: 0,
                total: 0,
                ratio: 0.0,
                description: "No functions in graph".into(),
                status: MetricStatus::NotApplicable,
                fidelity: None,
            },
            dead_code: DeadCodeMetricDetail {
                count: 0,
                total: 0,
                ratio: 0.0,
                description: "No functions in graph".into(),
                status: MetricStatus::NotApplicable,
                reason_breakdown: dead_code_classifier::empty_reason_breakdown(),
                // Empty graph: no language signal, no caveats needed.
                reason_breakdown_caveats: None,
                fidelity: None,
                no_callers_total: None,
                no_callers_high_confidence: None,
            },
            depth: DepthMetricDetail {
                max_call_depth: 0,
                description: "No call chains".into(),
                status: MetricStatus::NotApplicable,
                fidelity: None,
            },
            tangle_index: MetricDetail {
                count: 0,
                total: 0,
                ratio: 0.0,
                description: "No structural edges".into(),
                status: MetricStatus::NotApplicable,
                fidelity: None,
            },
            complexity: None,
            cohesion: None,
            distance_from_main_sequence: None,
            temporal_coupling: None,
            metric_confidence: None,
        },
        percentiles: None,
        summary: Summary {
            total_nodes: 0,
            total_edges: 0,
            total_functions: 0,
            total_modules: 0,
            cycles_found: 0,
            cycle_total_nodes: 0,
            hotspot_count: 0,
            hotspot_threshold_fan_in: 0,
            high_coupling_modules: 0,
            dead_functions: 0,
            max_call_depth: 0,
            tangle_index: 0.0,
            avg_module_coupling: 0.0,
            avg_fan_in: 0.0,
            avg_fan_out: 0.0,
        },
        findings: vec![],
        node_annotations: BTreeMap::new(),
        module_annotations: BTreeMap::new(),
        classifications: BTreeMap::new(),
        boundary_violations: vec![],
        resolution_quality: None,
        analysis_errors: vec![],
        integrity_status: build_integrity_status(false, false, 0),
        // Empty-report path: the DB had no nodes. Git signals are
        // computed against the working tree so they could in
        // principle still be meaningful here, but the orchestrator
        // will attach them from the caller if it chooses to —
        // keeping `empty_report` single-responsibility.
        git_signals: None,
        file_extraction_coverage: Vec::new(),
        // Empty graph → no File nodes → no defensible language signal.
        primary_language: None,
        analysis_provenance: None,
    }
}
/// Mirrors `graphengine_parsing::infrastructure::storage::schema::PARSE_META_SCHEMA_VERSION`
/// but duplicated here so `graphengine-analysis` does not take a hard
/// dependency on the parsing crate for what is effectively a
/// small-integer version constant. Bumping the parsing-side constant
/// without also bumping this one is an integration bug — both must
/// move together. The `apex_class_symbols_e2e::parse_meta_table_*`
/// tests in `graphengine-parsing/tests/apex_class_symbols_e2e.rs`
/// exercise the writer path, and `integration_test::stale_parse_*`
/// in this crate exercise the reader path.
///
/// History:
/// * `2` — TR-A.0 baseline (Apex class symbols + file_extraction_coverage).
/// * `3` — S1 incremental scanning (adds `file_cache` table). Old v2
///   DBs are treated as stale and trigger `CAVEAT_STALE_PARSE_DB_V1`;
///   the user re-runs `gridseak scan` to repopulate.
const ANALYSIS_EXPECTED_SCHEMA_VERSION: u32 = 3;

/// Key the parsing-side persistence layer uses for the schema
/// version row in the `parse_meta` table. See the parsing-side
/// `schema.rs` for the authoritative definition.
const PARSE_META_SCHEMA_VERSION_KEY: &str = "schema_version";

/// Return `true` if the parse DB was produced by a pre-TR-A.0 engine,
/// in which case any Apex resolver logic that consumes the
/// `apex_class_symbols` table will silently degrade and downstream
/// no_callers counts will be pessimistic. Absence of the `parse_meta`
/// table (oldest DBs) is treated as "stale" — the caveat prompts the
/// user to re-parse rather than silently mixing pre/post-rev-7 data.
pub(crate) fn is_parse_db_stale(conn: &Connection) -> bool {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='parse_meta'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return true;
    }
    let version: Option<String> = conn
        .query_row(
            "SELECT value FROM parse_meta WHERE key = ?1",
            rusqlite::params![PARSE_META_SCHEMA_VERSION_KEY],
            |row| row.get(0),
        )
        .ok();
    match version.and_then(|v| v.parse::<u32>().ok()) {
        Some(v) => v < ANALYSIS_EXPECTED_SCHEMA_VERSION,
        None => true,
    }
}

/// Build the IntegrityStatus stamped onto every HealthReport produced by
/// this engine. See `report.rs` for the locked caveat constants.
///
/// # Arguments
/// * `invariant_violations` — true if any `graph_invariants` analysis
///   error fired during this run.
/// * `stale_parse_db` — true if the parse DB was produced by an engine
///   older than the current `PARSE_META_SCHEMA_VERSION` (or predates
///   the `parse_meta` table entirely). When true, emits
///   `CAVEAT_STALE_PARSE_DB_V1`. This is TR-A.0's safety signal: any
///   downstream Apex no_callers / dead-code counts derived from a
///   stale DB are pessimistic because the Apex type oracle
///   (`apex_class_symbols`) was never populated, so constructor /
///   field-type / overload / inner-class dispatch silently degrades
///   to the pre-rev-7 heuristic.
pub(crate) fn build_integrity_status(
    invariant_violations: bool,
    stale_parse_db: bool,
    unknown_edge_kind_count: usize,
) -> IntegrityStatus {
    let mut schema_caveats = vec![
        report::CAVEAT_CYCLES_ORDERFIX_APPLIED.to_string(),
        report::CAVEAT_METRIC_STATUS_CONTRACT.to_string(),
        report::CAVEAT_DEAD_CODE_REASONS_V1.to_string(),
        report::CAVEAT_DUAL_METRIC_EMISSION_V1.to_string(),
    ];
    if stale_parse_db {
        schema_caveats.push(report::CAVEAT_STALE_PARSE_DB_V1.to_string());
    }
    if unknown_edge_kind_count > 0 {
        schema_caveats.push(report::CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1.to_string());
    }
    IntegrityStatus {
        engine_version: crate::VERSION.to_string(),
        engine_commit: option_env!("GE_COMMIT_SHA").map(|s| s.to_string()),
        schema_caveats,
        invariant_violations,
    }
}
