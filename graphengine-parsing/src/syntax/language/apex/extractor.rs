//! Apex-specific [`LanguageSpecificExtractor`] implementation.
//!
//! Apex's syntax is Java-like with extra semantics (triggers, SOQL, sharing
//! modifiers, annotations). Today this impl mirrors the Java surface so that
//! cyclomatic/cognitive complexity is computed correctly from the tree-sitter
//! Apex grammar (which shares node-kind names with tree-sitter-java).
//!
//! Apex-only semantics (annotation entry points, trigger context vars,
//! `with sharing` metadata, `@IsTest` detection) are introduced in Sprint B
//! via additional methods wired into this struct and its helpers in
//! `syntax/language/apex/`.

use std::collections::BTreeSet;

use tree_sitter::{Node, Tree};

use crate::domain::{Confidence, Node as DomainNode, NodeKind, Provenance, ProvenanceSource};
use crate::syntax::language::apex::{
    class_symbols_extractor, coverage as apex_coverage, entry_points, field_body_synth,
    fqn as apex_fqn, local_var_extractor, managed_packages, sharing, trigger_metadata,
};
use crate::syntax::language::extractor::{
    binary_operator_text, ExternalReferenceResult, LanguageSpecificExtractor,
};
use crate::syntax::utils::{apex_test_detector, node_converter::node_to_range};

#[derive(Debug, Default)]
pub struct ApexExtractor;

