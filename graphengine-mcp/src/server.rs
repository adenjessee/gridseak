use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use graphengine_analysis::health::config::{AnalysisConfig, Ecosystem};
use graphengine_analysis::health::report::HealthReport;
use graphengine_analysis::validation::overrides::ValidationOverrides;

use crate::cache::ReportCache;

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GraphEngineServer {
    cache: Arc<ReportCache>,
    tool_router: ToolRouter<Self>,
}

impl GraphEngineServer {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(ReportCache::new()),
            tool_router: Self::tool_router(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ParseRepoParams {
    /// Absolute path to the repository root directory. Required if repo_url is not provided.
    #[serde(default)]
    pub root: Option<String>,
    /// Git URL to clone. If provided, the repo is cloned to a temp dir and used as root.
    #[serde(default)]
    pub repo_url: Option<String>,
    /// Git branch to clone (defaults to the repo's default branch).
    #[serde(default)]
    pub branch: Option<String>,
    /// Programming language: rust, typescript, javascript, python, go.
    pub lang: String,
    /// Optional output database path. Defaults to /data/<lang>-<ts>.sqlite for cloned repos,
    /// or <root>/<lang>.sqlite for local paths.
    #[serde(default)]
    pub db_path: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct DbPathParams {
    /// Path to the SQLite database produced by parse_repo.
    pub db_path: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ApplyOverridesParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// The overrides JSON object (file_overrides, entry_point_overrides, etc.).
    #[serde(default)]
    pub overrides: serde_json::Value,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzeParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// Ecosystem override (rust, typescript, python, go, javascript). Auto-detected if omitted.
    #[serde(default)]
    pub ecosystem: Option<String>,
    /// Path to .git directory for temporal coupling analysis.
    #[serde(default)]
    pub git_dir: Option<String>,
    /// Path to a JSON overrides file from user validation.
    #[serde(default)]
    pub overrides_path: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ModuleHealthParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// Module key (e.g. "graphengine-parsing/src/infrastructure/lsp").
    pub module_key: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct BlastRadiusParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// Fully qualified function name (e.g. "module::submodule::my_function").
    pub function_fqn: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CouplingHotspotsParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// Coupling score threshold (0.0–1.0). Defaults to 0.5.
    #[serde(default)]
    pub threshold: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct NodeAnnotationParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// Node ID (SHA hash) or fully qualified function name.
    pub node_id_or_fqn: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListFindingsParams {
    /// Path to the SQLite database.
    pub db_path: String,
    /// Filter by finding type (e.g. "CircularDependency", "PotentiallyUnreachable").
    #[serde(default)]
    pub finding_type: Option<String>,
    /// Filter by severity (Critical, High, Warning, Info).
    #[serde(default)]
    pub severity: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CompareReportsParams {
    /// Path to the first JSON health report (baseline).
    pub report_a_path: String,
    /// Path to the second JSON health report (current).
    pub report_b_path: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct TriageFindingParams {
    /// Path to the SQLite database (overrides file stored alongside).
    pub db_path: String,
    /// The finding ID to triage (e.g. "cycle-1", "dead-1").
    pub finding_id: String,
    /// Triage action: "acknowledged", "wont_fix", or "false_positive".
    pub action: String,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl GraphEngineServer {
    // ── Phase 1: Parse ────────────────────────────────────────────────

    #[tool(description = "List programming languages supported by the graphengine parser.")]
    async fn list_languages(&self) -> Result<CallToolResult, McpError> {
        let languages = vec!["rust", "typescript", "javascript", "python", "go"];
        ok_json(&languages)
    }

    #[tool(
        description = "Parse a source code repository into a semantic dependency graph stored in SQLite. \
        This is the first step — it extracts functions, modules, imports, calls, and types using \
        tree-sitter and LSP. Returns the database path and graph statistics. \
        May take 30s–10min depending on repo size and LSP availability."
    )]
    async fn parse_repo(
        &self,
        Parameters(params): Parameters<ParseRepoParams>,
    ) -> Result<CallToolResult, McpError> {
        let lang = params.lang.to_lowercase();
        let valid = ["rust", "typescript", "javascript", "python", "go"];
        if !valid.contains(&lang.as_str()) {
            return ok_err(format!(
                "Unsupported language '{}'. Supported: {:?}",
                lang, valid
            ));
        }

        // Resolve root: clone from URL or use provided local path
        let (_temp_dir, root) = if let Some(ref url) = params.repo_url {
            let tmp = tempfile::tempdir()
                .map_err(|e| mcp_err(format!("Failed to create temp dir: {e}")))?;

            let mut cmd = tokio::process::Command::new("git");
            cmd.arg("clone").arg("--depth=1");
            if let Some(ref branch) = params.branch {
                cmd.args(["--branch", branch]);
            }
            cmd.arg(url).arg(tmp.path());
            cmd.env("GIT_TERMINAL_PROMPT", "0");
            cmd.env("GIT_ASKPASS", "");
            cmd.env(
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o StrictHostKeyChecking=no",
            );

            tracing::info!("Cloning {} into {:?}", url, tmp.path());
            let output = cmd
                .output()
                .await
                .map_err(|e| mcp_err(format!("git clone failed to spawn: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return ok_err(format!("git clone failed: {stderr}"));
            }

            let p = tmp.path().to_path_buf();
            (Some(tmp), p)
        } else if let Some(ref root_str) = params.root {
            let p = PathBuf::from(root_str);
            if !p.exists() {
                return ok_err(format!("Root directory does not exist: {}", root_str));
            }
            (None, p)
        } else {
            return ok_err("Either 'root' or 'repo_url' must be provided".to_string());
        };

        // DB path: for cloned repos, write to /data/ so the DB outlives the temp dir
        let db_path = params.db_path.unwrap_or_else(|| {
            if params.repo_url.is_some() {
                let data_dir = std::env::var("ENGINE_DATA_DIR").unwrap_or_else(|_| "/data".into());
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                format!("{}/{}-{}.sqlite", data_dir, lang, ts)
            } else {
                root.join(format!("{}.sqlite", lang))
                    .to_string_lossy()
                    .to_string()
            }
        });

        let canonical_root = root
            .canonicalize()
            .map_err(|e| mcp_err(format!("Failed to canonicalize root: {e}")))?;

        let workspace_url = url::Url::from_file_path(&canonical_root).ok();
        let start = std::time::Instant::now();

        let use_case =
            graphengine_parsing::application::use_cases::ParseRepositoryUseCase::with_real_components_progress(
                lang.clone(),
                graphengine_parsing::domain::Confidence::Medium,
                &db_path,
                workspace_url,
                Arc::new(graphengine_progress::StdoutEngineEventEmitter::to_stdout(false)),
            )
            .await
            .map_err(|e| mcp_err(format!("Failed to create parser: {e}")))?;

        let result = use_case
            .parse(root.clone(), lang.clone())
            .await
            .map_err(|e| mcp_err(format!("Parse failed: {e}")))?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let graph = result.graph();
        self.cache.invalidate(&db_path);

        ok_json(&serde_json::json!({
            "db_path": db_path,
            "node_count": graph.node_count(),
            "edge_count": graph.edge_count(),
            "duration_ms": duration_ms,
            "language": lang,
        }))
    }

    // ── Phase 2: Validate ─────────────────────────────────────────────

    #[tool(
        description = "Get the pre-analysis validation payload for a parsed database. \
        Returns uncertain file classifications, dead code candidates, module boundary issues, \
        and repo type detection. The agent can inspect these and produce overrides."
    )]
    async fn get_validation(
        &self,
        Parameters(params): Parameters<DbPathParams>,
    ) -> Result<CallToolResult, McpError> {
        let db_path = params.db_path;
        let payload = tokio::task::spawn_blocking(move || {
            graphengine_analysis::validation::emit_validation_payload(&db_path)
        })
        .await
        .map_err(|e| mcp_err(format!("Task join error: {e}")))?
        .map_err(|e| mcp_err(format!("Validation payload failed: {e}")))?;

        ok_json(&payload)
    }

    #[tool(
        description = "Apply user/agent validation overrides to a parsed database. \
        Overrides are saved to a JSON file alongside the database and automatically \
        applied on subsequent analyze calls. Accepts file classification corrections, \
        entry point exemptions, module depth changes, repo type override, and finding triage."
    )]
    async fn apply_overrides(
        &self,
        Parameters(params): Parameters<ApplyOverridesParams>,
    ) -> Result<CallToolResult, McpError> {
        let overrides: ValidationOverrides = serde_json::from_value(params.overrides.clone())
            .map_err(|e| McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: format!("Invalid overrides JSON: {e}").into(),
                data: None,
            })?;

        let ov_path = overrides_path_for(&params.db_path);
        let json = serde_json::to_string_pretty(&overrides)
            .map_err(|e| mcp_err(format!("Serialization error: {e}")))?;

        std::fs::write(&ov_path, &json)
            .map_err(|e| mcp_err(format!("Failed to write overrides file: {e}")))?;

        self.cache.invalidate(&params.db_path);

        ok_json(&serde_json::json!({
            "status": "ok",
            "overrides_path": ov_path,
            "file_overrides": overrides.file_overrides.len(),
            "entry_point_overrides": overrides.entry_point_overrides.len(),
            "entry_point_patterns": overrides.entry_point_patterns.len(),
            "finding_triage": overrides.finding_triage.len(),
        }))
    }

    // ── Phase 3: Analyze ──────────────────────────────────────────────

    #[tool(
        description = "Run the full structural health analysis on a parsed database. \
        Returns the complete HealthReport JSON including health score, 9-component breakdown, \
        all metrics, findings, node annotations, and module annotations. \
        Automatically loads overrides from the companion .overrides.json file if present."
    )]
    async fn analyze(
        &self,
        Parameters(params): Parameters<AnalyzeParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .run_analysis(
                &params.db_path,
                params.ecosystem.as_deref(),
                params.git_dir.as_deref(),
                params.overrides_path.as_deref(),
            )
            .await?;
        self.cache.insert(&params.db_path, report.clone());
        ok_json(&report)
    }

