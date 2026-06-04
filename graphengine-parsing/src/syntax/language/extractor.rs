//! Per-language extractor trait.
//!
//! Every language-specific bit of logic that we currently dispatch on via
//! `match config.language.as_str()` lives behind this one trait. The goal is
//! that adding a 10th language means adding one new impl in
//! `syntax/language/extractors/` and one arm in `loader::load_extractor` —
//! never again a new arm inside a shared extractor.
//!
//! There are four callers in the wider crate that previously did
//! match-on-language:
//!
//! - `syntax/extractors/complexity_extractor.rs` — AST node classification for
//!   cyclomatic / cognitive complexity (`is_function_definition`,
//!   `is_cyclomatic_decision_point`, `is_cognitive_structural`, `is_flow_break`,
//!   `is_logical_operator_node`, `is_continuation_if`).
//! - `syntax/extractors/symbol_extractor.rs` — "is this extracted symbol a
//!   test?" (`is_test_symbol`).
//! - `syntax/extractors/trait_context_detector.rs` — identical test-symbol
//!   predicate applied to functions (`is_test_symbol`).
//! - `infrastructure/lsp/receiver_detector.rs` — default trait-object /
//!   interface type patterns when YAML config doesn't provide them
//!   (`is_trait_object_type`).
//!
//! The trait surface is intentionally tight: only behaviour that is genuinely
//! language-dependent lives here. Everything else (FQN building, visibility
//! detection, query execution) stays shared.

use tree_sitter::{Node, Tree};

use crate::domain::apex::class_symbols::ApexTypeRef;
use crate::domain::{Edge, Node as DomainNode};

/// Result of a language's external-reference synthesis pass. See
/// [`LanguageSpecificExtractor::synthesize_external_references`].
#[derive(Debug, Default, Clone)]
pub struct ExternalReferenceResult {
    /// Virtual nodes representing dependencies outside the analysed
    /// source (e.g. Apex managed-package namespaces). Keyed into the
    /// graph by their stable id so repeated discoveries across files
    /// dedup to a single node.
    pub nodes: Vec<DomainNode>,
    /// Edges from the in-repo consumer to each external node.
    pub edges: Vec<Edge>,
}

/// Outcome of a single language's [`LanguageSpecificExtractor::post_syntax_hooks`] pass.
///
/// Intentionally not a [`Result`] because the orchestrator always continues:
/// a failed hook degrades the parse (e.g. Visualforce bindings go missing
/// when the VF stage errors) but never aborts it. That mirrors the pre-T5
/// `match … { Ok(_) => info!, Err(e) => warn!, … }` behaviour the
/// orchestrator used before this seam existed.
///
/// The variants carry the round-trip shape of the pre-T5 log lines so
/// `cargo test` fixtures comparing log output stay stable across the
/// refactor.
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum HookOutcome {
    /// Hook did not participate (default for languages that do not
    /// override `post_syntax_hooks`). The orchestrator emits no log
    /// line for this outcome.
    #[default]
    NoOp,
    /// Hook ran. The optional `summary` is a human-readable string the
    /// orchestrator writes at `info!` level; `None` means the hook ran
    /// but produced no output worth logging (e.g. Apex VF stage on a
    /// repo with zero `.page` files).
    Ok { summary: Option<String> },
    /// Hook failed. The caller logs `message` at `warn!` level and
    /// continues with the remaining pipeline stages. Behavioural parity
    /// with the pre-T5 orchestrator, which `warn!`ed and continued on
    /// every hook error.
    Warning { message: String },
}

/// Per-language syntax analysis hooks.
///
/// Implementations are stateless (ZSTs or pure constants) and therefore cheap
/// to pass around behind `Arc<dyn>` / `&dyn`. Sharable across parser threads.
pub trait LanguageSpecificExtractor: Send + Sync {
    /// The canonical language identifier (matches `LanguageConfig.language`).
    fn language(&self) -> &str;

    // -----------------------------------------------------------------------
    // Complexity: AST kind classification (by string only)
    // -----------------------------------------------------------------------

    /// True if `kind` names a function/method/closure definition in this
    /// language. Used by complexity extraction to delimit function bodies.
    fn is_function_definition(&self, kind: &str) -> bool;

    /// True if `kind` is a McCabe decision point (+1 cyclomatic).
    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool;

    /// True if `kind` is a Sonar-style structural construct
    /// (+1 + nesting cognitive, opens a new nesting context).
    fn is_cognitive_structural(&self, kind: &str) -> bool;

