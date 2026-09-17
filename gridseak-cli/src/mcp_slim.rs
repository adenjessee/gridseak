//! Plugin MCP: router + 4 verbs. Progressive disclosure — the other
//! fourteen tools stay on `gridseak mcp` (full), not in the always-on
//! plugin schema.

use gridseak_local_store::{LocalStorePaths, ProjectStore};
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};

use crate::gate::{compaction_card, default_ledger_path};
use crate::intent_router::{route, RouteInput};
use crate::mcp_preflight::load_analysis_readiness;

pub const SLIM_VERBS: &[&str] = &[
    "gridseak_route",
    "gridseak_verify_claim",
    "gridseak_graph_blast_radius",
    "gridseak_diff_impact",
    "gridseak_gate_status",
];

#[derive(Clone)]
pub struct SlimGridSeakMcp {
    store: ProjectStore,
    paths: LocalStorePaths,
    tool_router: ToolRouter<Self>,
}

impl SlimGridSeakMcp {
    pub fn new(store: ProjectStore, paths: LocalStorePaths) -> Self {
        Self {
            store,
            paths,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl SlimGridSeakMcp {
    #[tool(
        description = "Deterministic symptom router — maps a question to the correct GridSeak verb. Call this when unsure."
    )]
    async fn gridseak_route(
        &self,
        Parameters(params): Parameters<crate::RouteParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = self
            .store
            .resolve_project_lenient(&params.project)
            .map_err(crate::mcp_err)?;
        let decision = route(RouteInput {
            question: &params.question,
            file_hint: params.file.as_deref(),
            symbol_hint: params.symbol.as_deref(),
        });
        let readiness = load_analysis_readiness(&self.store, &project);
        let inner = serde_json::json!({
            "question": params.question,
            "recommended_tool": decision.tool.mcp_name(),
            "matched_symptom": decision.matched_symptom,
            "preconditions": decision.preconditions,
            "analysis_readiness": readiness,
            "slim_verbs": SLIM_VERBS,
        });
        crate::ok_json(&inner)
    }

    #[tool(
        description = "Verify a typed structural claim: calls(A,B) | no_callers(X) | reaches(A,B) | in_cycle(X) | dead(X). Returns verified | refuted | unknown plus witnesses."
    )]
    async fn gridseak_verify_claim(
        &self,
        Parameters(params): Parameters<crate::VerifyClaimParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = self
            .store
            .resolve_project_lenient(&params.project)
            .map_err(crate::mcp_err)?;
        let artifact = project
            .latest_scan
            .as_ref()
            .and_then(|s| s.graph_artifact_path.as_ref())
            .ok_or_else(|| crate::mcp_err("no graph artifact; run gridseak_scan"))?;
        let view = crate::graph_queries::agent_tools::verify_claim(
            std::path::Path::new(artifact),
            &params.claim,
        )
        .map_err(crate::mcp_err)?;
        crate::ok_json(&view)
    }

    #[tool(
        description = "If I change this function, what breaks? Transitive upstream callers. Pass a file path to use file blast instead (set file=)."
    )]
    async fn gridseak_graph_blast_radius(
        &self,
        Parameters(params): Parameters<BlastOrFileParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(file) = params.file.clone().filter(|s| !s.is_empty()) {
            let mapped = crate::GraphFileBlastRadiusParams {
                project: params.project.clone(),
                file,
                depth: params.depth,
                cap: params.cap,
            };
            let view = crate::graph_tool_file_blast_radius(&self.store, &mapped)?;
            return crate::ok_json(&view);
        }
        let symbol = params
            .symbol
            .clone()
            .ok_or_else(|| crate::mcp_err("pass symbol=FQN or file=path"))?;
        let mapped = crate::GraphBlastRadiusParams {
            project: params.project,
            symbol,
            depth: params.depth,
            cap: params.cap,
        };
        let view = crate::graph_tool_blast_radius(&self.store, &mapped)?;
        crate::ok_json(&view)
    }

    #[tool(
        description = "Changed files → affected callers with evidence tier and p_true. Use before committing a refactor."
    )]
    async fn gridseak_diff_impact(
        &self,
        Parameters(params): Parameters<crate::DiffImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = self
            .store
            .resolve_project_lenient(&params.project)
            .map_err(crate::mcp_err)?;
        let artifact = project
            .latest_scan
            .as_ref()
            .and_then(|s| s.graph_artifact_path.as_ref())
            .ok_or_else(|| crate::mcp_err("no graph artifact; run gridseak_scan"))?;
        let repo = project
            .roots
            .first()
            .map(|r| std::path::PathBuf::from(&r.path))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let view = crate::graph_queries::agent_tools::diff_impact(
            std::path::Path::new(artifact),
            &repo,
            params.base.as_deref(),
            params.head.as_deref(),
        )
        .map_err(crate::mcp_err)?;
        crate::ok_json(&view)
    }

    #[tool(
        description = "Gate register: compaction card + recent deny rows from ~/.gridseak/gate.jsonl. Call after compact."
    )]
    async fn gridseak_gate_status(
        &self,
        Parameters(params): Parameters<crate::ProjectRef>,
    ) -> Result<CallToolResult, McpError> {
        let _ = &self.paths;
        let card = compaction_card(&self.store, &params.project, None).map_err(crate::mcp_err)?;
        crate::ok_json(&serde_json::json!({
            "card": card,
            "ledger": default_ledger_path().display().to_string(),
            "verbs": SLIM_VERBS,
        }))
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BlastOrFileParams {
    #[serde(default = "crate::default_project_ref")]
    project: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    cap: Option<usize>,
}

#[tool_handler]
impl ServerHandler for SlimGridSeakMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "GridSeak slim plugin — router + verify_claim + blast_radius \
                 + diff_impact + gate_status. Deterministic, local, 0 LLM tokens. \
                 Quote the evidence tier. The gate (not this catalog) is the \
                 unskippable register."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn serve_slim(store: ProjectStore, paths: LocalStorePaths) -> anyhow::Result<()> {
    let service = SlimGridSeakMcp::new(store, paths)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SLIM_VERBS;

    #[test]
    fn slim_catalog_is_router_plus_four() {
        assert_eq!(SLIM_VERBS.len(), 5);
        assert!(SLIM_VERBS.contains(&"gridseak_route"));
        assert!(SLIM_VERBS.contains(&"gridseak_verify_claim"));
        assert!(SLIM_VERBS.contains(&"gridseak_graph_blast_radius"));
        assert!(SLIM_VERBS.contains(&"gridseak_diff_impact"));
        assert!(SLIM_VERBS.contains(&"gridseak_gate_status"));
    }

    #[test]
    fn plugin_manifest_lists_the_same_verbs() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin/plugin.json");
        let raw = std::fs::read_to_string(path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let verbs = v["alwaysOnVerbs"].as_array().expect("alwaysOnVerbs");
        for name in SLIM_VERBS {
            assert!(
                verbs.iter().any(|x| x.as_str() == Some(*name)),
                "plugin.json missing {name}"
            );
        }
    }

    fn plugin_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin")
    }

    #[test]
    fn cursor_plugin_hook_keys_cover_file_edits() {
        let path = plugin_root().join(".cursor-plugin/plugin.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let desc = v["description"].as_str().unwrap_or("");
        assert!(
            desc.contains("preToolUse"),
            "Cursor plugin must name preToolUse: {desc}"
        );
        assert!(
            !desc.contains("shell-only"),
            "Cursor plugin must not claim shell-only: {desc}"
        );
        assert!(
            raw.contains("beforeShellExecution"),
            "Cursor plugin missing beforeShellExecution"
        );
        assert!(
            raw.contains("preToolUse"),
            "Cursor plugin missing preToolUse"
        );
        assert!(
            raw.contains("Write|StrReplace|Delete|Shell"),
            "Cursor plugin missing Write|StrReplace|Delete|Shell matcher"
        );
        assert!(
            raw.contains("failClosed"),
            "Cursor plugin missing failClosed"
        );
        assert!(
            raw.contains("cursor-tool"),
            "Cursor plugin must call the cursor-tool host format"
        );
    }

    #[test]
    fn claude_plugin_hook_keys_cover_edits() {
        let path = plugin_root().join(".claude-plugin/plugin.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("PreToolUse"),
            "Claude plugin missing PreToolUse"
        );
        assert!(
            raw.contains("Edit|Write|MultiEdit"),
            "Claude plugin missing Edit|Write|MultiEdit matcher"
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let pre = &v["hooks"]["PreToolUse"];
        assert!(pre.is_array(), "Claude PreToolUse must be an array");
    }

    #[test]
    fn union_plugin_json_lists_both_host_hook_keys() {
        let path = plugin_root().join("plugin.json");
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.contains("beforeShellExecution"));
        assert!(raw.contains("failClosed"));
        assert!(raw.contains("PreToolUse"));
        assert!(raw.contains("preToolUse"));
        assert!(raw.contains("Edit|Write|MultiEdit"));
        assert!(raw.contains("Write|StrReplace|Delete|Shell"));
    }
}