    #[tool(
        description = "Get just the health score and its 9-component breakdown for a parsed database. \
        Faster to read than the full analyze output when you only need the score."
    )]
    async fn health_score(
        &self,
        Parameters(params): Parameters<DbPathParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;
        ok_json(&serde_json::json!({
            "health_score": report.health_score,
            "components": report.health_score_components,
        }))
    }

    #[tool(
        description = "Get the full annotation for a single module, including coupling, cohesion, \
        instability, abstractness, distance from main sequence, API surface ratio, and risk level."
    )]
    async fn module_health(
        &self,
        Parameters(params): Parameters<ModuleHealthParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;
        match report.module_annotations.get(&params.module_key) {
            Some(ann) => ok_json(ann),
            None => {
                let available: Vec<&String> = report.module_annotations.keys().collect();
                ok_err(format!(
                    "Module '{}' not found. Available modules: {:?}",
                    params.module_key, available
                ))
            }
        }
    }

    #[tool(
        description = "Get all dead code findings (functions with zero incoming call edges). \
        Returns the finding details plus the full node annotations for each dead function."
    )]
    async fn find_dead_code(
        &self,
        Parameters(params): Parameters<DbPathParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;
        let dead_findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| format!("{:?}", f.finding_type) == "PotentiallyUnreachable")
            .collect();
        let dead_nodes: Vec<_> = report
            .node_annotations
            .iter()
            .filter(|(_, ann)| ann.is_dead)
            .collect();
        ok_json(&serde_json::json!({
            "findings": dead_findings,
            "dead_node_count": dead_nodes.len(),
            "dead_nodes": dead_nodes,
        }))
    }

    #[tool(
        description = "Get all circular dependency (cycle) findings from the analyzed graph. \
        Each finding includes the cycle's node IDs and a human-readable description of the cycle path."
    )]
    async fn find_cycles(
        &self,
        Parameters(params): Parameters<DbPathParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;
        let cycle_findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| format!("{:?}", f.finding_type) == "CircularDependency")
            .collect();
        ok_json(&serde_json::json!({
            "cycle_count": cycle_findings.len(),
            "findings": cycle_findings,
        }))
    }

    #[tool(
        description = "Compute the transitive dependency impact (blast radius) for a single function. \
        Returns how many downstream nodes would be affected if this function changed."
    )]
    async fn blast_radius(
        &self,
        Parameters(params): Parameters<BlastRadiusParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;

        let found = report.node_annotations.iter().find(|(_, ann)| {
            ann.fqn == params.function_fqn || ann.display_name == params.function_fqn
        });

        match found {
            Some((id, ann)) => ok_json(&serde_json::json!({
                "node_id": id,
                "fqn": ann.fqn,
                "display_name": ann.display_name,
                "blast_radius": ann.blast_radius,
                "fan_in": ann.fan_in,
                "fan_out": ann.fan_out,
                "depth_from_root": ann.depth_from_root,
                "is_hotspot": ann.is_hotspot,
                "risk_level": format!("{:?}", ann.risk_level),
            })),
            None => ok_err(format!(
                "Function '{}' not found in node annotations. Use a fully qualified name or display name.",
                params.function_fqn,
            )),
        }
    }

    #[tool(description = "Find modules with coupling above a threshold. \
        High coupling means a module depends heavily on external modules. Threshold defaults to 0.5.")]
    async fn find_coupling_hotspots(
        &self,
        Parameters(params): Parameters<CouplingHotspotsParams>,
    ) -> Result<CallToolResult, McpError> {
        let threshold = params.threshold.unwrap_or(0.5);
        let report = self.get_or_analyze(&params.db_path).await?;
        let hotspots: Vec<_> = report
            .module_annotations
            .iter()
            .filter(|(_, ann)| ann.coupling_score > threshold)
            .map(|(key, ann)| {
                serde_json::json!({
                    "module_key": key,
                    "coupling_score": ann.coupling_score,
                    "internal_edges": ann.internal_edges,
                    "external_edges": ann.external_edges,
                    "is_production": ann.is_production,
                })
            })
            .collect();
        ok_json(&serde_json::json!({
            "threshold": threshold,
            "count": hotspots.len(),
            "hotspots": hotspots,
        }))
    }

    // ── Phase 4: Query ────────────────────────────────────────────────

    #[tool(
        description = "Get the full annotation for a single function node, including fan-in, \
        fan-out, blast radius, depth, complexity, and risk level. \
        Accepts either a node ID (SHA hash) or a fully qualified function name."
    )]
    async fn node_annotation(
        &self,
        Parameters(params): Parameters<NodeAnnotationParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;
        let key = &params.node_id_or_fqn;

        if let Some(ann) = report.node_annotations.get(key) {
            return ok_json(ann);
        }

        let found = report
            .node_annotations
            .iter()
            .find(|(_, ann)| ann.fqn == *key || ann.display_name == *key);

        match found {
            Some((id, ann)) => ok_json(&serde_json::json!({
                "node_id": id,
                "annotation": ann,
            })),
            None => ok_err(format!(
                "Node '{}' not found. Provide a node ID, FQN, or display name.",
                key
            )),
        }
    }

    #[tool(
        description = "List findings from the analysis report, optionally filtered by type and/or severity. \
        Finding types: CircularDependency, HighCoupling, PotentiallyUnreachable, DeepCallChain, \
        BlastRadiusHotspot, LowCohesion, GodFunction, LayerViolation, TemporalCoupling, etc. \
        Severities: Critical, High, Warning, Info."
    )]
    async fn list_findings(
        &self,
        Parameters(params): Parameters<ListFindingsParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.get_or_analyze(&params.db_path).await?;
        let mut findings: Vec<_> = report.findings.iter().collect();

        if let Some(ref ft) = params.finding_type {
            let ft_lower = ft.to_lowercase();
            findings.retain(|f| {
                format!("{:?}", f.finding_type)
                    .to_lowercase()
                    .contains(&ft_lower)
            });
        }
        if let Some(ref sev) = params.severity {
            let sev_lower = sev.to_lowercase();
            findings.retain(|f| format!("{:?}", f.severity).to_lowercase() == sev_lower);
        }

        ok_json(&serde_json::json!({
            "count": findings.len(),
            "findings": findings,
        }))
    }

    #[tool(
        description = "Compare two health reports and show what improved, regressed, or stayed the same. \
        Provide paths to two JSON report files (e.g. before and after a refactor)."
    )]
    async fn compare_reports(
        &self,
        Parameters(params): Parameters<CompareReportsParams>,
    ) -> Result<CallToolResult, McpError> {
        let report_a = load_report(&params.report_a_path)?;
        let report_b = load_report(&params.report_b_path)?;

        let score_a = report_a.health_score.unwrap_or(0) as i32;
        let score_b = report_b.health_score.unwrap_or(0) as i32;
        let delta = score_b - score_a;

        let compare_component = |a: u32, b: u32, name: &str| -> serde_json::Value {
            let d = b as i32 - a as i32;
            serde_json::json!({
                "name": name, "before": a, "after": b, "delta": d,
                "trend": if d > 0 { "improved" } else if d < 0 { "regressed" } else { "unchanged" },
            })
        };

        let ca = &report_a.health_score_components;
        let cb = &report_b.health_score_components;
        let components = vec![
            compare_component(
                ca.cycle_severity.score,
                cb.cycle_severity.score,
                "cycle_severity",
            ),
            compare_component(
                ca.coupling_health.score,
                cb.coupling_health.score,
                "coupling_health",
            ),
            compare_component(
                ca.hotspot_concentration.score,
                cb.hotspot_concentration.score,
                "hotspot_concentration",
            ),
            compare_component(
                ca.dead_code_ratio.score,
                cb.dead_code_ratio.score,
                "dead_code_ratio",
            ),
            compare_component(
                ca.depth_complexity.score,
                cb.depth_complexity.score,
                "depth_complexity",
            ),
            compare_component(ca.complexity.score, cb.complexity.score, "complexity"),
            compare_component(ca.cohesion.score, cb.cohesion.score, "cohesion"),
            compare_component(ca.distance.score, cb.distance.score, "distance"),
            compare_component(
                ca.temporal_coupling.score,
                cb.temporal_coupling.score,
                "temporal_coupling",
            ),
        ];

        ok_json(&serde_json::json!({
            "score_before": score_a, "score_after": score_b, "score_delta": delta,
            "trend": if delta > 0 { "improved" } else if delta < 0 { "regressed" } else { "unchanged" },
            "findings_before": report_a.findings.len(),
            "findings_after": report_b.findings.len(),
            "components": components,
            "summary_before": report_a.summary,
            "summary_after": report_b.summary,
        }))
    }

    #[tool(
        description = "Triage a finding: mark it as acknowledged, wont_fix, or false_positive. \
        The triage is stored in the overrides file and suppressed in future analysis runs."
    )]
    async fn triage_finding(
        &self,
        Parameters(params): Parameters<TriageFindingParams>,
    ) -> Result<CallToolResult, McpError> {
        let valid_actions = ["acknowledged", "wont_fix", "false_positive"];
        if !valid_actions.contains(&params.action.as_str()) {
            return ok_err(format!(
                "Invalid action '{}'. Must be one of: {:?}",
                params.action, valid_actions
            ));
        }

        let ov_path = overrides_path_for(&params.db_path);
        let mut overrides = load_or_default_overrides(&ov_path);

        overrides
            .finding_triage
            .retain(|t| t.finding_id != params.finding_id);
        overrides
            .finding_triage
            .push(graphengine_analysis::validation::overrides::FindingTriage {
                finding_id: params.finding_id.clone(),
                action: match params.action.as_str() {
                    "acknowledged" => {
                        graphengine_analysis::validation::overrides::TriageAction::Acknowledged
                    }
                    "wont_fix" => {
                        graphengine_analysis::validation::overrides::TriageAction::WontFix
                    }
                    _ => graphengine_analysis::validation::overrides::TriageAction::FalsePositive,
                },
                reason: None,
            });

        let json = serde_json::to_string_pretty(&overrides)
            .map_err(|e| mcp_err(format!("Serialization error: {e}")))?;

        std::fs::write(&ov_path, &json)
            .map_err(|e| mcp_err(format!("Failed to write overrides: {e}")))?;

        self.cache.invalidate(&params.db_path);

        ok_json(&serde_json::json!({
            "status": "ok",
            "finding_id": params.finding_id,
            "action": params.action,
            "overrides_path": ov_path,
        }))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — MCP metadata + capabilities
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for GraphEngineServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "GraphEngine: a structural health analysis engine for source code repositories. \
                 Workflow: list_languages → parse_repo → (optional) get_validation + apply_overrides → \
                 analyze → query tools (health_score, find_dead_code, find_cycles, blast_radius, etc.). \
                 Parse produces a SQLite database; analyze produces a JSON health report with a \
                 composite score out of 100."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl GraphEngineServer {
    async fn get_or_analyze(&self, db_path: &str) -> Result<HealthReport, McpError> {
        if let Some(report) = self.cache.get(db_path) {
            return Ok(report);
        }
        let report = self.run_analysis(db_path, None, None, None).await?;
        self.cache.insert(db_path, report.clone());
        Ok(report)
    }

    async fn run_analysis(
        &self,
        db_path: &str,
        ecosystem: Option<&str>,
        git_dir: Option<&str>,
        overrides_path: Option<&str>,
    ) -> Result<HealthReport, McpError> {
        if !std::path::Path::new(db_path).exists() {
            return Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: format!("Database not found: {db_path}").into(),
                data: None,
            });
        }

        let config =
            ecosystem.map(|eco| AnalysisConfig::for_ecosystem(Ecosystem::from_language_str(eco)));

        let explicit_overrides_path = overrides_path.map(|s| s.to_string());
        let companion_overrides_path = overrides_path_for(db_path);

        let overrides = if let Some(ref p) = explicit_overrides_path {
            match graphengine_analysis::validation::overrides::load_overrides(p) {
                Ok(o) => Some(o),
                Err(e) => {
                    return Err(McpError {
                        code: ErrorCode::INVALID_PARAMS,
                        message: format!("Invalid overrides file: {e}").into(),
                        data: None,
                    });
                }
            }
        } else if std::path::Path::new(&companion_overrides_path).exists() {
            graphengine_analysis::validation::overrides::load_overrides(&companion_overrides_path)
                .ok()
        } else {
            None
        };

        let db = db_path.to_string();
        let git = git_dir.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            graphengine_analysis::health::run_analysis_with_config(
                &db,
                config,
                None,
                git.as_deref(),
                overrides.as_ref(),
            )
        })
        .await
        .map_err(|e| mcp_err(format!("Task join error: {e}")))?
        .map_err(|e| mcp_err(format!("Analysis failed: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

fn mcp_err(msg: impl Into<String>) -> McpError {
    McpError {
        code: ErrorCode::INTERNAL_ERROR,
        message: msg.into().into(),
        data: None,
    }
}

fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json =
        serde_json::to_string_pretty(value).map_err(|e| mcp_err(format!("JSON error: {e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

fn ok_err(msg: impl Into<String>) -> Result<CallToolResult, McpError> {
    let mut result = CallToolResult::success(vec![Content::text(msg.into())]);
    result.is_error = Some(true);
    Ok(result)
}

fn overrides_path_for(db_path: &str) -> String {
    if let Some(stem) = db_path.strip_suffix(".sqlite") {
        format!("{stem}.overrides.json")
    } else if let Some(stem) = db_path.strip_suffix(".db") {
        format!("{stem}.overrides.json")
    } else {
        format!("{db_path}.overrides.json")
    }
}

fn load_or_default_overrides(path: &str) -> ValidationOverrides {
    if std::path::Path::new(path).exists() {
        graphengine_analysis::validation::overrides::load_overrides(path).unwrap_or_default()
    } else {
        ValidationOverrides::default()
    }
}

fn load_report(path: &str) -> Result<HealthReport, McpError> {
    let content = std::fs::read_to_string(path).map_err(|e| McpError {
        code: ErrorCode::INVALID_PARAMS,
        message: format!("Cannot read report file '{}': {e}", path).into(),
        data: None,
    })?;
    serde_json::from_str(&content).map_err(|e| McpError {
        code: ErrorCode::INVALID_PARAMS,
        message: format!("Invalid report JSON in '{}': {e}", path).into(),
        data: None,
    })
}
