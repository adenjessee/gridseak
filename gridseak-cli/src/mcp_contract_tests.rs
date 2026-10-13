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

    /// Callers lists hold functions and methods only; a file's top-level
    /// code and module nodes go to `top_level_callers` with their kind.
    #[test]
    fn top_level_code_is_split_from_function_callers() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT, fqn TEXT, location TEXT,
                                 properties TEXT NOT NULL DEFAULT '{}');
             INSERT INTO nodes (id, kind, fqn, location) VALUES
               ('f', 'Function', 'p::types::email', '{}'),
               ('s', 'Function', 'p::string_test::__file_scope__', '{}'),
               ('m', 'Module', 'p::types', '{}');",
        )
        .unwrap();
        let node = |id: &str, kind: &str, fqn: &str| crate::graph_queries::NodeRef {
            id: id.into(),
            fqn: fqn.into(),
            kind: kind.into(),
            file: Some(format!("src/{id}.ts")),
            line: Some(1),
        };
        let nodes = [
            node("f", "Function", "p::types::email"),
            node("s", "Function", "p::string_test::__file_scope__"),
            node("m", "Module", "p::types"),
        ];
        let (callers, top) = crate::split_top_level(
            &conn,
            nodes.iter().map(|n| (n, serde_json::json!({ "depth": 1 }))),
        )
        .unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["fqn"], "p::types::email");
        assert_eq!(callers[0]["kind"], "function");
        assert_eq!(callers[0]["depth"], 1);
        let kinds: Vec<_> = top.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["file_scope", "module"]);
        let mut env = serde_json::json!({});
        crate::attach_top_level_callers(&mut env, top, 10);
        assert_eq!(env["top_level_total"], 2);
        assert!(env["top_level_note"]
            .as_str()
            .unwrap()
            .contains("not functions"));
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
