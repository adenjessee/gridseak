//! Validation override structures and applicators.
//!
//! The user's corrections are submitted as a JSON overrides object.
//! `ge-analyze` accepts this alongside the database and applies it
//! before running analysis. This is a pure data file — no code execution,
//! no schema changes, fully declarative.

use std::collections::HashSet;

use crate::health::config::{AnalysisConfig, DeadCodeConfig};
use crate::health::graph::AnalysisGraph;

/// The full override payload submitted by the user/agent.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ValidationOverrides {
    #[serde(default)]
    pub file_overrides: Vec<FileOverride>,

    #[serde(default)]
    pub entry_point_overrides: Vec<EntryPointOverride>,

    #[serde(default)]
    pub entry_point_patterns: Vec<EntryPointPattern>,

    #[serde(default)]
    pub module_overrides: Option<ModuleOverrides>,

    #[serde(default)]
    pub repo_type_override: Option<String>,

    #[serde(default)]
    pub finding_triage: Vec<FindingTriage>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileOverride {
    pub path: String,
    pub classification: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntryPointOverride {
    pub function_id: String,
    pub action: EntryPointAction,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryPointAction {
    MarkAsEntryPoint,
    ConfirmDead,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntryPointPattern {
    pub pattern: String,
    pub action: EntryPointAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ModuleOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_depth: Option<usize>,

    #[serde(default)]
    pub strategy: Option<String>,

    #[serde(default)]
    pub file_level_modules: Vec<String>,

    #[serde(default)]
    pub merges: Vec<ModuleMerge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModuleMerge {
    pub sources: Vec<String>,
    pub target: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FindingTriage {
    pub finding_id: String,
    pub action: TriageAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageAction {
    Acknowledged,
    WontFix,
    FalsePositive,
}

/// Summary of what was applied from the overrides.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct OverridesSummary {
    pub files_reclassified: usize,
    pub entry_points_added: usize,
    pub entry_point_patterns_added: usize,
    pub module_depth_changed: bool,
    pub repo_type_overridden: bool,
    pub findings_triaged: usize,
}

/// Apply file classification overrides to the graph.
///
/// For each file override, find the File node by path and update its
/// `is_test`, `is_vendor`, `is_generated` flags based on the classification.
pub fn apply_file_overrides(graph: &mut AnalysisGraph, overrides: &[FileOverride]) -> usize {
    let mut applied = 0;

    for fo in overrides {
        let matching_ids: Vec<String> = graph
            .nodes
            .iter()
            .filter(|(_, n)| {
                n.kind == crate::health::graph::NodeKind::File
                    && (n.path_repo_rel.as_deref() == Some(&fo.path)
                        || n.file_path.as_deref() == Some(&fo.path)
                        || n.fqn == fo.path)
            })
            .map(|(id, _)| id.clone())
            .collect();

        for id in matching_ids {
            if let Some(node) = graph.nodes.get_mut(&id) {
                match fo.classification.as_str() {
                    "test" | "non_production" => {
                        node.is_test = true;
                        node.is_vendor = false;
                        node.is_generated = false;
                    }
                    "production" => {
                        node.is_test = false;
                        node.is_vendor = false;
                        node.is_generated = false;
                    }
                    "vendor" => {
                        node.is_vendor = true;
                    }
                    "generated" => {
                        node.is_generated = true;
                    }
                    _ => continue,
                }
                applied += 1;
            }
        }
    }

    if applied > 0 {
        graph.finalize_production_edges();
    }

    applied
}

/// Apply entry point overrides to the dead code config.
///
/// - `mark_as_entry_point` overrides are collected into a set that the
///   analysis pipeline checks before flagging dead code.
/// - `confirm_dead` overrides are tracked so the UI can display them
///   but they don't change analysis behavior (dead is the default).
pub fn apply_entry_point_overrides(
    overrides: &ValidationOverrides,
    dc_config: &mut DeadCodeConfig,
) -> (HashSet<String>, usize) {
    let mut exempt_ids: HashSet<String> = HashSet::new();
    let mut count = 0;

    for epo in &overrides.entry_point_overrides {
        if matches!(epo.action, EntryPointAction::MarkAsEntryPoint) {
            exempt_ids.insert(epo.function_id.clone());
            count += 1;
        }
    }

    for epp in &overrides.entry_point_patterns {
        if matches!(epp.action, EntryPointAction::MarkAsEntryPoint) {
            dc_config
                .extra_entry_point_patterns
                .push(epp.pattern.clone());
        }
    }

    (exempt_ids, count)
}

/// Apply module overrides to the analysis config.
pub fn apply_module_overrides(
    overrides: &ValidationOverrides,
    config: &mut AnalysisConfig,
) -> bool {
    let mut changed = false;

    if let Some(ref mo) = overrides.module_overrides {
        if let Some(depth) = mo.analysis_depth {
            if depth != config.modules.analysis_depth {
                config.modules.analysis_depth = depth;
                changed = true;
            }
        }
    }

    changed
}

/// Filter findings based on triage actions from previous runs.
/// Returns the set of finding IDs that should be suppressed.
pub fn suppressed_finding_ids(overrides: &ValidationOverrides) -> HashSet<String> {
    overrides
        .finding_triage
        .iter()
        .filter(|t| {
            matches!(
                t.action,
                TriageAction::WontFix | TriageAction::FalsePositive
            )
        })
        .map(|t| t.finding_id.clone())
        .collect()
}

/// Apply all overrides and return a summary.
pub fn apply_all_overrides(
    graph: &mut AnalysisGraph,
    config: &mut AnalysisConfig,
    overrides: &ValidationOverrides,
) -> OverridesSummary {
    let files_reclassified = apply_file_overrides(graph, &overrides.file_overrides);

    let (_entry_point_exempt_ids, entry_points_added) =
        apply_entry_point_overrides(overrides, &mut config.dead_code);

    let module_depth_changed = apply_module_overrides(overrides, config);

    let repo_type_overridden = overrides.repo_type_override.is_some();

    let findings_triaged = overrides
        .finding_triage
        .iter()
        .filter(|t| {
            matches!(
                t.action,
                TriageAction::WontFix | TriageAction::FalsePositive
            )
        })
        .count();

    OverridesSummary {
        files_reclassified,
        entry_points_added,
        entry_point_patterns_added: overrides.entry_point_patterns.len(),
        module_depth_changed,
        repo_type_overridden,
        findings_triaged,
    }
}

/// Load overrides from a JSON file path.
pub fn load_overrides(path: &str) -> Result<ValidationOverrides, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read overrides file '{}': {}", path, e))?;

    serde_json::from_str(&content).map_err(|e| format!("Failed to parse overrides JSON: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_overrides_parse() {
        let json = "{}";
        let o: ValidationOverrides = serde_json::from_str(json).unwrap();
        assert!(o.file_overrides.is_empty());
        assert!(o.entry_point_overrides.is_empty());
        assert!(o.finding_triage.is_empty());
    }

    #[test]
    fn full_overrides_parse() {
        let json = r#"{
            "file_overrides": [
                {"path": "src/testing/helpers.ts", "classification": "test"}
            ],
            "entry_point_overrides": [
                {"function_id": "abc123", "action": "mark_as_entry_point"}
            ],
            "entry_point_patterns": [
                {"pattern": "handle_.*", "action": "mark_as_entry_point", "reason": "HTTP handlers"}
            ],
            "module_overrides": {
                "analysis_depth": 4
            },
            "repo_type_override": "library",
            "finding_triage": [
                {"finding_id": "dead-1", "action": "wont_fix", "reason": "Intentional API surface"}
            ]
        }"#;
        let o: ValidationOverrides = serde_json::from_str(json).unwrap();
        assert_eq!(o.file_overrides.len(), 1);
        assert_eq!(o.file_overrides[0].classification, "test");
        assert_eq!(o.entry_point_overrides.len(), 1);
        assert_eq!(o.entry_point_patterns.len(), 1);
        assert_eq!(
            o.module_overrides.as_ref().map(|m| m.analysis_depth),
            Some(Some(4))
        );
        assert_eq!(o.repo_type_override.as_deref(), Some("library"));
        assert_eq!(o.finding_triage.len(), 1);
    }

    #[test]
    fn suppressed_findings() {
        let overrides = ValidationOverrides {
            finding_triage: vec![
                FindingTriage {
                    finding_id: "dead-1".into(),
                    action: TriageAction::WontFix,
                    reason: None,
                },
                FindingTriage {
                    finding_id: "cycle-3".into(),
                    action: TriageAction::FalsePositive,
                    reason: None,
                },
                FindingTriage {
                    finding_id: "coupling-foo".into(),
                    action: TriageAction::Acknowledged,
                    reason: None,
                },
            ],
            ..Default::default()
        };

        let suppressed = suppressed_finding_ids(&overrides);
        assert!(suppressed.contains("dead-1"));
        assert!(suppressed.contains("cycle-3"));
        assert!(!suppressed.contains("coupling-foo"));
    }
}
