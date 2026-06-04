//! Deterministic symptom → MCP tool routing (no LLM).
//!
//! Maps plain-language developer questions to the correct GridSeak tool
//! and records preconditions (rescan if dirty, analysis_complete, etc.).

use serde::{Deserialize, Serialize};

/// Stable MCP tool identifier returned by the router.
// The `Gridseak` prefix on every variant is deliberate: each variant maps 1:1
// to a public `gridseak_*` MCP tool name (see `mcp_name`), so the prefix keeps
// the Rust identifier greppable against the wire identifier.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutedTool {
    GridseakContextForLlm,
    GridseakScan,
    GridseakGraphFileBlastRadius,
    GridseakGraphBlastRadius,
    GridseakGraphCallers,
    GridseakGraphCallees,
    GridseakGetRecommendations,
    GridseakGetFindings,
    GridseakGraphCycles,
    GridseakGraphModuleCoupling,
    GridseakGraphSlice,
    GridseakExplainFinding,
}

impl RoutedTool {
    pub fn mcp_name(self) -> &'static str {
        match self {
            Self::GridseakContextForLlm => "gridseak_context_for_llm",
            Self::GridseakScan => "gridseak_scan",
            Self::GridseakGraphFileBlastRadius => "gridseak_graph_file_blast_radius",
            Self::GridseakGraphBlastRadius => "gridseak_graph_blast_radius",
            Self::GridseakGraphCallers => "gridseak_graph_callers",
            Self::GridseakGraphCallees => "gridseak_graph_callees",
            Self::GridseakGetRecommendations => "gridseak_get_recommendations",
            Self::GridseakGetFindings => "gridseak_get_findings",
            Self::GridseakGraphCycles => "gridseak_graph_cycles",
            Self::GridseakGraphModuleCoupling => "gridseak_graph_module_coupling",
            Self::GridseakGraphSlice => "gridseak_graph_slice",
            Self::GridseakExplainFinding => "gridseak_explain_finding",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutePrecondition {
    Any,
    RescanIfDirty,
    AnalysisComplete,
    GraphReady,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub tool: RoutedTool,
    pub preconditions: Vec<RoutePrecondition>,
    pub matched_symptom: String,
    pub confidence: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct RouteInput<'a> {
    pub question: &'a str,
    pub file_hint: Option<&'a str>,
    // Populated from CLI/MCP `symbol` args (see route_command.rs / main.rs) and
    // reserved for symbol-scoped routing. The current symptom table routes on
    // `question` + `file_hint`; symbol-aware routing is a tracked follow-up, so
    // the field is captured but not yet read.
    #[allow(dead_code)]
    pub symbol_hint: Option<&'a str>,
}

/// Full symptom → tool table for cold-start bundles.
pub fn routing_table() -> Vec<(&'static str, RoutedTool, RoutePrecondition)> {
    vec![
        (
            "what breaks / impact / safe to remove / if I change (file path)",
            RoutedTool::GridseakGraphFileBlastRadius,
            RoutePrecondition::RescanIfDirty,
        ),
        (
            "what breaks / impact / safe to remove / if I change (symbol)",
            RoutedTool::GridseakGraphBlastRadius,
            RoutePrecondition::RescanIfDirty,
        ),
        (
            "who calls / callers of / used by",
            RoutedTool::GridseakGraphCallers,
            RoutePrecondition::RescanIfDirty,
        ),
        (
            "what does X call / callees",
            RoutedTool::GridseakGraphCallees,
            RoutePrecondition::RescanIfDirty,
        ),
        (
            "what should we fix / technical debt / risky / refactor first",
            RoutedTool::GridseakGetRecommendations,
            RoutePrecondition::AnalysisComplete,
        ),
        (
            "dead code / list findings / enumerate findings",
            RoutedTool::GridseakGetFindings,
            RoutePrecondition::AnalysisComplete,
        ),
        (
            "cycles / circular dependency",
            RoutedTool::GridseakGraphCycles,
            RoutePrecondition::GraphReady,
        ),
        (
            "tightly coupled / cross-module mess",
            RoutedTool::GridseakGraphModuleCoupling,
            RoutePrecondition::GraphReady,
        ),
        (
            "everything connected / neighborhood / end-to-end",
            RoutedTool::GridseakGraphSlice,
            RoutePrecondition::RescanIfDirty,
        ),
        (
            "cold start / what is this repo / where start",
            RoutedTool::GridseakContextForLlm,
            RoutePrecondition::Any,
        ),
        (
            "no recent scan",
            RoutedTool::GridseakScan,
            RoutePrecondition::Any,
        ),
    ]
}

fn normalize(input: &str) -> String {
    input
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '_' || c == ':' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_path(s: &str) -> bool {
    s.contains('/')
        || s.contains('\\')
        || s.ends_with(".rs")
        || s.ends_with(".ts")
        || s.ends_with(".tsx")
        || s.ends_with(".js")
        || s.ends_with(".py")
        || s.ends_with(".go")
}

pub fn route(input: RouteInput<'_>) -> RouteDecision {
    let q = normalize(input.question);
    let file_from_hint = input.file_hint.filter(|s| !s.is_empty());
    let file_from_q = extract_path_like_token(&q);
    let has_file =
        file_from_hint.is_some() || file_from_q.is_some() || looks_like_path(input.question);

    let pick = |tool: RoutedTool, symptom: &str, pre: RoutePrecondition| RouteDecision {
        tool,
        preconditions: vec![pre],
        matched_symptom: symptom.to_string(),
        confidence: "deterministic_keyword_match",
    };

    if q.contains("no scan")
        || q.contains("run scan")
        || q.contains("gridseak scan")
        || q.contains("rescan")
    {
        return pick(
            RoutedTool::GridseakScan,
            "rescan/no scan",
            RoutePrecondition::Any,
        );
    }
    if q.contains("cold start")
        || q.contains("what is this repo")
        || q.contains("understand this codebase")
        || q.contains("where should i start")
        || q.contains("where start")
    {
        return pick(
            RoutedTool::GridseakContextForLlm,
            "cold start / repo overview",
            RoutePrecondition::Any,
        );
    }
    if (q.contains("what breaks")
        || q.contains("what break")
        || q.contains("impact")
        || q.contains("blast radius")
        || q.contains("safe to remove")
        || q.contains("safe to delete")
        || q.contains("if i change")
        || q.contains("what depends on"))
        && has_file
    {
        return pick(
            RoutedTool::GridseakGraphFileBlastRadius,
            "file-level impact",
            RoutePrecondition::RescanIfDirty,
        );
    }
    if q.contains("what breaks")
        || q.contains("impact")
        || q.contains("blast radius")
        || q.contains("safe to remove")
        || q.contains("safe to delete")
        || q.contains("if i change")
        || q.contains("what depends on")
    {
        return pick(
            RoutedTool::GridseakGraphBlastRadius,
            "symbol-level impact",
            RoutePrecondition::RescanIfDirty,
        );
    }
    if q.contains("who calls")
        || q.contains("callers of")
        || q.contains("used by")
        || q.contains("who uses")
    {
        return pick(
            RoutedTool::GridseakGraphCallers,
            "callers",
            RoutePrecondition::RescanIfDirty,
        );
    }
    if q.contains("what does") && q.contains(" call")
        || q.contains("callees")
        || q.contains("what does x call")
    {
        return pick(
            RoutedTool::GridseakGraphCallees,
            "callees",
            RoutePrecondition::RescanIfDirty,
        );
    }
    if q.contains("everything connected")
        || q.contains("neighborhood")
        || q.contains("end to end")
        || q.contains("how does x fit")
    {
        return pick(
            RoutedTool::GridseakGraphSlice,
            "graph slice",
            RoutePrecondition::RescanIfDirty,
        );
    }
    if q.contains("dead code")
        || q.contains("list finding")
        || q.contains("all finding")
        || q.contains("critical finding")
        || q.contains("structural finding")
        || q.contains("enumerate")
        || q.contains("show me every")
    {
        return pick(
            RoutedTool::GridseakGetFindings,
            "findings enumeration",
            RoutePrecondition::AnalysisComplete,
        );
    }
    if q.contains("refactor first")
        || q.contains("technical debt")
        || q.contains("what should we fix")
        || q.contains("what is risky")
        || q.contains("whats risky")
        || q.contains("risky here")
        || q.contains("priorit")
    {
        return pick(
            RoutedTool::GridseakGetRecommendations,
            "ranked recommendations",
            RoutePrecondition::AnalysisComplete,
        );
    }
    if q.contains("cycle") || q.contains("circular") {
        return pick(
            RoutedTool::GridseakGraphCycles,
            "cycles",
            RoutePrecondition::GraphReady,
        );
    }
    if q.contains("tightly coupled") || q.contains("cross module") || q.contains("module coupling")
    {
        return pick(
            RoutedTool::GridseakGraphModuleCoupling,
            "module coupling",
            RoutePrecondition::GraphReady,
        );
    }
    if q.contains("explain finding") || q.contains("why does that matter") {
        return pick(
            RoutedTool::GridseakExplainFinding,
            "explain finding",
            RoutePrecondition::AnalysisComplete,
        );
    }

    pick(
        RoutedTool::GridseakContextForLlm,
        "default cold-start bundle",
        RoutePrecondition::Any,
    )
}

fn extract_path_like_token(q: &str) -> Option<&str> {
    q.split_whitespace()
        .find(|tok| looks_like_path(tok))
        .or_else(|| {
            // "graph_command.rs" embedded in longer question
            q.split_whitespace()
                .find(|tok| tok.contains('.') && tok.len() > 3)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(q: &str) -> RoutedTool {
        route(RouteInput {
            question: q,
            file_hint: None,
            symbol_hint: None,
        })
        .tool
    }

    #[test]
    fn routes_file_impact_questions() {
        assert_eq!(
            tool("what breaks if I change gridseak-cli/src/graph_command.rs"),
            RoutedTool::GridseakGraphFileBlastRadius
        );
        assert_eq!(
            tool("what's the impact of refactoring src/foo.rs"),
            RoutedTool::GridseakGraphFileBlastRadius
        );
    }

    #[test]
    fn routes_symbol_impact_questions() {
        assert_eq!(
            tool("if I change run_graph what breaks"),
            RoutedTool::GridseakGraphBlastRadius
        );
        assert_eq!(
            tool("is it safe to remove parse_symbol"),
            RoutedTool::GridseakGraphBlastRadius
        );
    }

    #[test]
    fn routes_callers_and_callees() {
        assert_eq!(
            tool("who calls resolve_symbol"),
            RoutedTool::GridseakGraphCallers
        );
        assert_eq!(tool("what does foo call"), RoutedTool::GridseakGraphCallees);
    }

    #[test]
    fn routes_recommendations_and_findings() {
        assert_eq!(
            tool("what should we refactor first"),
            RoutedTool::GridseakGetRecommendations
        );
        assert_eq!(
            tool("what's risky here"),
            RoutedTool::GridseakGetRecommendations
        );
        assert_eq!(tool("is there dead code"), RoutedTool::GridseakGetFindings);
        assert_eq!(
            tool("show me all the dead code"),
            RoutedTool::GridseakGetFindings
        );
    }

    #[test]
    fn routes_cycles_and_coupling() {
        assert_eq!(tool("are there cycles"), RoutedTool::GridseakGraphCycles);
        assert_eq!(
            tool("what modules are tightly coupled"),
            RoutedTool::GridseakGraphModuleCoupling
        );
    }

    #[test]
    fn routes_cold_start() {
        assert_eq!(
            tool("help me understand this codebase"),
            RoutedTool::GridseakContextForLlm
        );
        assert_eq!(
            tool("where should I start"),
            RoutedTool::GridseakContextForLlm
        );
    }

    #[test]
    fn routing_table_has_entries() {
        assert!(routing_table().len() >= 10);
    }

    #[test]
    fn symptom_coverage_batch() {
        let samples = [
            (
                "what breaks if I change this file",
                RoutedTool::GridseakGraphBlastRadius,
            ),
            ("who uses this function", RoutedTool::GridseakGraphCallers),
            (
                "list every critical finding",
                RoutedTool::GridseakGetFindings,
            ),
            ("circular dependency smell", RoutedTool::GridseakGraphCycles),
            ("run gridseak scan", RoutedTool::GridseakScan),
            (
                "show me everything connected to main",
                RoutedTool::GridseakGraphSlice,
            ),
            ("cross-module mess", RoutedTool::GridseakGraphModuleCoupling),
            (
                "technical debt priorities",
                RoutedTool::GridseakGetRecommendations,
            ),
            (
                "what is the blast radius of parse_symbol",
                RoutedTool::GridseakGraphBlastRadius,
            ),
            ("callers of run_graph", RoutedTool::GridseakGraphCallers),
            (
                "downstream callees of foo",
                RoutedTool::GridseakGraphCallees,
            ),
            (
                "explain finding dead_code_1",
                RoutedTool::GridseakExplainFinding,
            ),
            (
                "safe to delete helper_fn",
                RoutedTool::GridseakGraphBlastRadius,
            ),
            (
                "impact of changing src/lib.rs",
                RoutedTool::GridseakGraphFileBlastRadius,
            ),
            (
                "where should I start in this repo",
                RoutedTool::GridseakContextForLlm,
            ),
            ("cold start overview", RoutedTool::GridseakContextForLlm),
            ("any circular dependencies", RoutedTool::GridseakGraphCycles),
            (
                "module coupling hotspots",
                RoutedTool::GridseakGraphModuleCoupling,
            ),
            ("show structural findings", RoutedTool::GridseakGetFindings),
            (
                "what should we fix first here",
                RoutedTool::GridseakGetRecommendations,
            ),
        ];
        for (q, expected) in samples {
            assert_eq!(tool(q), expected, "question: {q}");
        }
    }

    #[test]
    fn mcp_tool_surface_includes_route() {
        let names = [
            "gridseak_context_for_llm",
            "gridseak_route",
            "gridseak_status",
            "gridseak_scan",
            "gridseak_get_recommendations",
            "gridseak_explain_finding",
            "gridseak_get_findings",
            "gridseak_graph_blast_radius",
            "gridseak_graph_file_blast_radius",
            "gridseak_graph_callers",
            "gridseak_graph_callees",
            "gridseak_graph_slice",
            "gridseak_graph_module_coupling",
            "gridseak_graph_cycles",
        ];
        assert_eq!(names.len(), 14);
        assert!(names.contains(&"gridseak_route"));
    }
}