    /// True if `kind` is a flow break that adds +1 cognitive
    /// (`break`, `continue`, `throw`, `raise`, `goto`, …).
    fn is_flow_break(&self, kind: &str) -> bool;

    // -----------------------------------------------------------------------
    // Complexity: node-level classification (needs AST context)
    // -----------------------------------------------------------------------

    /// True if this AST node represents a short-circuit logical operator
    /// (`&&`, `||`, `and`, `or`) — each one adds +1 to both cyclomatic and
    /// cognitive complexity.
    fn is_logical_operator_node(&self, node: &Node, source: &[u8]) -> bool;

    /// True if this AST node represents an `else if` / `elif` *continuation*
    /// of an outer `if`. Continuations get +1 cognitive with **no** nesting
    /// penalty, so the walker needs to distinguish them from nested ifs.
    fn is_continuation_if(&self, node: &Node) -> bool;

    // -----------------------------------------------------------------------
    // Symbol-level classification
    // -----------------------------------------------------------------------

    /// Does this non-function AST node represent a test symbol (test class,
    /// test module, Apex test class, …)? Called from `symbol_extractor` when
    /// setting `is_test = true` on modules/structs/enums/etc.
    ///
    /// This is intentionally separate from [`is_test_function`] because some
    /// languages (Rust) distinguish: a module with `#[test]` attribute is a
    /// test module, but a function *inside* a `#[cfg(test)]` module is also a
    /// test even without its own attribute.
    ///
    /// Default: `false`.
    fn is_test_symbol(&self, _node: &Node, _source: &[u8]) -> bool {
        false
    }

    /// Does this AST node represent a test function? Called from
    /// `trait_context_detector` when setting `is_test = true` on extracted
    /// functions.
    ///
    /// Default: falls back to [`is_test_symbol`]. Languages override when
    /// function-level test detection is stricter or looser than the generic
    /// symbol-level check (Rust does both: `#[test]` attribute *or* enclosing
    /// `#[cfg(test)]` module).
    fn is_test_function(&self, node: &Node, source: &[u8]) -> bool {
        self.is_test_symbol(node, source)
    }

    // -----------------------------------------------------------------------
    // Receiver-type classification (for call edge trait-dispatch detection)
    // -----------------------------------------------------------------------

    /// Default trait-object / interface pattern check used by
    /// `ReceiverTypeDetector` when the YAML config has no
    /// `receiver_type_detection.trait_object_patterns`.
    ///
    /// The YAML path is preferred because it is language-configurable without
    /// a code change; this method is the safety net for older configs.
    fn is_trait_object_type(&self, type_string: &str) -> bool {
        // Conservative default: anything that even *mentions* dynamic dispatch
        // keywords. Languages can narrow this in their impl.
        type_string.contains("dyn")
            || type_string.contains("interface")
            || type_string.contains("trait")
    }

    // -----------------------------------------------------------------------
    // Language-specific graph-node annotation
    // -----------------------------------------------------------------------