impl LanguageSpecificExtractor for ApexExtractor {
    fn language(&self) -> &str {
        "apex"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        // tree-sitter-sfapex reuses Java-family node names.
        matches!(
            kind,
            "method_declaration" | "constructor_declaration" | "lambda_expression"
        )
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        // tree-sitter-sfapex emits `switch_label` once per `when` arm
        // (verified empirically — see `apex_switch_rule_each_adds_one_path`).
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "enhanced_for_statement"
                | "while_statement"
                | "do_statement"
                | "switch_label"
                | "catch_clause"
                | "ternary_expression"
        )
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "enhanced_for_statement"
                | "while_statement"
                | "do_statement"
                | "switch_expression"
                | "switch_statement"
                | "try_statement"
                | "catch_clause"
                | "ternary_expression"
        )
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        matches!(
            kind,
            "break_statement" | "continue_statement" | "throw_statement"
        )
    }

    fn is_logical_operator_node(&self, node: &Node, source: &[u8]) -> bool {
        if node.kind() != "binary_expression" {
            return false;
        }
        binary_operator_text(node, source)
            .map(|op| op == "&&" || op == "||")
            .unwrap_or(false)
    }

    fn is_continuation_if(&self, node: &Node) -> bool {
        if node.kind() != "if_statement" {
            return false;
        }
        let parent_is_if = node
            .parent()
            .map(|p| p.kind() == "if_statement")
            .unwrap_or(false);
        if !parent_is_if {
            return false;
        }
        node.prev_named_sibling()
            .map(|s| s.kind() == "block")
            .unwrap_or(false)
    }

    fn is_test_symbol(&self, node: &Node, source: &[u8]) -> bool {
        apex_test_detector::is_apex_test(node, source)
    }

    fn is_test_function(&self, node: &Node, source: &[u8]) -> bool {
        // Apex functions use the same marker set as symbols: `@IsTest`
        // on the method or an enclosing class, or the `testMethod` keyword.
        apex_test_detector::is_apex_test(node, source)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        // Apex interfaces are declared with the `interface` keyword; Salesforce
        // hover formatters surface them verbatim.
        type_string.contains("interface") || type_string.ends_with("Interface")
    }

    fn entry_point_tags(
        &self,
        node: &Node,
        source: &[u8],
        annotation_query: Option<&str>,
    ) -> Vec<&'static str> {
        entry_points::classify_with_annotation_query(node, source, annotation_query)
            .into_iter()
            .map(|k| k.as_str())
            .collect()
    }

    /// Apex FQN override (Sprint E.2).
    ///
    /// Dispatches on the declaration node's kind:
    ///
    /// - Method / constructor declarations get their enclosing-type
    ///   dotted path plus a parenthesised parameter-type signature so
    ///   overloads and sibling-inner-class collisions are resolved.
    /// - Type declarations (class / interface / enum / trigger) get the
    ///   enclosing-type dotted path so inner classes no longer share
    ///   an FQN with enclosing-class methods of the same simple name.
    ///
    /// Any other node kind defers to the shared builder (returns
    /// `None`). The Apex syntax extractor pipeline only passes
    /// declaration nodes to this hook, so a fallthrough primarily
    /// covers defensive correctness if the caller ever broadens the
    /// contract.
    fn build_symbol_fqn(
        &self,
        node: &Node,
        source: &[u8],
        simple_name: &str,
        file_path: &str,
        workspace_root: Option<&str>,
    ) -> Option<String> {
        match node.kind() {
            "method_declaration" | "constructor_declaration" => Some(apex_fqn::build_method_fqn(
                node,
                source,
                simple_name,
                file_path,
                workspace_root,
            )),
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "trigger_declaration" => Some(apex_fqn::build_type_fqn(
                node,
                source,
                simple_name,
                file_path,
                workspace_root,
            )),
            _ => None,
        }
    }

    /// Apex trigger-body synthesis (Sprint E.3).
    ///
    /// Apex triggers have no enclosing method in source — their top
    /// level is a bare statement block on the `trigger_declaration`
    /// node. Without a caller `Function` in the graph, every call site
    /// inside a trigger body loses its `Call` edge because the
    /// heuristic resolver's "smallest enclosing function" lookup
    /// returns nothing.
    ///
    /// We repair this by synthesising one `Function` node per trigger
    /// that covers the `trigger_body` range. The function is named
    /// `__trigger__` (double-underscore marks it as compiler-owned;
    /// no legal Apex identifier starts and ends with `__`), tagged
    /// `synthetic = true`, and given an FQN of shape
    /// `<path>::<TriggerName>::__trigger__()` so it reconciles with
    /// jorje's method-FQN convention.
    ///
    /// Called for every extracted symbol; returns empty for non-trigger
    /// declarations so the cost for classes/interfaces/enums is one
    /// kind comparison.
    fn synthesize_symbol_siblings(
        &self,
        node: &Node,
        source: &[u8],
        parent: &DomainNode,
        file_path: &str,
        workspace_root: Option<&str>,
    ) -> Vec<DomainNode> {
        // R41 (field initializers) + R39 (property accessor bodies):
        // for every `class_declaration` we synthesize one `Function`
        // node per orphan body so the heuristic resolver's
        // `find_enclosing_function` has a caller to attribute
        // enclosed call sites to. Non-class declarations fall through
        // to the trigger-body branch below.
        if node.kind() == "class_declaration" {
            return field_body_synth::synthesize_field_body_functions(
                node,
                source,
                parent,
                file_path,
                workspace_root,
            );
        }
        if node.kind() != "trigger_declaration" {
            return Vec::new();
        }
        let Some(body) = node.child_by_field_name("body") else {
            return Vec::new();
        };

        let body_range = node_to_range(&body, file_path);
        let fqn = apex_fqn::build_trigger_body_fqn(node, source, file_path, workspace_root);
        let provenance = Provenance::new(ProvenanceSource::TreeSitter, Confidence::High);
        // T2: hash the trigger body so the synthetic `__trigger__` node is
        // content-stable across blank-line / comment edits inside the
        // trigger body.
        let mut synthetic = match body.utf8_text(source).ok() {
            Some(body_text) => DomainNode::with_body(
                NodeKind::Function,
                fqn,
                body_range,
                provenance,
                body_text,
                Some("apex"),
            ),
            None => DomainNode::new(NodeKind::Function, fqn, body_range, provenance),
        };
        synthetic.set_property("synthetic", true);
        synthetic.set_property("synthetic_kind", "apex_trigger_body");
        synthetic.set_property("parent_trigger_id", parent.id.clone());
        if let Some(sobj) = parent.properties.get("sobject").and_then(|v| v.as_str()) {
            synthetic.set_property("sobject", sobj.to_string());
        }
        vec![synthetic]
    }

    fn extract_struct_metadata(
        &self,
        node: &Node,
        source: &[u8],
    ) -> Vec<(&'static str, serde_json::Value)> {
        let Some(model) = sharing::classify(node, source) else {
            return Vec::new();
        };
        vec![(
            "apex_sharing",
            serde_json::Value::String(model.as_str().to_string()),
        )]
    }

    /// Apex trigger-event surfacing (Sprint E.4).
    ///
    /// Runs the YAML-defined `trigger_events` query scoped to the
    /// given `trigger_declaration` node and returns
    /// `("trigger_events", Value::Array([...]))` when any events are
    /// found. Returning `Vec::new()` on zero events keeps the node
    /// properties tight — consumers treat "absent" and "[]" the same
    /// and the scan output stays free of empty arrays.
    fn extract_trigger_metadata(
        &self,
        trigger_node: &Node,
        source: &[u8],
        events_query: &str,
        language: tree_sitter::Language,
    ) -> Vec<(&'static str, serde_json::Value)> {
        let events = trigger_metadata::extract_events(trigger_node, source, events_query, language);
        if events.is_empty() {
            return Vec::new();
        }
        vec![(
            "trigger_events",
            serde_json::Value::Array(events.into_iter().map(serde_json::Value::String).collect()),
        )]
    }

    /// Apex-specific external-reference synthesis.
    ///
    /// Runs the managed-package detector, dedups the namespaces it finds
    /// in this file, synthesizes one virtual `Module` node per unique
    /// namespace (stable id via SHA-hashed FQN, so two files that both
    /// reference `npsp` converge on one graph node), and emits one
    /// `Import` edge from the enclosing file-module consumer node to
    /// each namespace.
    ///
    /// This is the production-pipeline counterpart to the
    /// `synthesize_module_node` / `synthesize_import_edge` helpers that
    /// already had unit-test coverage but were previously un-wired.
    fn synthesize_external_references(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
        consumer_node_id: &str,
    ) -> ExternalReferenceResult {
        let sites = managed_packages::extract(tree, source, file_path);
        if sites.is_empty() {
            return ExternalReferenceResult::default();
        }

        let namespaces: BTreeSet<String> = sites.into_iter().map(|site| site.namespace).collect();

        let mut result = ExternalReferenceResult::default();
        for ns in namespaces {
            let module_node = managed_packages::synthesize_module_node(&ns);
            let edge = managed_packages::synthesize_import_edge(
                consumer_node_id.to_string(),
                module_node.id.clone(),
            );
            result.nodes.push(module_node);
            result.edges.push(edge);
        }
        result
    }

    /// TR-A.0: extract `ApexClassSymbols` for every class / interface /
    /// enum declared in this `.cls` file and serialise them as
    /// `(dotted_api_name, json)` tuples.
    ///
    /// The dotted api name is keyed from the file stem for top-level
    /// declarations (Apex's contract: one top-level class per `.cls`,
    /// named the same as the file). Inner declarations emit nested
    /// names (`Outer.Inner`). `.trigger` files are explicitly skipped
    /// — their implicit `Trigger.new` / `Trigger.old` context vars
    /// need a sibling `TriggerSymbols` type that is deferred to
    /// Phase B (see `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`
    /// §4.11 triggers-scoped-out note).
    ///
    /// Returns an empty vec for any file the extractor decides to
    /// skip (triggers, files whose stem can't be recovered), which
    /// is the byte-identical-safe default — an empty return value
    /// cannot alter downstream graph shape.
    fn extract_class_symbols(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<(String, String)> {
        // TR-A.0 scopes triggers OUT — they carry implicit context
        // variables that `ApexClassSymbols` doesn't model. Phase B
        // introduces a `TriggerSymbols` sibling type; until then
        // returning early here is the honest behaviour (rather than
        // emitting a partial shape that the resolver would misuse).
        if file_path
            .rsplit('.')
            .next()
            .map(|ext| ext.eq_ignore_ascii_case("trigger"))
            .unwrap_or(false)
        {
            return Vec::new();
        }

        let top_level = file_stem(file_path);
        if top_level.is_empty() {
            return Vec::new();
        }

        let root = tree.root_node();
        let decls = class_symbols_extractor::extract_class_declarations(&root, source, &top_level);

        decls
            .into_iter()
            .filter_map(|(api_name, symbols)| {
                // `serde_json::to_string` produces stable output for
                // this type (no `HashMap`, only `Vec<_>` / `Option<_>`
                // / primitives / enums with `#[serde(tag = ...)]`).
                // That stability is what keeps the
                // `apex_class_symbols` table byte-identical across
                // repeated parses of the same source.
                serde_json::to_string(&symbols)
                    .ok()
                    .map(|json| (api_name, json))
            })
            .collect()
    }

    /// TR-A.1: Apex is the only language that owns signature-matching
    /// resolver arms in Phase A (constructor overload dispatch today;
    /// method overload dispatch in TR-A.4). Route `@args` captures
    /// from the shared call-site extractor through
    /// [`super::arg_type_inferrer::infer_arg_types`] so
    /// `CallSite.arg_types` is populated with literal /
    /// constructor-expression types for every Apex call site whose
    /// YAML query captures `argument_list`. Identifier-typed
    /// arguments land as `Unresolved` and are treated as wildcards at
    /// signature-match time (never drop a candidate on unknowns).
    /// Identifier- / field-type inference lands in TR-A.3 (§4.1).
    fn infer_call_site_arg_types(
        &self,
        args_node: &tree_sitter::Node<'_>,
        source: &[u8],
    ) -> Vec<crate::domain::apex::class_symbols::ApexTypeRef> {
        super::arg_type_inferrer::infer_arg_types(args_node, source)
    }

    /// TR-A.3: per-method local-variable scopes, consumed by the
    /// field-type-aware dispatch resolver. Walks the same parsed tree
    /// already produced by earlier extraction phases — no additional
    /// parse cost. Triggers are scoped out here for the same reason
    /// class-symbol extraction scopes them out: a `.trigger` file's
    /// implicit context variables live in a dedicated Phase B shape
    /// (`TriggerSymbols`), not in `ApexClassSymbols` / `LocalVarScope`.
    fn extract_file_coverage(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Option<crate::application::ports::FileExtractionCoverage> {
        // Triggers are scoped out of the Apex extractor's class-level
        // passes for the same reason as `extract_class_symbols` —
        // their context-variable semantics are not modelled yet. We
        // still emit a coverage record for them so downstream
        // classifiers can decide; the R39 / R41 counters do not fire
        // on trigger bodies (those gaps will be introduced via
        // `CoverageGap::ApexTriggerBodyUncaptured` when the emitter
        // lands — see T8 §8 out-of-scope follow-ups).
        Some(apex_coverage::extract_apex_file_coverage(
            tree, source, file_path,
        ))
    }

    fn extract_local_var_scopes(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<crate::application::ports::LocalVarScope> {
        if file_path
            .rsplit('.')
            .next()
            .map(|ext| ext.eq_ignore_ascii_case("trigger"))
            .unwrap_or(false)
        {
            return Vec::new();
        }
        local_var_extractor::extract_local_var_scopes(tree, source, file_path)
    }

    /// T5 — Apex-specific post-syntax hooks.
    ///
    /// Composes the two Apex-only pipeline stages that previously lived
    /// behind hardcoded orchestrator calls:
    ///
    /// 1. [`vf_extraction_stage::run`] — Visualforce-page extraction
    ///    (TR-A.5). Emits a container Struct + `__vf_page__` Function
    ///    + Contains edge per `.page`, and pushes
    ///      `UnresolvedReference::FrameworkBinding` rows for resolved
    ///      `{!method}` bindings so downstream semantic resolution emits
    ///      `EdgeKind::Framework(VisualforcePage)` by construction.
    /// 2. [`framework_entry_point_stage::run`] — Round 5 R11 fix.
    ///    Walks every user-declared class's inheritance chain and
    ///    tags ancestor contract methods with the platform-interface
    ///    entry-point tag whenever any descendant declares the
    ///    interface. Without this pass the abstract-base pattern
    ///    (CRLP_Batch_Base_Skew et al.) produces `no_callers` false
    ///    positives on every contract method.
    ///
    /// Both stages short-circuit on empty `class_symbols` so running
    /// this hook on a non-Apex parse (which has no class_symbols) is
    /// a cheap no-op. The stage order is deliberately unchanged from
    /// the pre-T5 orchestrator: VF extraction runs first so the
    /// framework-propagation pass sees the VF synthetic body nodes.
    ///
    /// Fail-open semantics mirror pre-T5: stage 1 failing does not
    /// prevent stage 2 from running. Stage 2 failing is reported as
    /// [`HookOutcome::Warning`] while preserving any success summary
    /// from stage 1 — no observable parse-level failure reporting
    /// drift across the rework.
    fn post_syntax_hooks(
        &self,
        workspace_root: &std::path::Path,
        syntax_results: &mut crate::application::ports::SyntaxResults,
    ) -> crate::syntax::language::extractor::HookOutcome {
        use crate::syntax::language::apex::{framework_entry_point_stage, vf_extraction_stage};
        use crate::syntax::language::extractor::HookOutcome;

        let mut summaries: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        match vf_extraction_stage::run(workspace_root, syntax_results) {
            Ok(stats) => {
                if stats.pages_parsed > 0 || stats.pages_failed > 0 {
                    summaries.push(format!(
                        "VF extraction: {} pages parsed, {} failed, {} synthetic nodes, {} bindings resolved, {} unresolved",
                        stats.pages_parsed,
                        stats.pages_failed,
                        stats.synthetic_nodes_emitted,
                        stats.bindings_resolved,
                        stats.bindings_unresolved,
                    ));
                }
            }
            Err(e) => {
                warnings.push(format!(
                    "VF extraction stage failed ({e}); continuing without Visualforce bindings"
                ));
            }
        }

        match framework_entry_point_stage::run(syntax_results) {
            Ok(stats) => {
                if stats.function_nodes_tagged > 0 || stats.function_nodes_already_tagged > 0 {
                    summaries.push(format!(
                        "framework entry-point propagation: {} classes with platform interface, {} contract-method targets, {} nodes newly tagged, {} already tagged",
                        stats.classes_with_platform_interface,
                        stats.contract_method_targets,
                        stats.function_nodes_tagged,
                        stats.function_nodes_already_tagged,
                    ));
                }
            }
            Err(e) => {
                warnings.push(format!(
                    "framework entry-point propagation stage failed ({e}); continuing without ancestor-chain interface tagging"
                ));
            }
        }

        if !warnings.is_empty() {
            // Surface the first warning so the `warn!` log line stays
            // recognisable. Extra warnings append via newline so a
            // downstream consumer can split and inspect.
            let joined = warnings.join("; ");
            return HookOutcome::Warning { message: joined };
        }
        if summaries.is_empty() {
            HookOutcome::NoOp
        } else {
            HookOutcome::Ok {
                summary: Some(summaries.join(" | ")),
            }
        }
    }
}

/// Recover the Apex top-level declaration name from the file path.
///
/// Apex's on-disk convention: `.../Foo.cls` declares top-level class
/// `Foo`. Case-sensitive at the API level; persistence normalises
/// via `COLLATE NOCASE` on the `api_name` column (see
/// `infrastructure/storage/schema.rs`).
fn file_stem(file_path: &str) -> String {
    std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod post_syntax_hooks_tests {
    //! T5 behavioural proof for the Apex override: when `class_symbols`
    //! is empty (i.e. no `.cls` files were parsed), the hook runs both
    //! stages but both short-circuit internally and the outcome is
    //! `NoOp`. This locks the "non-Apex parse invariant" — running
    //! the Apex `LanguageSpecificExtractor` on a non-Apex syntax
    //! snapshot must not emit VF / propagation log lines.

    use super::ApexExtractor;
    use crate::application::ports::SyntaxResults;
    use crate::syntax::language::extractor::{HookOutcome, LanguageSpecificExtractor};
    use std::path::Path;

    #[test]
    fn apex_post_syntax_hooks_noop_on_empty_class_symbols() {
        let extractor = ApexExtractor;
        let mut results = SyntaxResults::new();
        let outcome = extractor.post_syntax_hooks(Path::new("/tmp/fake-root"), &mut results);
        assert_eq!(
            outcome,
            HookOutcome::NoOp,
            "Apex hook must no-op when class_symbols is empty (non-Apex parse invariant)"
        );
    }
}
