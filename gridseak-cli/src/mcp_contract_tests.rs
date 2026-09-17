//! MCP envelope contract snapshots.
//!
//! Unknown provenance must not be silently promoted. Compiler edges must
//! appear as `tier_3`. The legend is the single-source object from parsing.

#[cfg(test)]
mod tests {
    use graphengine_parsing::domain::evidence_tier::{
        provenance_json_to_tier_id, tier_legend_json,
    };

    #[test]
    fn compiler_provenance_is_never_unknown() {
        assert_eq!(
            provenance_json_to_tier_id(r#"{"source":"Compiler"}"#),
            Some("tier_3")
        );
    }

    #[test]
    fn unknown_provenance_fails_the_contract() {
        assert!(provenance_json_to_tier_id(r#"{"source":"NotASource"}"#).is_none());
    }

    #[test]
    fn tier_legend_lists_compiler_on_tier_3() {
        let legend = tier_legend_json();
        let sources = legend["sources"]["tier_3"]
            .as_array()
            .expect("sources.tier_3");
        assert!(sources.iter().any(|s| s.as_str() == Some("Compiler")));
        assert!(sources.iter().any(|s| s.as_str() == Some("Lsp")));
        assert!(legend["tier_3"].as_str().unwrap().contains("compiler"));
    }

    #[test]
    fn required_mcp_tools_are_named() {
        let tools = [
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
            "gridseak_diff_impact",
            "gridseak_verify_claim",
            "gridseak_structural_invariants",
            "gridseak_record_judgment",
        ];
        assert_eq!(tools.len(), 18);
        for name in tools {
            assert!(name.starts_with("gridseak_"));
        }
    }
}