    /// Return entry-point tags for this symbol (empty for non-entry points).
    ///
    /// An "entry point" is a symbol reachable from outside the analysed
    /// codebase — a REST handler, a callable for Flow / LWC, an async job
    /// entry, a SOAP endpoint, etc. Shared extractors set an `entry_points`
    /// property on graph nodes so analysis can treat them as sinks rather
    /// than uncalled dead code.
    ///
    /// Default: `vec![]`. Currently only Apex implements this.
    ///
    /// Tag values are language-defined stable strings; see
    /// `syntax::language::apex::EntryPointKind::as_str`.
    ///
    /// `annotation_query` is the YAML-defined `annotations` query
    /// string fetched by the caller from
    /// `LanguageConfig::get_query("annotations")`. Passing it in —
    /// rather than having the impl re-read YAML — keeps the grammar
    /// shape described in one place (`configs/<lang>.yaml`) and lets
    /// language impls cross-check their own AST walk against the
    /// YAML-declared grammar. `None` is acceptable: impls SHOULD
    /// degrade to a direct AST walk so a missing binding never
    /// silently drops entry-point tagging.
    fn entry_point_tags(
        &self,
        _node: &Node,
        _source: &[u8],
        _annotation_query: Option<&str>,
    ) -> Vec<&'static str> {
        Vec::new()
    }

    /// Return language-specific properties to attach to a struct/class node.
    ///
    /// Returned `(key, value)` pairs are merged into the graph node's
    /// `properties` map by `symbol_extractor`. Property keys are
    /// language-specific and form part of the cross-client contract — once
    /// published they should be treated as stable.
    ///
    /// Currently used by Apex to surface the class-level `with sharing` /
    /// `without sharing` / `inherited sharing` modifier (or `omitted` when a
    /// top-level class declares no sharing modifier — a Salesforce security
    /// concern). Other languages return an empty vector by default.
    fn extract_struct_metadata(
        &self,
        _node: &Node,
        _source: &[u8],
    ) -> Vec<(&'static str, serde_json::Value)> {
        Vec::new()
    }

    /// Build a language-specific fully-qualified name override for a
    /// single extracted symbol, or return `None` to use the shared
    /// path-based FQN builder.
    ///
    /// Introduced in Sprint E.2 to give Apex (and any future Java-like
    /// language) a place to encode enclosing inner-class paths and
    /// method parameter signatures. Returning `None` is the stable
    /// contract — every existing language keeps its current FQN shape
    /// by default.
    ///
    /// - `node`: the AST node for the declaration (function,
    ///   class, interface, enum, trigger, module, …).
    /// - `simple_name`: the declared simple name as captured by the
    ///   tree-sitter query.
    /// - `file_path`: path used by the shared builder for the path
    ///   prefix. Implementations that override SHOULD reuse this prefix
    ///   via `build_simple_fqn` so the FQN stays workspace-consistent.
    /// - `workspace_root`: workspace root passed through from
    ///   [`build_simple_fqn`].
    fn build_symbol_fqn(
        &self,
        _node: &Node,
        _source: &[u8],
        _simple_name: &str,
        _file_path: &str,
        _workspace_root: Option<&str>,
    ) -> Option<String> {
        None
    }

    /// Extract extra language-specific properties attached to a
    /// trigger-kind struct node (Apex).
    ///
    /// Separated from [`extract_struct_metadata`] because trigger
    /// metadata needs the `trigger_events` query string pulled from
    /// the YAML config, which the generic path does not have.
    /// `events_query` is fetched by the caller from
    /// `LanguageConfig::get_query("trigger_events")` — passing it in
    /// preserves YAML as the single source of truth for grammar
    /// bindings (no query strings live in Rust constants).
    ///
    /// Default: `Vec::new()`. Only Apex implements this today.
    fn extract_trigger_metadata(
        &self,
        _trigger_node: &Node,
        _source: &[u8],
        _events_query: &str,
        _language: tree_sitter::Language,
    ) -> Vec<(&'static str, serde_json::Value)> {
        Vec::new()
    }

    /// Synthesize language-specific **sibling** symbols that belong
    /// logically to a just-extracted parent symbol.
    ///
    /// Introduced in Sprint E.3 so that Apex triggers — which have no
    /// enclosing method in source — can still emit a real `Function`
    /// node (`__trigger__`) that owns the trigger-body range. Without
    /// that function node, call sites at the top level of a trigger
    /// have no caller to attach `Call` edges to and fall out of the
    /// graph entirely.
    ///
    /// Each returned `DomainNode` is inserted alongside the parent and
    /// wired to the parent via a `Contains` edge by the shared
    /// `symbol_extractor` — implementations never build the edge
    /// themselves, so containment stays uniform across languages.
    ///
    /// - `node`: the AST node of the parent declaration
    ///   (`trigger_declaration`, `class_declaration`, …).
    /// - `parent`: the already-constructed domain node for that parent.
    ///   Its `id` is the Contains edge's `from_id`.
    ///
    /// Default: no siblings. Currently only Apex implements this.
    fn synthesize_symbol_siblings(
        &self,
        _node: &Node,
        _source: &[u8],
        _parent: &DomainNode,
        _file_path: &str,
        _workspace_root: Option<&str>,
    ) -> Vec<DomainNode> {
        Vec::new()
    }

    /// Synthesize language-specific **external** references for one file.
    ///
    /// This hook exists because some languages — principally Apex —
    /// carry first-class references to code the analyser cannot see
    /// (Salesforce managed packages installed into the org at runtime).
    /// Those references are deterministic from source text (they live
    /// in recognisable `npsp.Foo` / `npe01__Bar__c` tokens); there is
    /// nothing for a semantic resolver to resolve. The extractor
    /// synthesises a virtual `Module` node per external namespace and
    /// an `Import` edge from the enclosing file-module node (passed as
    /// `consumer_node_id`) to each distinct namespace.
    ///
    /// Implementations MUST deduplicate edges per-file so a single class
    /// referencing `npsp` ten times produces exactly one Import edge.
    /// Node ids are stable across the whole scan (SHA-based on the
    /// virtual FQN), so returning the same node from two different
    /// files deduplicates automatically at graph insertion.
    ///
    /// Default: no external references. Only Apex implements this.
    fn synthesize_external_references(
        &self,
        _tree: &Tree,
        _source: &[u8],
        _file_path: &str,
        _consumer_node_id: &str,
    ) -> ExternalReferenceResult {
        ExternalReferenceResult::default()
    }

    /// Extract per-class structural symbol tables from this file's
    /// AST — fields, methods, constructors, inner classes, inheritance
    /// — and return them as `(dotted_api_name, symbols_json_payload)`
    /// tuples. The payload is serialised by the implementing language
    /// (Apex uses `serde_json::to_string(&ApexClassSymbols)`) so the
    /// generic syntax pipeline can persist it to the language's
    /// symbol-table DB table without depending on any language's
    /// domain types.
    ///
    /// Apex ships this in TR-A.0 as the backing store for
    /// constructor / field-type / overload / inner-class dispatch
    /// (PRs 2–5). All other languages default to "no symbols" —
    /// this is the `no-op-for-byte-identical` extension point the
    /// rev-6.1 regression gate relies on (a language that doesn't
    /// produce symbols cannot accidentally emit graph-affecting
    /// edges from them).
    ///
    /// Returns an empty `Vec` by default. Implementations that return
    /// a non-empty vec MUST use stable JSON serialisation so the DB
    /// payload is byte-identical between parse runs of identical
    /// source.
    fn extract_class_symbols(
        &self,
        _tree: &Tree,
        _source: &[u8],
        _file_path: &str,
    ) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Extract per-method local-variable scopes from this file's AST.
    /// Each returned [`crate::application::ports::LocalVarScope`]
    /// carries the body-range of a method / constructor and the list
    /// of locals (plus formal parameters) declared inside that body,
    /// each with its declared [`ApexTypeRef`].
    ///
    /// Apex ships this in TR-A.3 so the field-type-aware dispatch
    /// resolver can bind `someLocal.method(...)` to the method owned
    /// by the local's declared type without re-parsing the file.
    /// Other languages default to "no scopes" — their YAML-based
    /// extractors do not currently consume local-var data and the
    /// empty-vec default keeps their graph shape byte-identical.
    ///
    /// This result is **ephemeral**: it flows on
    /// [`crate::application::ports::SyntaxResults::local_var_scopes`]
    /// and is consumed during the same parse run's semantic
    /// resolution. It is never persisted to the parse DB.
    fn extract_local_var_scopes(
        &self,
        _tree: &Tree,
        _source: &[u8],
        _file_path: &str,
    ) -> Vec<crate::application::ports::LocalVarScope> {
        Vec::new()
    }

    /// Infer the [`ApexTypeRef`] of each positional argument at a
    /// call site. Called by
    /// [`crate::syntax::extractors::call_site_extractor::CallSiteExtractor`]
    /// when a YAML query captures `@args` on an `argument_list` node
    /// and the surrounding call shape is one this language wants to
    /// resolve by signature (e.g. Apex constructor / method dispatch).
    ///
    /// The type is `ApexTypeRef` today because Apex is the only
    /// language that owns a signature-matching resolver arm in
    /// Phase A (TR-A.1 closes over this for constructor overload
    /// disambiguation; TR-A.4 reuses it for method overloads).
    /// Generalising to a language-agnostic argument type enum is a
    /// Phase-B concern once a second language grows overload
    /// dispatch (Java / C# would be the natural next candidates).
    ///
    /// Non-Apex languages return the default empty vec and pay zero
    /// cost — the caller writes the empty vec straight through to
    /// `CallSite.arg_types`, which is byte-identical with the
    /// pre-TR-A.1 shape for every non-Apex language.
    fn infer_call_site_arg_types(&self, _args_node: &Node<'_>, _source: &[u8]) -> Vec<ApexTypeRef> {
        Vec::new()
    }

    /// Extract a per-file extraction-coverage record for this parsed
    /// file. Introduced in T8 (universal-fidelity sprint) — see
    /// `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`.
    ///
    /// The record separates "AST nodes present" from "AST nodes the
    /// extractor's query set walked", and names specific gap shapes
    /// (R39 property accessors, R41 map-literal initializers, …)
    /// that the health-report classifier consults to downgrade
    /// `dead_code.no_callers` confidence.
    ///
    /// Default impl returns `None` — languages without an audited
    /// coverage pass contribute zero records, which the classifier
    /// interprets as "no known invalidating gap, behave as pre-T8".
    /// Apex implements this for R39 / R41 in
    /// [`crate::syntax::language::apex::coverage`].
    ///
    /// Called once per parsed file after the extractors have run.
    /// Implementations should not re-parse — they receive the same
    /// `Tree` already built for symbol / call-site extraction.
    fn extract_file_coverage(
        &self,
        _tree: &Tree,
        _source: &[u8],
        _file_path: &str,
    ) -> Option<crate::application::ports::FileExtractionCoverage> {
        None
    }

    /// Run language-specific post-syntax hooks after tree-sitter extraction
    /// and before semantic resolution.
    ///
    /// This seam exists so the parse-repo orchestrator can dispatch
    /// per-language stages (Apex Visualforce extraction, Apex framework
    /// entry-point propagation, future LWC / Java Spring / Rust
    /// proc-macro count hooks) without re-growing a hardcoded call list
    /// per language. See the T5 design document at
    /// `docs/workstreams/universal-fidelity/tasks/T5-orchestrator-collapse.md`.
    ///
    /// # Contract
    ///
    /// - Runs **after** [`extract`](crate::application::ports::SyntaxExtractor::extract)
    ///   has populated `symbols`, `references`, `imports`, `type_refs`,
    ///   and `class_symbols`.
    /// - Runs **before** semantic resolution (LSP + heuristic call
    ///   resolution).
    /// - May mutate `syntax_results` in place.
    /// - MUST be no-op-safe when called on a parse that did not
    ///   populate the structures the hook consumes (e.g. Apex's VF
    ///   stage short-circuits on empty `class_symbols`).
    /// - Errors are reported as [`HookOutcome::Warning`]; the caller
    ///   logs + continues. Panics are a programmer error.
    ///
    /// The default implementation returns [`HookOutcome::NoOp`]. Every
    /// language that does not own a post-syntax hook inherits this.
    fn post_syntax_hooks(
        &self,
        _workspace_root: &std::path::Path,
        _syntax_results: &mut crate::application::ports::SyntaxResults,
    ) -> HookOutcome {
        HookOutcome::NoOp
    }
}

