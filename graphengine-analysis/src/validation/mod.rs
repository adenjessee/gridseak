//! Pre-analysis user validation API.
//!
//! After parsing completes but before analysis runs, this module emits a
//! `ValidationPayload` describing what the engine is uncertain about.
//! The user (or an AI agent) reviews the payload and returns corrections
//! via `ValidationOverrides`, which are applied before analysis begins.
//!
//! The validation read is fast (< 1 second) — it queries the parsed SQLite
//! database without running any analysis algorithms.

pub mod ecosystem_notes;
pub mod invocation_patterns;
pub mod overrides;

use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::health::config::{AnalysisConfig, DeadCodeConfig, Ecosystem, ModuleConfig};
use crate::health::graph::{self, AnalysisGraph};
use crate::health::path_classification;
use crate::health::repo_classification;
use crate::health::structural_classification;

use ecosystem_notes::EcosystemNote;
use invocation_patterns::DeadCodeCandidate;

// ---------------------------------------------------------------------------
// Validation payload (output)
// ---------------------------------------------------------------------------

/// The full validation payload emitted after parsing, before analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationPayload {
    pub file_classifications: FileClassifications,
    pub dead_code_candidates: Vec<DeadCodeCandidate>,
    pub module_boundaries: Vec<ModuleBoundaryIssue>,
    pub repo_type: RepoTypeInfo,
    pub ecosystem_notes: Vec<EcosystemNote>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileClassifications {
    pub uncertain: Vec<UncertainFile>,
    pub auto_classified: ClassificationCounts,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UncertainFile {
    pub path: String,
    pub auto_classification: String,
    pub confidence: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClassificationCounts {
    pub production: usize,
    pub test: usize,
    pub benchmark: usize,
    pub example: usize,
    pub fixture: usize,
    pub vendor: usize,
    pub generated: usize,
    pub config: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModuleBoundaryIssue {
    pub module_key: String,
    pub function_count: usize,
    pub file_count: usize,
    pub warning: String,
    pub suggested_strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sub_modules_if_deeper: Vec<SubModulePreview>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubModulePreview {
    pub key: String,
    pub function_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoTypeInfo {
    pub auto_detected: String,
    pub confidence: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Emit the full validation payload from a parsed SQLite database.
///
/// This is a read-only query — it does not modify the database.
/// It loads the graph, detects classifications, dead code candidates,
/// module boundaries, and repo type. Fast: typically < 1 second.
pub fn emit_validation_payload(db_path: &str) -> Result<ValidationPayload> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("Failed to open database")?;

    graph::validate_schema(&conn).context("Invalid database schema")?;

    let ecosystem = graph::detect_ecosystem(&conn);
    let cfg = AnalysisConfig::for_ecosystem(ecosystem);
    let mut ag = AnalysisGraph::load_with_depth(&conn, cfg.modules.analysis_depth)?;

    // Run structural classification to refine test detection
    let language_str = ecosystem.to_string();
    let structural_test_files = structural_classification::classify_files(&ag, &language_str);
    {
        let test_file_paths: HashSet<String> = structural_test_files
            .iter()
            .filter(|(_, c)| {
                matches!(
                    c.role,
                    structural_classification::FileRole::Test
                        | structural_classification::FileRole::TestSupport
                )
            })
            .filter_map(|(id, _)| ag.nodes.get(id).and_then(|n| n.file_path.clone()))
            .collect();

        for node in ag.nodes.values_mut() {
            if node.kind == graph::NodeKind::File {
                let path = node.file_path.as_deref().or(node.path_repo_rel.as_deref());
                if let Some(p) = path {
                    if test_file_paths.contains(p) {
                        node.is_test = true;
                    }
                }
            }
        }
    }
    ag.finalize_production_edges();

    let file_classifications = build_file_classifications(&ag);
    let dead_code_candidates = build_dead_code_candidates(&ag, &cfg.dead_code, ecosystem);
    let module_boundaries = build_module_boundaries(&ag, &cfg.modules);
    let repo_type = build_repo_type_info(&ag);
    let eco_notes = ecosystem_notes::generate_ecosystem_notes(ecosystem);

    Ok(ValidationPayload {
        file_classifications,
        dead_code_candidates,
        module_boundaries,
        repo_type,
        ecosystem_notes: eco_notes,
    })
}

// ---------------------------------------------------------------------------
// File classification analysis
// ---------------------------------------------------------------------------

fn build_file_classifications(graph: &AnalysisGraph) -> FileClassifications {
    let mut uncertain = Vec::new();
    let mut counts = ClassificationCounts::default();

    for node in graph.nodes.values() {
        if node.kind != graph::NodeKind::File {
            continue;
        }

        let path = node
            .path_repo_rel
            .as_deref()
            .or(node.file_path.as_deref())
            .unwrap_or(&node.fqn);

        // Structural flags from parser
        if node.is_vendor {
            counts.vendor += 1;
            continue;
        }
        if node.is_generated || node.is_build_output {
            counts.generated += 1;
            continue;
        }

        // Structural test detection (is_test set by parser or structural classifier)
        if node.is_test {
            counts.test += 1;
            // Check for uncertainty: files flagged as test that are inside src/
            let path_lower = path.to_lowercase();
            let is_in_src = path_lower.starts_with("src/") || path_lower.contains("/src/");
            let has_test_signal_in_name = path_classification::is_test_path(path);
            if is_in_src && !has_test_signal_in_name {
                uncertain.push(UncertainFile {
                    path: path.to_string(),
                    auto_classification: "test".into(),
                    confidence: "low".into(),
                    reason: format!(
                        "structurally classified as test but path '{}' is inside src/ with no test naming pattern",
                        path
                    ),
                });
            }
            continue;
        }

        // Path heuristic classification
        if let Some((role, reason)) = path_classification::classify_path(path) {
            match role {
                "test" => counts.test += 1,
                "benchmark" => counts.benchmark += 1,
                "example" => counts.example += 1,
                "fixture" => counts.fixture += 1,
                "config" | "docs" | "auxiliary" => counts.config += 1,
                _ => counts.production += 1,
            }

            // Flag uncertain cases: path has ambiguous signals
            if is_ambiguous_classification(path, role) {
                uncertain.push(UncertainFile {
                    path: path.to_string(),
                    auto_classification: role.into(),
                    confidence: "medium".into(),
                    reason: reason.into(),
                });
            }
            continue;
        }

        // Default: production
        counts.production += 1;

        // Flag files with mixed signals (e.g., "testing" in path but inside src/)
        if has_mixed_classification_signals(path) {
            uncertain.push(UncertainFile {
                path: path.to_string(),
                auto_classification: "production".into(),
                confidence: "low".into(),
                reason: format!(
                    "path '{}' contains testing-related words but is classified as production",
                    path
                ),
            });
        }
    }

    FileClassifications {
        uncertain,
        auto_classified: counts,
    }
}

fn is_ambiguous_classification(path: &str, role: &str) -> bool {
    let lower = path.to_lowercase();

    // Files in src/ classified as non-production based solely on path words
    if role != "production" {
        let in_src = lower.starts_with("src/") || lower.contains("/src/");
        if in_src {
            return true;
        }
    }

    // Files with "helper" or "util" in name classified as test/fixture
    if matches!(role, "fixture" | "test") {
        let filename = lower.rsplit('/').next().unwrap_or(&lower);
        if filename.contains("helper") || filename.contains("util") || filename.contains("support")
        {
            return true;
        }
    }

    false
}

fn has_mixed_classification_signals(path: &str) -> bool {
    let lower = path.to_lowercase();
    let normalized = lower.replace('\\', "/");

    let suspicious_words = ["testing", "test_helper", "test_util", "testdata", "mock_"];
    for component in normalized.split('/') {
        for word in &suspicious_words {
            if component.contains(word) {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Dead code candidate enrichment
// ---------------------------------------------------------------------------

fn build_dead_code_candidates(
    graph: &AnalysisGraph,
    dc_config: &DeadCodeConfig,
    ecosystem: Ecosystem,
) -> Vec<DeadCodeCandidate> {
    invocation_patterns::collect_dead_code_candidates(graph, dc_config, ecosystem)
}

// ---------------------------------------------------------------------------
// Module boundary analysis
// ---------------------------------------------------------------------------

const VERY_LARGE_MODULE_THRESHOLD: usize = 200;
const LARGE_MODULE_THRESHOLD: usize = 100;

fn build_module_boundaries(
    graph: &AnalysisGraph,
    module_config: &ModuleConfig,
) -> Vec<ModuleBoundaryIssue> {
    let mut issues = Vec::new();

    for module_key in &graph.folder_module_ids {
        if graph.is_non_production_node(module_key) {
            continue;
        }

        let members = match graph.analysis_module_members_of(module_key) {
            Some(m) => m,
            None => continue,
        };

        let function_count = members
            .iter()
            .filter(|id| {
                graph
                    .nodes
                    .get(id.as_str())
                    .map(|n| n.kind.is_function_like() && !graph::is_synthetic_node(n))
                    .unwrap_or(false)
            })
            .count();

        // Count unique files in the module
        let file_paths: HashSet<String> = members
            .iter()
            .filter_map(|id| {
                graph
                    .nodes
                    .get(id.as_str())
                    .and_then(|n| n.file_path.clone())
            })
            .collect();
        let file_count = file_paths.len();

        if function_count < LARGE_MODULE_THRESHOLD {
            continue;
        }

        // Check if this is a flat module (all files in one directory)
        let unique_dirs: HashSet<String> = file_paths
            .iter()
            .filter_map(|p| {
                let parts: Vec<&str> = p.rsplitn(2, '/').collect();
                if parts.len() == 2 {
                    Some(parts[1].to_string())
                } else {
                    None
                }
            })
            .collect();
        let is_flat = unique_dirs.len() <= 1;

        let (warning, suggested_strategy, suggested_depth, note) = if function_count
            >= VERY_LARGE_MODULE_THRESHOLD
        {
            if is_flat {
                (
                    "very_large_flat".to_string(),
                    "file_level".to_string(),
                    None,
                    Some(
                        "All files are in one directory — increasing depth has no effect. Consider per-file module splitting."
                            .to_string(),
                    ),
                )
            } else {
                (
                    "very_large".to_string(),
                    "increase_depth".to_string(),
                    Some(module_config.analysis_depth + 1),
                    None,
                )
            }
        } else {
            (
                "large".to_string(),
                if is_flat {
                    "file_level".to_string()
                } else {
                    "increase_depth".to_string()
                },
                if is_flat {
                    None
                } else {
                    Some(module_config.analysis_depth + 1)
                },
                None,
            )
        };

        // Preview what sub-modules would exist at depth + 1
        let sub_modules =
            preview_sub_modules(graph, module_key, module_config.analysis_depth + 1, is_flat);

        issues.push(ModuleBoundaryIssue {
            module_key: module_key.clone(),
            function_count,
            file_count,
            warning,
            suggested_strategy,
            suggested_depth,
            note,
            sub_modules_if_deeper: sub_modules,
        });
    }

    issues.sort_by(|a, b| b.function_count.cmp(&a.function_count));
    issues
}

fn preview_sub_modules(
    graph: &AnalysisGraph,
    module_key: &str,
    deeper_depth: usize,
    is_flat: bool,
) -> Vec<SubModulePreview> {
    let members = match graph.analysis_module_members_of(module_key) {
        Some(m) => m,
        None => return Vec::new(),
    };

    let mut sub_counts: BTreeMap<String, usize> = BTreeMap::new();

    for id in members {
        let node = match graph.nodes.get(id.as_str()) {
            Some(n) => n,
            None => continue,
        };
        if !node.kind.is_function_like() || graph::is_synthetic_node(node) {
            continue;
        }

        let path = graph.resolve_path_for(id);
        if let Some(ref p) = path {
            let key = if is_flat {
                // File-level: use the full file path as key
                p.clone()
            } else {
                // Deeper directory: extract path prefix at deeper_depth
                let segments: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
                let dir_segments = if segments.last().map(|s| s.contains('.')).unwrap_or(false) {
                    &segments[..segments.len() - 1]
                } else {
                    &segments[..]
                };
                let take = dir_segments.len().min(deeper_depth);
                dir_segments[..take].join("/")
            };

            if !key.is_empty() {
                *sub_counts.entry(key).or_default() += 1;
            }
        }
    }

    let mut previews: Vec<SubModulePreview> = sub_counts
        .into_iter()
        .filter(|(key, _)| key != module_key)
        .map(|(key, count)| SubModulePreview {
            key,
            function_count: count,
        })
        .collect();

    previews.sort_by(|a, b| b.function_count.cmp(&a.function_count));
    previews.truncate(20);
    previews
}

// ---------------------------------------------------------------------------
// Repo type detection
// ---------------------------------------------------------------------------

fn build_repo_type_info(graph: &AnalysisGraph) -> RepoTypeInfo {
    let file_paths: Vec<&str> = graph
        .nodes
        .values()
        .filter_map(|n| n.file_path.as_deref())
        .collect();

    let ws_root = repo_classification::infer_workspace_root(&file_paths);

    match ws_root {
        Some(root) => {
            let rtype = repo_classification::classify_repo(&root);
            let (detected, confidence, reason) = match rtype {
                repo_classification::RepoType::Library => {
                    let reason = detect_library_reason(&root);
                    ("library", "high", reason)
                }
                repo_classification::RepoType::Application => (
                    "application",
                    "medium",
                    "no library manifest indicators found — defaults to application".to_string(),
                ),
            };
            RepoTypeInfo {
                auto_detected: detected.into(),
                confidence: confidence.into(),
                reason,
            }
        }
        None => {
            // Repo-relative paths (non-absolute) — try heuristic from FQNs
            let has_lib = graph
                .nodes
                .values()
                .any(|n| n.fqn.contains("lib.rs") || n.fqn.contains("lib::") || n.name == "lib");
            let has_main = graph.nodes.values().any(|n| n.name == "main");

            if has_lib && !has_main {
                RepoTypeInfo {
                    auto_detected: "library".into(),
                    confidence: "medium".into(),
                    reason: "contains lib.rs but no main entry point".into(),
                }
            } else if has_main {
                RepoTypeInfo {
                    auto_detected: "application".into(),
                    confidence: "medium".into(),
                    reason: "has main entry point".into(),
                }
            } else {
                RepoTypeInfo {
                    auto_detected: "application".into(),
                    confidence: "low".into(),
                    reason:
                        "could not infer workspace root from file paths — defaulting to application"
                            .into(),
                }
            }
        }
    }
}

fn detect_library_reason(root: &std::path::Path) -> String {
    if root.join("package.json").exists() {
        return "package.json has 'main' or 'exports' field — npm library".into();
    }
    if root.join("src/lib.rs").exists() || root.join("Cargo.toml").exists() {
        let cargo = root.join("Cargo.toml");
        if cargo.exists() {
            if let Ok(content) = std::fs::read_to_string(&cargo) {
                if content.contains("[lib]") {
                    return "Cargo.toml has [lib] section".into();
                }
            }
        }
        if root.join("src/lib.rs").exists() {
            return "has src/lib.rs — Rust library crate".into();
        }
    }
    if root.join("setup.py").exists() {
        return "has setup.py — Python package".into();
    }
    if root.join("pyproject.toml").exists() {
        return "has pyproject.toml — Python package".into();
    }
    if root.join("go.mod").exists() && !root.join("main.go").exists() && !root.join("cmd").exists()
    {
        return "has go.mod but no main.go or cmd/ — Go library".into();
    }
    "library manifest detected".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_signal_detection() {
        assert!(has_mixed_classification_signals("src/testing/helpers.ts"));
        assert!(!has_mixed_classification_signals("src/auth/login.ts"));
    }

    #[test]
    fn ambiguous_test_in_src() {
        assert!(is_ambiguous_classification(
            "src/mock/handler.ts",
            "fixture"
        ));
        assert!(!is_ambiguous_classification("tests/auth.test.ts", "test"));
    }
}
