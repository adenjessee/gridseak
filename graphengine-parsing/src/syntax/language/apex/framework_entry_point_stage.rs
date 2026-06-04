//! Apex framework entry-point propagation **stage wrapper**.
//!
//! Post-T5: invoked from
//! [`crate::syntax::language::apex::extractor::ApexExtractor::post_syntax_hooks`]
//! via the trait-method dispatch the orchestrator uses. Previously lived at
//! `application::use_cases::parse_repo::pipeline::framework_entry_point_propagation`.
//!
//! Runs after Visualforce extraction and before symbol-table build,
//! when all Apex class symbols are populated in `SyntaxResults` and the
//! synthetic VF body nodes have been appended. This stage walks every
//! user-declared class's inheritance chain and tags ancestor contract
//! methods (`start`/`execute`/`finish`/`handleInboundEmail`) with the
//! corresponding platform-interface entry-point tag whenever any
//! descendant in the chain declares `implements Database.Batchable`
//! (etc.). See
//! [`super::framework_entry_point_propagation`]
//! for the algorithm details and the Round 5 / R11 context.
//!
//! # Why here, not inside `syntax_extraction`
//!
//! Syntax extraction is per-file and has no cross-class view. The
//! abstract-base → concrete-subclass propagation is inherently
//! cross-class, so it runs once after the full `class_symbols`
//! registry is assembled.
//!
//! # No-op for non-Apex parses
//!
//! Short-circuits when `syntax_results.class_symbols` is empty.
//! Rust / Java / C# / TS / JS / Python / Go runs never touch this
//! path — class_symbols is Apex-only.

use anyhow::Result;
use tracing::{debug, info};

use crate::application::ports::SyntaxResults;
use crate::syntax::language::apex::class_symbol_codec::deserialise_class_symbols;
use crate::syntax::language::apex::framework_entry_point_propagation::{
    propagate_framework_entry_points, FrameworkPropagationStats,
};

/// Run the propagation stage. Mutates `syntax_results.symbols` in
/// place. Returns the propagation stats for orchestrator-level logging
/// and test assertions.
pub fn run(syntax_results: &mut SyntaxResults) -> Result<FrameworkPropagationStats> {
    if syntax_results.class_symbols.is_empty() {
        debug!("framework entry-point propagation: no class_symbols present, skipping stage");
        return Ok(FrameworkPropagationStats::default());
    }

    let class_symbols = deserialise_class_symbols(
        &syntax_results.class_symbols,
        "framework entry-point propagation",
    );
    if class_symbols.is_empty() {
        debug!(
            "framework entry-point propagation: no deserialisable class_symbols, skipping stage"
        );
        return Ok(FrameworkPropagationStats::default());
    }

    let stats = propagate_framework_entry_points(&class_symbols, &mut syntax_results.symbols);

    if stats.function_nodes_tagged > 0 || stats.classes_with_platform_interface > 0 {
        info!(
            "framework entry-point propagation: {} classes with platform interfaces, \
             {} contract-method targets, {} nodes newly tagged, {} already tagged",
            stats.classes_with_platform_interface,
            stats.contract_method_targets,
            stats.function_nodes_tagged,
            stats.function_nodes_already_tagged,
        );
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{
        Access, ApexClassSymbols, ApexMethod, ApexParameter, ApexTypeRef,
    };
    use crate::domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range};

    fn node(fqn: &str) -> Node {
        Node::new(
            NodeKind::Function,
            fqn.to_string(),
            Range {
                start_line: 0,
                start_char: 0,
                end_line: 1,
                end_char: 0,
                file: "/tmp/ws/x.cls".into(),
            },
            Provenance::new(ProvenanceSource::TreeSitter, Confidence::High),
        )
    }

    fn method_ctx(name: &str) -> ApexMethod {
        ApexMethod {
            name: name.into(),
            parameters: vec![ApexParameter {
                name: "bc".into(),
                ty: ApexTypeRef::Primitive {
                    name: "Database.BatchableContext".into(),
                },
            }],
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual: false,
            is_abstract: false,
        }
    }

    fn class_json(parent: Option<&str>, interfaces: &[&str], methods: Vec<ApexMethod>) -> String {
        let symbols = ApexClassSymbols {
            methods,
            parent_class: parent.map(|s| s.to_string()),
            implemented_interfaces: interfaces.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        serde_json::to_string(&symbols).unwrap()
    }

    #[test]
    fn stage_short_circuits_when_class_symbols_empty() {
        let mut results = SyntaxResults::new();
        let stats = run(&mut results).unwrap();
        assert_eq!(stats.function_nodes_tagged, 0);
        assert_eq!(stats.classes_with_platform_interface, 0);
    }

    #[test]
    fn stage_tags_abstract_base_execute_via_subclass_implements() {
        // Integration-shape test: the stage must be a thin wrapper
        // that drives the pure propagator against the results struct
        // correctly. If this passes, the orchestrator just needs to
        // call `run` at the right point in the pipeline.
        let mut results = SyntaxResults::new();
        results.class_symbols = vec![
            (
                "BaseBatch".into(),
                class_json(None, &[], vec![method_ctx("execute")]),
            ),
            (
                "ConcreteBatch".into(),
                class_json(Some("BaseBatch"), &["Database.Batchable"], vec![]),
            ),
        ];
        results.symbols.push(node(
            "/tmp/ws/BaseBatch.cls::BaseBatch::execute(Database.BatchableContext)",
        ));

        let stats = run(&mut results).unwrap();
        assert_eq!(stats.function_nodes_tagged, 1);

        let tags: Vec<String> = results.symbols[0]
            .properties
            .get("entry_points")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(tags, vec!["batchable".to_string()]);
    }

    #[test]
    fn stage_tolerates_malformed_class_symbols_row() {
        let mut results = SyntaxResults::new();
        results.class_symbols = vec![
            (
                "BaseBatch".into(),
                class_json(None, &[], vec![method_ctx("execute")]),
            ),
            ("Bad".into(), "not-json".into()),
            (
                "ConcreteBatch".into(),
                class_json(Some("BaseBatch"), &["Database.Batchable"], vec![]),
            ),
        ];
        results.symbols.push(node(
            "/tmp/ws/BaseBatch.cls::BaseBatch::execute(Database.BatchableContext)",
        ));

        let stats = run(&mut results).unwrap();
        // The malformed row is dropped, but the well-formed
        // Base/Concrete pair still drives the tag.
        assert_eq!(stats.function_nodes_tagged, 1);
    }
}