/// No-op fallback extractor used for unknown / unsupported languages.
///
/// All predicates return `false` — the caller treats the language as having
/// no decision points, no tests, no trait dispatch. This is the same
/// behaviour the old `_ => false` match arm gave us.
pub struct GenericExtractor {
    language: String,
}

impl GenericExtractor {
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            language: language.into(),
        }
    }
}

impl LanguageSpecificExtractor for GenericExtractor {
    fn language(&self) -> &str {
        &self.language
    }

    fn is_function_definition(&self, _kind: &str) -> bool {
        false
    }

    fn is_cyclomatic_decision_point(&self, _kind: &str) -> bool {
        false
    }

    fn is_cognitive_structural(&self, _kind: &str) -> bool {
        false
    }

    fn is_flow_break(&self, _kind: &str) -> bool {
        false
    }

    fn is_logical_operator_node(&self, _node: &Node, _source: &[u8]) -> bool {
        false
    }

    fn is_continuation_if(&self, _node: &Node) -> bool {
        false
    }
}

/// Shared helper: extract the operator token text from a `binary_expression`
/// AST node, used by the `{ts, js, rust, java, csharp, go, apex}` impls to
/// distinguish `&&` / `||` from arithmetic operators.
pub(crate) fn binary_operator_text<'a>(node: &Node, source: &'a [u8]) -> Option<&'a str> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if !child.is_named() {
                let text = child.utf8_text(source).ok()?;
                if text == "&&" || text == "||" {
                    return Some(text);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod post_syntax_hooks_default {
    //! T5 compile-time + behavioural proof: every non-overriding
    //! `LanguageSpecificExtractor` inherits the `HookOutcome::NoOp`
    //! default. Parameterised over the stock language extractor set.
    //! A language that later grows a hook override but forgets to
    //! extend this test will still build — but if it accidentally
    //! widens the default (e.g. someone changes the default to
    //! `Ok { summary: None }`), every test below breaks.

    use super::{GenericExtractor, HookOutcome, LanguageSpecificExtractor};
    use crate::application::ports::SyntaxResults;
    use std::path::Path;

    fn assert_noop<E: LanguageSpecificExtractor>(extractor: E) {
        let mut results = SyntaxResults::new();
        let outcome = extractor.post_syntax_hooks(Path::new("/tmp/fake-root"), &mut results);
        assert_eq!(outcome, HookOutcome::NoOp, "expected NoOp default");
    }

    #[test]
    fn generic_extractor_default_is_noop() {
        assert_noop(GenericExtractor::new("fake"));
    }

    #[test]
    fn hook_outcome_default_is_noop() {
        assert_eq!(HookOutcome::default(), HookOutcome::NoOp);
    }
}
