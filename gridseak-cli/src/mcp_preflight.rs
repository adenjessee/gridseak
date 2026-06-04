//! MCP preflight gates: STALE_SNAPSHOT, analysis readiness, envelope enrichment.

use std::path::Path;

use gridseak_local_store::{ProjectDto, ProjectStore};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::intent_router::{RoutePrecondition, RoutedTool};
use crate::workspace_delta::{
    compute_workspace_delta, normalize_rel_path, AnalysisReadiness, WorkspaceDelta,
};

pub const ANALYSIS_STATUS_KEY: &str = "analysis_status";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisStatus {
    pub mode: String,
    pub complete: bool,
    #[serde(default)]
    pub segments_done: Vec<String>,
    #[serde(default)]
    pub segments_pending: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleSnapshotError {
    pub error: &'static str,
    pub message: String,
    pub next_tool: Vec<&'static str>,
    pub workspace_delta: WorkspaceDelta,
    pub agent_directive: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisNotReadyError {
    pub error: &'static str,
    pub message: String,
    pub next_tool: Vec<&'static str>,
    pub analysis_readiness: AnalysisReadiness,
}

pub fn project_root(project: &ProjectDto) -> Option<String> {
    project.roots.first().map(|r| r.path.clone())
}

pub fn load_analysis_readiness(store: &ProjectStore, project: &ProjectDto) -> AnalysisReadiness {
    let graph_ready = project
        .latest_scan
        .as_ref()
        .and_then(|s| s.graph_artifact_path.clone())
        .is_some();

    let mut readiness = AnalysisReadiness {
        graph_ready,
        analysis_complete: graph_ready,
        mode: Some("full".into()),
        segments_pending: Vec::new(),
    };

    let Some(scan) = project.latest_scan.as_ref() else {
        readiness.analysis_complete = false;
        return readiness;
    };

    let Ok(report) = store.load_report(&scan.id) else {
        readiness.analysis_complete = false;
        return readiness;
    };

    if report.get("integrity_status").is_some() || report.get("findings").is_some() {
        readiness.analysis_complete = true;
    }

    if let Some(artifact) = scan.graph_artifact_path.as_ref() {
        if let Ok(conn) =
            Connection::open_with_flags(artifact, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        {
            if let Ok(Some(raw)) =
                graphengine_parsing::infrastructure::storage::parse_meta_store::read_parse_meta(
                    &conn,
                    ANALYSIS_STATUS_KEY,
                )
            {
                if let Ok(status) = serde_json::from_str::<AnalysisStatus>(&raw) {
                    readiness.analysis_complete = status.complete;
                    readiness.mode = Some(status.mode);
                    readiness.segments_pending = status.segments_pending;
                }
            }
        }
    }

    readiness
}

// The Err type is intentionally the full MCP error-response payload (message,
// next_tool hints, workspace_delta, agent_directive) that callers forward
// verbatim to the agent — it is the response, not an exceptional control-flow
// value, so its size is by design rather than a footgun to box.
#[allow(clippy::result_large_err)]
pub fn check_stale_snapshot(
    project: &ProjectDto,
    query_paths: &[String],
) -> Result<(), StaleSnapshotError> {
    let Some(scan) = project.latest_scan.as_ref() else {
        return Ok(());
    };
    let Some(completed_at) = scan.completed_at.as_deref() else {
        return Ok(());
    };
    let Some(root) = project_root(project) else {
        return Ok(());
    };

    let delta = compute_workspace_delta(Path::new(&root), completed_at, Some(query_paths))
        .unwrap_or_else(|_| WorkspaceDelta {
            since_scan_seconds: 0,
            dirty_paths: Vec::new(),
            buckets: Default::default(),
            intersects_query: None,
            agent_directive: None,
        });

    if let Some(hits) = delta.intersects_query.as_ref() {
        if !hits.is_empty() {
            let scan_id_short = scan.id.chars().take(8).collect::<String>();
            return Err(StaleSnapshotError {
                error: "STALE_SNAPSHOT",
                message: format!(
                    "{} changed since scan {} ({}s ago). Structural query refused.",
                    hits.join(", "),
                    scan_id_short,
                    delta.since_scan_seconds
                ),
                next_tool: vec!["gridseak_scan"],
                agent_directive: format!(
                    "Run gridseak_scan before citing callers, blast radius, or cycles for: {}",
                    hits.join(", ")
                ),
                workspace_delta: delta,
            });
        }
    }
    Ok(())
}

pub fn check_analysis_complete(readiness: &AnalysisReadiness) -> Result<(), AnalysisNotReadyError> {
    if readiness.analysis_complete {
        return Ok(());
    }
    Err(AnalysisNotReadyError {
        error: "ANALYSIS_INCOMPLETE",
        message: "Recommendations and headline metrics require analysis_complete. \
                  Graph tools may still work on the last snapshot."
            .into(),
        next_tool: vec![],
        analysis_readiness: readiness.clone(),
    })
}

pub fn symbol_file_path(conn: &Connection, symbol: &str) -> Option<String> {
    let resolution = crate::graph_queries::resolve_symbol_detailed(conn, symbol).ok()?;
    let loc_json: String = conn
        .query_row(
            "SELECT location FROM nodes WHERE id = ?1",
            [&resolution.node.id],
            |row| row.get(0),
        )
        .ok()?;
    serde_json::from_str::<serde_json::Value>(&loc_json)
        .ok()
        .and_then(|v| {
            v.get("file")
                .and_then(|f| f.as_str())
                .map(normalize_rel_path)
        })
}

pub fn enrich_envelope(
    mut envelope: Value,
    project: &ProjectDto,
    store: &ProjectStore,
    query_paths: Option<&[String]>,
    routing_hint: Option<RoutedTool>,
) -> Value {
    let readiness = load_analysis_readiness(store, project);
    if let Some(root) = project_root(project) {
        if let Some(scan) = project.latest_scan.as_ref() {
            if let Some(completed_at) = scan.completed_at.as_deref() {
                if let Ok(delta) =
                    compute_workspace_delta(Path::new(&root), completed_at, query_paths)
                {
                    envelope.as_object_mut().expect("envelope object").insert(
                        "workspace_delta".into(),
                        serde_json::to_value(delta).unwrap_or(json!({})),
                    );
                }
            }
            if let Some(artifact) = scan.graph_artifact_path.as_ref() {
                if let Ok(conn) = Connection::open_with_flags(
                    artifact,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                ) {
                    let dirty = envelope
                        .get("workspace_delta")
                        .and_then(|d| d.get("dirty_paths"))
                        .and_then(|p| p.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect::<Vec<_>>()
                        });
                    let dirty_ref = dirty.as_deref();
                    if let Ok(plan) =
                        graphengine_analysis::health::pipeline::predict_incremental_plan(
                            &conn, dirty_ref,
                        )
                    {
                        envelope.as_object_mut().expect("envelope object").insert(
                            "incremental_plan".into(),
                            serde_json::to_value(plan).unwrap_or(json!({})),
                        );
                    }
                }
            }
        }
    }
    envelope.as_object_mut().expect("envelope object").insert(
        "analysis_readiness".into(),
        serde_json::to_value(&readiness).unwrap_or(json!({})),
    );
    envelope.as_object_mut().expect("envelope object").insert(
        "analysis_provenance".into(),
        json!({
            "mode": readiness.mode,
            "complete": readiness.analysis_complete,
            "segments_pending": readiness.segments_pending,
        }),
    );
    if let Some(tool) = routing_hint {
        envelope.as_object_mut().expect("envelope object").insert(
            "routing_hint".into(),
            json!({
                "recommended_tool": tool.mcp_name(),
                "preconditions": route_preconditions_json(tool),
            }),
        );
    }
    envelope
}

fn route_preconditions_json(tool: RoutedTool) -> Value {
    use crate::intent_router::routing_table;
    let pre = routing_table()
        .into_iter()
        .find(|(_, t, _)| *t == tool)
        .map(|(_, _, p)| p)
        .unwrap_or(RoutePrecondition::Any);
    json!([match pre {
        RoutePrecondition::Any => "any",
        RoutePrecondition::RescanIfDirty => "rescan_if_dirty",
        RoutePrecondition::AnalysisComplete => "analysis_complete",
        RoutePrecondition::GraphReady => "graph_ready",
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_delta::{compute_workspace_delta, AnalysisReadiness};

    #[test]
    fn analysis_incomplete_blocks_recommendations() {
        let readiness = AnalysisReadiness {
            graph_ready: true,
            analysis_complete: false,
            mode: Some("segmented_sync".into()),
            segments_pending: vec!["HealthScore".into()],
        };
        let err = check_analysis_complete(&readiness).expect_err("incomplete analysis blocked");
        assert_eq!(err.error, "ANALYSIS_INCOMPLETE");
    }

    #[test]
    fn stale_delta_intersects_query_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/foo.rs"), "v1").expect("write");
        let completed_at = (chrono::Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
        std::fs::write(root.join("src/foo.rs"), "v2\n").expect("touch");
        let delta = compute_workspace_delta(root, &completed_at, Some(&["src/foo.rs".into()]))
            .expect("delta");
        let hits = delta.intersects_query.expect("intersects_query");
        assert!(!hits.is_empty(), "dirty foo.rs must intersect query");
    }
}
