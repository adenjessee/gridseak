//! Dead-code reason classifier: framework-keyed dispatch.
//!
//! Takes a set of node IDs judged "dead" by [`super::dead_code`]
//! and assigns each one a [`DeadCodeReason`] plus an evidence
//! string. The aggregate distribution is stamped onto
//! [`super::report::DeadCodeMetricDetail`]; per-function reason +
//! evidence land on [`super::report::NodeAnnotation`].
//!
//! # Architecture (Wave 2: framework-keyed)
//!
//! Dispatch walks three layers in order:
//!
//! 1. **Universal pre-rules** ([`universal::register_default`]) —
//!    language- and framework-agnostic signals that are valid on
//!    every node: `entry_point_tags`, `is_attribute_invoked`,
//!    `is_callback_target`, `parent_is_test`. First match wins.
//! 2. **Framework-keyed rule sets** ([`frameworks::register_default`])
//!    — dispatched by the framework tags on the node itself (or on
//!    its parent file, via the propagation in
//!    `AnalysisGraph::propagate_file_metadata_to_descendants`). Each
//!    framework owns a single module under
//!    [`frameworks`]. Polyglot repos pass through every rule set
//!    applicable to each node rather than collapsing to a single
//!    repo-level ecosystem.
//! 3. **Generic terminal fallback** ([`generic::GenericClassifier`])
//!    — visibility-based verdict for any node that no framework
//!    rule claimed. Never returns `None`.
//!
//! Every dead node receives a non-`None` verdict. The fallthrough
//! to [`DeadCodeReason::Unclassified`] is total — there is no path
//! that produces a live node with a dead-code reason.
//!
//! # Deletion of the ecosystem-keyed registry
//!
//! The pre-Wave-2 registry dispatched by the repo-level
//! `Ecosystem` enum. That collapsed NPSP (Apex + LWC JS + Python
//! tooling) to a single "Apex" bucket, misclassifying the LWC and
//! Python files. The new registry keeps `ClassifyContext.ecosystem`
//! purely advisory — rule sets may read it for evidence text but
//! MUST NOT use it for dispatch. See
//! `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` R25.

use std::collections::BTreeMap;

use super::config::Ecosystem;
use super::graph::AnalysisGraph;
use super::report::{Confidence, DeadCodeReason};

pub mod frameworks;
pub mod generic;
pub mod universal;

/// Per-node input bundle for the classifier.
pub struct ClassifyContext<'a> {
    pub node_id: &'a str,
    pub graph: &'a AnalysisGraph,
    /// Repo-level ecosystem hint. Advisory only: framework-keyed
    /// rule sets dispatch on `GraphNode.frameworks`, not this
    /// field. Retained for evidence strings and for the legacy
    /// `GenericClassifier` whose trait shape still requires it.
    pub ecosystem: Ecosystem,
    pub fan_in: usize,
}

/// Framework-keyed rule set trait. Every concrete rule module
/// under [`frameworks`] implements this. Universal pre-rules
/// (in [`universal`]) implement it too using the sentinel
/// framework `"*"`.
pub trait FrameworkRuleSet: Send + Sync {
    /// Framework name this rule set owns, or `"*"` for universal
    /// rules that run on every node. Must be stable across builds
    /// — downstream tooling stamps it into evidence.
    fn framework(&self) -> &'static str;
    /// Short stable identifier (e.g. `"apex-tdtm"`,
    /// `"python-django"`). Stamped into
    /// [`super::report::NodeAnnotation::dead_code_classifier`].
    fn name(&self) -> &'static str;
    /// Return `Some((reason, evidence))` when this rule set
    /// recognises the node, `None` to fall through.
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)>;
}

/// Terminal fallback trait (visibility-based). One implementor
/// only: [`generic::GenericClassifier`]. Kept separate from
/// [`FrameworkRuleSet`] because the generic fallback is total —
/// it never returns `None` in the dead-code pipeline, while a
/// framework rule set typically does.
pub trait TerminalClassifier: Send + Sync {
    fn name(&self) -> &'static str;
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)>;
}

impl<T: TerminalClassifier + ?Sized> TerminalClassifier for Box<T> {
    fn name(&self) -> &'static str {
        (**self).name()
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        (**self).classify(ctx)
    }
}

/// Single verdict — one per dead function.
#[derive(Debug, Clone)]
pub struct DeadCodeVerdict {
    pub node_id: String,
    pub reason: DeadCodeReason,
    pub evidence: String,
    pub classifier: &'static str,
    /// How much weight a consumer should give this verdict. Every
    /// verdict is produced at [`Confidence::High`] by the
    /// framework-keyed registry (the classifier itself has no
    /// native uncertainty — it either matched or it didn't).
    /// Post-classification passes downgrade the confidence when
    /// out-of-band evidence contradicts the "this function is
    /// dead" conclusion. Current downgraders:
    ///
    /// 1. `apply_git_signal_churn_downgrade` (T7): downgrades to
    ///    `Medium` on files touched within
    ///    `graphengine_git_signals::predicates::ACTIVE_RECENT_MAX_DAYS`
    ///    when the repository shape permits `Confidence::High`
    ///    signals. "Actively edited" is very weak evidence for
    ///    liveness — but it is still evidence.
    ///
    /// Downgraders are composable. A verdict can drop from High →
    /// Medium → Low as successive passes find further contradicting
    /// evidence; see `DeadCodeVerdict::apply_downgrade_to`.
    pub confidence: Confidence,
}

impl DeadCodeVerdict {
    /// Clamp this verdict's confidence to at most `target`.
    /// Downgraders call this so a later weaker reason cannot
    /// accidentally promote confidence back up (e.g. a second
    /// downgrader running after one that set Low must never leave
    /// the verdict at Medium).
    pub fn apply_downgrade_to(&mut self, target: Confidence) {
        self.confidence = min_confidence(self.confidence, target);
    }
}

/// Rank Confidence variants so downgrade logic can pick the
/// stricter of two. `High > Medium > Low`. Used for the clamp in
/// [`DeadCodeVerdict::apply_downgrade_to`].
fn min_confidence(a: Confidence, b: Confidence) -> Confidence {
    fn rank(c: Confidence) -> u8 {
        match c {
            Confidence::High => 3,
            Confidence::Medium => 2,
            Confidence::Low => 1,
        }
    }
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
}

/// Framework-keyed dispatcher. Owns a universal pre-rule chain,
/// a map of framework-name → rule set, and a terminal fallback.
pub struct FrameworkRuleRegistry {
    universal: Vec<Box<dyn FrameworkRuleSet>>,
    by_framework: BTreeMap<&'static str, Box<dyn FrameworkRuleSet>>,
    terminal: Box<dyn TerminalClassifier>,
}

impl Default for FrameworkRuleRegistry {
    /// Day-1 registration: universal pre-rules + every
    /// framework module in [`frameworks::register_default`] +
    /// the generic terminal fallback.
    fn default() -> Self {
        let mut by_framework: BTreeMap<&'static str, Box<dyn FrameworkRuleSet>> = BTreeMap::new();
        for rule in frameworks::register_default() {
            let key = rule.framework();
            assert!(
                key != "*",
                "framework `*` is reserved for universal rules; use universal::register_default"
            );
            assert!(
                !by_framework.contains_key(key),
                "duplicate framework registration: {}",
                key
            );
            by_framework.insert(key, rule);
        }
        Self {
            universal: universal::register_default(),
            by_framework,
            terminal: Box::new(generic::GenericClassifier::new()),
        }
    }
}

impl FrameworkRuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Produce a verdict for one dead node. Never returns `None`.
    pub fn classify(&self, ctx: &ClassifyContext<'_>) -> DeadCodeVerdict {
        // --- Layer 1: universal pre-rules ---
        for rule in &self.universal {
            if let Some((reason, evidence)) = rule.classify(ctx) {
                return DeadCodeVerdict {
                    node_id: ctx.node_id.to_string(),
                    reason,
                    evidence,
                    classifier: rule.name(),
                    confidence: Confidence::High,
                };
            }
        }

        // --- Layer 2: framework-keyed rules ---
        // Walk the node's frameworks in order. `frameworks` on the
        // GraphNode is already sorted (BTreeSet → Vec in the
        // parser) so dispatch is deterministic.
        if let Some(node) = ctx.graph.nodes.get(ctx.node_id) {
            for fw in &node.frameworks {
                if let Some(rule) = self.by_framework.get(fw.as_str()) {
                    if let Some((reason, evidence)) = rule.classify(ctx) {
                        return DeadCodeVerdict {
                            node_id: ctx.node_id.to_string(),
                            reason,
                            evidence,
                            classifier: rule.name(),
                            confidence: Confidence::High,
                        };
                    }
                }
            }
        }

        // --- Layer 3: generic terminal fallback ---
        if let Some((reason, evidence)) = self.terminal.classify(ctx) {
            return DeadCodeVerdict {
                node_id: ctx.node_id.to_string(),
                reason,
                evidence,
                classifier: self.terminal.name(),
                confidence: Confidence::High,
            };
        }

        // Total-coverage guarantee: terminal fallback SHOULD
        // always return Some (by construction). Unclassified is
        // reserved for the violated-contract case.
        DeadCodeVerdict {
            node_id: ctx.node_id.to_string(),
            reason: DeadCodeReason::Unclassified,
            evidence: "no rule returned a verdict (terminal fallback contract violated)"
                .to_string(),
            classifier: "registry",
            confidence: Confidence::High,
        }
    }

    /// Classify a batch of dead nodes in input order.
    pub fn classify_batch(
        &self,
        dead_node_ids: &[String],
        graph: &AnalysisGraph,
        ecosystem: Ecosystem,
    ) -> Vec<DeadCodeVerdict> {
        dead_node_ids
            .iter()
            .map(|id| {
                let fan_in = graph.fan_in(id);
                let ctx = ClassifyContext {
                    node_id: id,
                    graph,
                    ecosystem,
                    fan_in,
                };
                self.classify(&ctx)
            })
            .collect()
    }
}

/// Backwards-compatible alias. External callers (and the rest of
/// the analysis crate) still reference the old type name.
pub type ClassifierRegistry = FrameworkRuleRegistry;

/// Aggregate a list of verdicts into a `reason_breakdown` map.
/// Every reason key is present (including zeroes).
pub fn build_reason_breakdown(verdicts: &[DeadCodeVerdict]) -> BTreeMap<String, usize> {
    let mut out = empty_reason_breakdown();
    for v in verdicts {
        *out.entry(v.reason.as_str().to_string()).or_insert(0) += 1;
    }
    out
}

/// T7 Layer 0 post-pass: downgrade the confidence of every dead-
/// code verdict whose underlying file shows recent churn in the
/// provided [`graphengine_git_signals::GitSignalReport`]. "Recent"
/// is defined by
/// [`graphengine_git_signals::predicates::ACTIVE_RECENT_MAX_DAYS`]
/// (30 days at T7 authoring time) and the gate is `High` git-
/// signal confidence — shallow-clone repositories produce
/// `Confidence::Low` signals and therefore never trigger a
/// downgrade here. This preserves the measured-fallback
/// discipline: on repos where git evidence is untrustworthy we do
/// not *promote* dead-code verdicts (they stay `High`), we just
/// fail to contradict them.
///
/// The motivating scenario: a file with `no_callers` on the
/// static graph but five commits in the last two weeks is almost
/// certainly not dead — it might be newly added code whose
/// callers land in a later commit, or scaffolding for an
/// in-flight feature. `Medium`-confidence on such a verdict is
/// honest; leaving it at `High` would be overconfident.
///
/// When `git_signals` is `None`, this function is a no-op. That
/// is the T7 "signal absent, not signal negative" contract: no
/// churn measurement means no churn-based downgrade, not "no
/// churn".
pub fn apply_git_signal_churn_downgrade(
    verdicts: &mut [DeadCodeVerdict],
    graph: &AnalysisGraph,
    git_signals: Option<&graphengine_git_signals::GitSignalReport>,
) -> usize {
    let Some(report) = git_signals else {
        return 0;
    };

    let mut downgraded: usize = 0;
    for verdict in verdicts.iter_mut() {
        let Some(node) = graph.nodes.get(&verdict.node_id) else {
            continue;
        };
        // Resolve the file path the node sits in. Repo-relative
        // path is the stable key we can look up in
        // `GitSignalReport.per_file`; `file_path` is the absolute
        // form and only matches if the git-signal extractor was
        // run from the same root. Prefer `path_repo_rel` for
        // exactly this reason.
        let key = match node.path_repo_rel.as_deref().or(node.file_path.as_deref()) {
            Some(path) => std::path::PathBuf::from(path),
            None => continue,
        };
        let Some(signals) = report.per_file.get(&key) else {
            continue;
        };
        if !matches!(
            signals.confidence,
            graphengine_git_signals::Confidence::High
        ) {
            // Shallow-clone guard: never downgrade based on
            // unreliable evidence.
            continue;
        }
        let within_window = signals
            .last_touched_days
            .map(|d| d < graphengine_git_signals::predicates::ACTIVE_RECENT_MAX_DAYS)
            .unwrap_or(false);
        if !within_window {
            continue;
        }
        // Clamp at Medium for the 30-day window; consumers with
        // tighter thresholds can layer additional downgrades on
        // top.
        if matches!(verdict.confidence, Confidence::High) {
            verdict.apply_downgrade_to(Confidence::Medium);
            downgraded += 1;
        }
    }
    downgraded
}

/// Downgrade `NoCallers` verdicts whose file has an invalidating
/// [`graphengine_parsing::application::ports::CoverageGap`] from
/// `High` to `Medium` and append the
/// `"extraction_coverage_r39_r41"` marker to the verdict's
/// `evidence` string (grep-able in the health-report payload).
///
/// This is the T8 dissolving-element for §3 Q5 ("filter candidates
/// vs keep them, both wrong in isolation"): the headline metric
/// stays unchanged — every `NoCallers` verdict is still emitted —
/// but the confidence it carries is honest about the extractor
/// gap. Downstream consumers that want the cleaner "only verdicts
/// we are confident about" view iterate over `Confidence::High`
/// verdicts, which is exactly what the
/// `no_callers_high_confidence` companion metric reports.
///
/// `coverage` is a slice rather than a map so callers can pass
/// [`crate::health::report::HealthReport::file_extraction_coverage`]
/// directly. When the slice is empty the function is a no-op, so
/// reports from languages without a coverage pass flow through
/// unchanged.
///
/// Returns the number of verdicts whose confidence was lowered.
/// Design: `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`
/// §4.3.
pub fn apply_extraction_coverage_downgrade(
    verdicts: &mut [DeadCodeVerdict],
    graph: &AnalysisGraph,
    coverage: &[graphengine_parsing::application::ports::FileExtractionCoverage],
) -> usize {
    if coverage.is_empty() {
        return 0;
    }
    // Build a set of repo-relative / absolute path strings that have
    // an invalidating gap. We key by the same two paths the T7
    // downgrader uses — `path_repo_rel` preferred, `file_path`
    // fallback — so both layers of evidence meet at a consistent
    // identifier.
    let invalidating_files: std::collections::HashSet<std::path::PathBuf> = coverage
        .iter()
        .filter(|c| c.has_invalidating_no_callers_gap())
        .map(|c| c.file_path.clone())
        .collect();
    if invalidating_files.is_empty() {
        return 0;
    }

    let mut downgraded: usize = 0;
    for verdict in verdicts.iter_mut() {
        // T8 concerns only the `NoCallers` reason (and the
        // `fan_in`-derived verdicts that share its invisibility
        // problem). Other reasons — disabled trigger, metadata
        // orphan, unreachable entry-point — are unaffected by an
        // unwalked accessor body, so we leave them at the
        // classifier's original confidence.
        if !matches!(verdict.reason, DeadCodeReason::NoCallers) {
            continue;
        }
        let Some(node) = graph.nodes.get(&verdict.node_id) else {
            continue;
        };
        let matches = match (node.path_repo_rel.as_deref(), node.file_path.as_deref()) {
            (Some(rel), Some(abs)) => {
                let rel_path = std::path::PathBuf::from(rel);
                let abs_path = std::path::PathBuf::from(abs);
                invalidating_files.contains(&rel_path) || invalidating_files.contains(&abs_path)
            }
            (Some(p), None) | (None, Some(p)) => {
                invalidating_files.contains(&std::path::PathBuf::from(p))
            }
            (None, None) => false,
        };
        if !matches {
            continue;
        }
        if matches!(verdict.confidence, Confidence::High) {
            verdict.apply_downgrade_to(Confidence::Medium);
            if !verdict.evidence.contains("extraction_coverage_r39_r41") {
                if verdict.evidence.is_empty() {
                    verdict.evidence = "extraction_coverage_r39_r41".to_string();
                } else {
                    verdict.evidence.push_str("; extraction_coverage_r39_r41");
                }
            }
            downgraded += 1;
        }
    }
    downgraded
}

/// Empty reason-breakdown map with every reason pre-populated at
/// zero.
pub fn empty_reason_breakdown() -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for r in DeadCodeReason::all() {
        out.insert(r.as_str().to_string(), 0);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::graph::{AnalysisGraph, EdgeKind, GraphEdge, GraphNode, NodeKind};
    use std::collections::BTreeMap;

    /// Mirror the exact extraction logic of
    /// `GraphNode::extract_name` in
    /// `graphengine-analysis/src/health/graph.rs` so test fixtures
    /// reflect production `node.name` shape faithfully.
    ///
    /// Intentionally *not* smarter than production: for real Apex
    /// FQNs (`…::<Class>::<method>(<params>)`) this returns
    /// `<method>(<params>)` with the parameter tuple intact. That
    /// leak is tracked as R29 in
    /// `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md`.
    /// Once R29 lands, this helper and `extract_name` should be
    /// updated together.
    fn derive_simple_name(fqn: &str) -> String {
        fqn.rsplit("::").next().unwrap_or(fqn).to_string()
    }

    fn mk_fn_with(
        id: &str,
        fqn: &str,
        file: &str,
        tags: Vec<&str>,
        visibility: Option<&str>,
        language: Option<&str>,
        frameworks: Vec<&str>,
    ) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: NodeKind::Function,
            fqn: fqn.into(),
            name: derive_simple_name(fqn),
            file_path: Some(file.into()),
            start_line: None,
            end_line: None,
            path_repo_rel: Some(file.into()),
            role: None,
            is_test: false,
            is_vendor: false,
            is_build_output: false,
            is_generated: false,
            cyclomatic_complexity: None,
            cognitive_complexity: None,
            visibility: visibility.map(str::to_string),
            import_sources: vec![],
            is_trait_impl: false,
            trait_name: None,
            is_attribute_invoked: false,
            is_callback_target: false,
            entry_point_tags: tags.into_iter().map(String::from).collect(),
            language: language.map(str::to_string),
            frameworks: frameworks.into_iter().map(String::from).collect(),
            is_synthetic: false,
        }
    }

    fn mk_file_with(
        id: &str,
        path: &str,
        is_test: bool,
        language: Option<&str>,
        frameworks: Vec<&str>,
    ) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: NodeKind::File,
            fqn: format!("file::{id}"),
            name: id.into(),
            file_path: Some(path.into()),
            start_line: None,
            end_line: None,
            path_repo_rel: Some(path.into()),
            role: Some("source".into()),
            is_test,
            is_vendor: false,
            is_build_output: false,
            is_generated: false,
            cyclomatic_complexity: None,
            cognitive_complexity: None,
            visibility: None,
            import_sources: vec![],
            is_trait_impl: false,
            trait_name: None,
            is_attribute_invoked: false,
            is_callback_target: false,
            entry_point_tags: vec![],
            language: language.map(str::to_string),
            frameworks: frameworks.into_iter().map(String::from).collect(),
            is_synthetic: false,
        }
    }

    fn build(nodes: Vec<GraphNode>, edges: Vec<(&str, &str, EdgeKind)>) -> AnalysisGraph {
        let mut m = BTreeMap::new();
        for n in nodes {
            m.insert(n.id.clone(), n);
        }
        let es = edges
            .into_iter()
            .map(|(f, t, k)| GraphEdge {
                from_id: f.into(),
                to_id: t.into(),
                kind: k,
                confidence: crate::health::graph::Confidence::High,
            })
            .collect();
        let mut g = AnalysisGraph::build(m, es);
        g.compute_module_membership();
        g
    }

    fn ctx<'a>(g: &'a AnalysisGraph, id: &'a str, eco: Ecosystem) -> ClassifyContext<'a> {
        ClassifyContext {
            node_id: id,
            graph: g,
            ecosystem: eco,
            fan_in: g.fan_in(id),
        }
    }

    #[test]
    fn empty_breakdown_contains_all_keys() {
        let m = empty_reason_breakdown();
        for r in DeadCodeReason::all() {
            assert_eq!(m.get(r.as_str()), Some(&0));
        }
    }

    #[test]
    fn universal_entry_point_tag_wins_before_framework_rules() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with("f", "classes/Svc.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "Svc.getAccount",
                    "classes/Svc.cls",
                    vec!["aura_enabled"],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::Apex));
        assert_eq!(v.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
        assert_eq!(v.classifier, "universal-entry-point-tag");
    }

    #[test]
    fn universal_parent_is_test_wins_over_tdtm_pattern() {
        // Regression for the Wave 1 Layer-5 fix: a TDTM-named class
        // whose filename is `*_TEST.cls` must be classified as test,
        // not as TDTM dynamic dispatch. Under framework-keyed
        // dispatch, parent-is-test is a universal pre-rule so it
        // wins before the TDTM rule set runs.
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "classes/CON_ContactMergeTDTM_TEST.cls",
                    true,
                    Some("apex"),
                    vec!["tdtm"],
                ),
                mk_fn_with(
                    "m",
                    "classes/CON_ContactMergeTDTM_TEST.cls::CON_ContactMergeTDTM_TEST::run()",
                    "classes/CON_ContactMergeTDTM_TEST.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["tdtm"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::Apex));
        assert_eq!(v.reason, DeadCodeReason::TestOnlyReference);
        assert_eq!(v.classifier, "universal-parent-is-test");
    }

    #[test]
    fn framework_keyed_dispatch_to_tdtm_rule() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "classes/TDTM_Opportunity.cls",
                    false,
                    Some("apex"),
                    vec!["tdtm"],
                ),
                mk_fn_with(
                    "m",
                    "classes/TDTM_Opportunity.cls::TDTM_Opportunity::run()",
                    "classes/TDTM_Opportunity.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["tdtm"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::Apex));
        assert_eq!(v.reason, DeadCodeReason::DynamicDispatchTarget);
        assert_eq!(v.classifier, "apex-tdtm");
    }

    #[test]
    fn framework_keyed_dispatch_to_triggerdml_rule() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "triggers/AccountTrigger.trigger",
                    false,
                    Some("apex"),
                    vec!["triggerdml"],
                ),
                mk_fn_with(
                    "m",
                    "triggers/AccountTrigger.trigger::AccountTrigger::run()",
                    "triggers/AccountTrigger.trigger",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["triggerdml"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::Apex));
        assert_eq!(v.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
        assert_eq!(v.classifier, "apex-triggerdml");
    }

    #[test]
    fn framework_keyed_dispatch_to_django_cbv() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with("f", "app/views.py", false, Some("python"), vec!["django"]),
                mk_fn_with(
                    "m",
                    "views.get",
                    "app/views.py",
                    vec![],
                    None,
                    Some("python"),
                    vec!["django"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::Python));
        assert_eq!(v.reason, DeadCodeReason::DeclarativeWiringUnparsed);
        assert_eq!(v.classifier, "python-django");
    }

    #[test]
    fn framework_keyed_dispatch_to_aura_rule() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "force-app/main/default/aura/Hello/HelloController.js",
                    false,
                    Some("javascript"),
                    vec!["aura"],
                ),
                mk_fn_with(
                    "m",
                    "Hello.handleSave",
                    "force-app/main/default/aura/Hello/HelloController.js",
                    vec![],
                    Some("public"),
                    Some("javascript"),
                    vec!["aura"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::JavaScript));
        assert_eq!(v.reason, DeadCodeReason::DeclarativeWiringUnparsed);
        assert_eq!(v.classifier, "js-aura");
        assert!(v.evidence.contains("Aura bundle"));
    }

    #[test]
    fn framework_keyed_dispatch_to_jest_rule() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "jest.setup.js",
                    false,
                    Some("javascript"),
                    vec!["jest"],
                ),
                mk_fn_with(
                    "m",
                    "setupGlobals",
                    "jest.setup.js",
                    vec![],
                    None,
                    Some("javascript"),
                    vec!["jest"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::JavaScript));
        assert_eq!(v.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
        assert_eq!(v.classifier, "js-jest");
        assert!(v.evidence.contains("Jest"));
        assert!(v.evidence.contains("jest.setup.js"));
    }

    #[test]
    fn framework_keyed_dispatch_to_vitest_rule() {
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "vitest.setup.ts",
                    false,
                    Some("typescript"),
                    vec!["vitest"],
                ),
                mk_fn_with(
                    "m",
                    "setupGlobals",
                    "vitest.setup.ts",
                    vec![],
                    None,
                    Some("typescript"),
                    vec!["vitest"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::JavaScript));
        assert_eq!(v.reason, DeadCodeReason::FrameworkAnnotationUnresolved);
        assert_eq!(v.classifier, "js-vitest");
        assert!(v.evidence.contains("Vitest"));
        assert!(v.evidence.contains("vitest.setup.ts"));
    }

    #[test]
    fn aura_rule_defers_when_fan_in_is_nonzero() {
        // A non-dead Aura symbol (hypothetically; in the pipeline,
        // dead-code would not even invoke the classifier) must not
        // receive a DeclarativeWiringUnparsed stamp. Proves the
        // fan_in guard in AuraRules.
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "f",
                    "aura/Hello/HelloController.js",
                    false,
                    Some("javascript"),
                    vec!["aura"],
                ),
                mk_fn_with(
                    "caller",
                    "Hello.init",
                    "aura/Hello/HelloController.js",
                    vec![],
                    Some("public"),
                    Some("javascript"),
                    vec!["aura"],
                ),
                mk_fn_with(
                    "m",
                    "Hello.handleSave",
                    "aura/Hello/HelloController.js",
                    vec![],
                    Some("public"),
                    Some("javascript"),
                    vec!["aura"],
                ),
            ],
            vec![
                ("f", "caller", EdgeKind::Contains),
                ("f", "m", EdgeKind::Contains),
                ("caller", "m", EdgeKind::Call),
            ],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::JavaScript));
        // fan_in=1 → Aura rule returns None; falls through to
        // generic. visibility=public on a JS symbol is not
        // private-unused either.
        assert_ne!(v.classifier, "js-aura");
    }

    #[test]
    fn plain_rule_falls_through_to_generic() {
        // A `plain`-tagged node with no framework evidence should
        // fall through to the generic visibility-based classifier.
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with("f", "src/util.py", false, Some("python"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "util._helper",
                    "src/util.py",
                    vec![],
                    Some("private"),
                    Some("python"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let v = reg.classify(&ctx(&g, "m", Ecosystem::Python));
        // name starts with `_` → VisibilityPrivateUnused on the
        // generic classifier (python convention).
        assert_eq!(v.reason, DeadCodeReason::VisibilityPrivateUnused);
        assert_eq!(v.classifier, "generic");
    }

    #[test]
    fn polyglot_repo_routes_per_node_not_per_repo() {
        // Same registry, same graph, two files: one Apex TDTM,
        // one Python Django. Each node routes to its own framework
        // rule set. This is the core of the Wave 2 promise: polyglot
        // repos no longer collapse to a single ecosystem bucket.
        let reg = FrameworkRuleRegistry::default();
        let g = build(
            vec![
                mk_file_with(
                    "apex_f",
                    "classes/TDTM_Opportunity.cls",
                    false,
                    Some("apex"),
                    vec!["tdtm"],
                ),
                mk_fn_with(
                    "apex_m",
                    "classes/TDTM_Opportunity.cls::TDTM_Opportunity::run()",
                    "classes/TDTM_Opportunity.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["tdtm"],
                ),
                mk_file_with(
                    "py_f",
                    "app/views.py",
                    false,
                    Some("python"),
                    vec!["django"],
                ),
                mk_fn_with(
                    "py_m",
                    "views.get",
                    "app/views.py",
                    vec![],
                    None,
                    Some("python"),
                    vec!["django"],
                ),
            ],
            vec![
                ("apex_f", "apex_m", EdgeKind::Contains),
                ("py_f", "py_m", EdgeKind::Contains),
            ],
        );
        // Advisory Ecosystem hint is Apex (to prove it's ignored).
        let apex_v = reg.classify(&ctx(&g, "apex_m", Ecosystem::Apex));
        let py_v = reg.classify(&ctx(&g, "py_m", Ecosystem::Apex));
        assert_eq!(apex_v.classifier, "apex-tdtm");
        assert_eq!(py_v.classifier, "python-django");
    }

    fn dead_verdict(node_id: &str) -> DeadCodeVerdict {
        DeadCodeVerdict {
            node_id: node_id.to_string(),
            reason: DeadCodeReason::NoCallers,
            evidence: "fan_in=0".into(),
            classifier: "generic",
            confidence: Confidence::High,
        }
    }

    fn git_report_with(
        path: &str,
        last_touched_days: u32,
        conf: graphengine_git_signals::Confidence,
    ) -> graphengine_git_signals::GitSignalReport {
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        let mut per_file = BTreeMap::new();
        per_file.insert(
            PathBuf::from(path),
            graphengine_git_signals::FileSignals {
                change_frequency: 3,
                distinct_authors: 1,
                last_touched_days: Some(last_touched_days),
                ownership_dispersion: 0.0,
                hotspot_score: 3.0,
                confidence: conf,
            },
        );
        graphengine_git_signals::GitSignalReport {
            repository_shape: graphengine_git_signals::RepoShape::Full,
            per_file,
            co_change_clusters: Vec::new(),
            integrity_caveats: vec![
                graphengine_git_signals::CAVEAT_LAYER0_GIT_SIGNALS_V1.to_string()
            ],
            commits_walked: 3,
            files_touched: 1,
        }
    }

    #[test]
    fn apply_git_signal_downgrade_is_no_op_when_git_signals_is_none() {
        let g = build(
            vec![
                mk_file_with("f", "src/a.rs", false, Some("rust"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "a::helper",
                    "src/a.rs",
                    vec![],
                    Some("private"),
                    Some("rust"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![dead_verdict("m")];
        let n = apply_git_signal_churn_downgrade(&mut verdicts, &g, None);
        assert_eq!(n, 0);
        assert!(matches!(verdicts[0].confidence, Confidence::High));
    }

    #[test]
    fn apply_git_signal_downgrade_downgrades_on_recent_high_churn() {
        let g = build(
            vec![
                mk_file_with("f", "src/a.rs", false, Some("rust"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "a::helper",
                    "src/a.rs",
                    vec![],
                    Some("private"),
                    Some("rust"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![dead_verdict("m")];
        let report = git_report_with("src/a.rs", 5, graphengine_git_signals::Confidence::High);
        let n = apply_git_signal_churn_downgrade(&mut verdicts, &g, Some(&report));
        assert_eq!(n, 1);
        assert!(matches!(verdicts[0].confidence, Confidence::Medium));
    }

    #[test]
    fn apply_git_signal_downgrade_respects_shallow_clone_guard() {
        // Git-signal confidence Low => never downgrade, even on
        // ostensibly recent files. Mirrors the load-bearing
        // shallow-clone invariant at the classifier layer.
        let g = build(
            vec![
                mk_file_with("f", "src/a.rs", false, Some("rust"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "a::helper",
                    "src/a.rs",
                    vec![],
                    Some("private"),
                    Some("rust"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![dead_verdict("m")];
        let report = git_report_with("src/a.rs", 3, graphengine_git_signals::Confidence::Low);
        let n = apply_git_signal_churn_downgrade(&mut verdicts, &g, Some(&report));
        assert_eq!(n, 0);
        assert!(matches!(verdicts[0].confidence, Confidence::High));
    }

    #[test]
    fn apply_downgrade_to_clamps_and_does_not_promote() {
        let mut v = dead_verdict("x");
        v.apply_downgrade_to(Confidence::Medium);
        assert!(matches!(v.confidence, Confidence::Medium));
        // Now try to "promote" via another downgrade call with a
        // higher target. clamp must keep the stricter (lower) one.
        v.apply_downgrade_to(Confidence::High);
        assert!(matches!(v.confidence, Confidence::Medium));
        v.apply_downgrade_to(Confidence::Low);
        assert!(matches!(v.confidence, Confidence::Low));
    }

    #[test]
    fn build_reason_breakdown_counts_correctly() {
        let verdicts = vec![
            DeadCodeVerdict {
                node_id: "a".into(),
                reason: DeadCodeReason::NoCallers,
                evidence: "".into(),
                classifier: "generic",
                confidence: Confidence::High,
            },
            DeadCodeVerdict {
                node_id: "b".into(),
                reason: DeadCodeReason::NoCallers,
                evidence: "".into(),
                classifier: "generic",
                confidence: Confidence::High,
            },
            DeadCodeVerdict {
                node_id: "c".into(),
                reason: DeadCodeReason::FrameworkAnnotationUnresolved,
                evidence: "".into(),
                classifier: "apex-restresource",
                confidence: Confidence::High,
            },
        ];
        let bd = build_reason_breakdown(&verdicts);
        assert_eq!(bd.get("no_callers"), Some(&2));
        assert_eq!(bd.get("framework_annotation_unresolved"), Some(&1));
        assert_eq!(bd.get("visibility_private_unused"), Some(&0));
    }

    // -----------------------------------------------------------------
    // T8 — extraction-coverage downgrade (classifier-level)
    // -----------------------------------------------------------------
    //
    // Guard rails we exercise here (T8 design §4.3, acceptance
    // criterion #4):
    //
    //   * Absent coverage (empty slice) must be a strict no-op —
    //     never promote, never demote, never mutate evidence.
    //   * Records without invalidating gaps are ignored — only
    //     `CoverageGap::*{invalidates_no_callers}` variants can
    //     trigger a downgrade. This lets us grow `CoverageGap`
    //     variants for telemetry-only gaps without paying for a
    //     classifier change.
    //   * Downgrade only the `NoCallers` reason. Other reasons
    //     (framework annotation, visibility-private-unused, etc.)
    //     have their own evidence trail and are not gated by AST
    //     walk completeness.
    //   * Never touch a verdict below `High`. The T7 churn
    //     downgrader may have already clamped to `Medium`; calling
    //     the two downgraders in either order must leave the
    //     verdict at `Medium` (commutativity).
    //   * Evidence string is appended at most once — repeated
    //     calls do not grow the string.
    fn cov_file(
        path: &str,
        gaps: Vec<graphengine_parsing::application::ports::CoverageGap>,
    ) -> graphengine_parsing::application::ports::FileExtractionCoverage {
        use graphengine_parsing::application::ports::{CoverageConfidence, FileExtractionCoverage};
        FileExtractionCoverage {
            file_path: std::path::PathBuf::from(path),
            language: "apex".to_string(),
            walked_node_count: 100,
            unwalked_node_count: 0,
            coverage_gaps: gaps,
            confidence: CoverageConfidence::High,
        }
    }

    #[test]
    fn extraction_coverage_downgrade_is_no_op_on_empty_coverage() {
        let g = build(
            vec![
                mk_file_with("f", "src/A.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "A.foo",
                    "src/A.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![dead_verdict("m")];
        let n = apply_extraction_coverage_downgrade(&mut verdicts, &g, &[]);
        assert_eq!(n, 0);
        assert!(matches!(verdicts[0].confidence, Confidence::High));
        assert_eq!(verdicts[0].evidence, "fan_in=0");
    }

    #[test]
    fn extraction_coverage_downgrade_ignores_files_without_invalidating_gap() {
        use graphengine_parsing::application::ports::CoverageGap;
        let g = build(
            vec![
                mk_file_with("f", "src/A.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "A.foo",
                    "src/A.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![dead_verdict("m")];
        // Zero-count gap is not invalidating (predicate guards on
        // `count > 0`).
        let coverage = vec![cov_file(
            "src/A.cls",
            vec![CoverageGap::ApexPropertyAccessor { count: 0 }],
        )];
        let n = apply_extraction_coverage_downgrade(&mut verdicts, &g, &coverage);
        assert_eq!(n, 0);
        assert!(matches!(verdicts[0].confidence, Confidence::High));
    }

    #[test]
    fn extraction_coverage_downgrade_applies_to_matching_no_callers() {
        use graphengine_parsing::application::ports::CoverageGap;
        let g = build(
            vec![
                mk_file_with("f", "src/A.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "A.foo",
                    "src/A.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![dead_verdict("m")];
        let coverage = vec![cov_file(
            "src/A.cls",
            vec![CoverageGap::ApexPropertyAccessor { count: 1 }],
        )];
        let n = apply_extraction_coverage_downgrade(&mut verdicts, &g, &coverage);
        assert_eq!(n, 1);
        assert!(matches!(verdicts[0].confidence, Confidence::Medium));
        assert!(verdicts[0].evidence.contains("extraction_coverage_r39_r41"));
        // Idempotent: second call neither downgrades further nor
        // grows the evidence string.
        let evidence_before = verdicts[0].evidence.clone();
        let n2 = apply_extraction_coverage_downgrade(&mut verdicts, &g, &coverage);
        assert_eq!(n2, 0);
        assert_eq!(verdicts[0].evidence, evidence_before);
    }

    #[test]
    fn extraction_coverage_downgrade_skips_non_no_callers_reasons() {
        use graphengine_parsing::application::ports::CoverageGap;
        let g = build(
            vec![
                mk_file_with("f", "src/A.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "A.foo",
                    "src/A.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        let mut verdicts = vec![DeadCodeVerdict {
            node_id: "m".into(),
            reason: DeadCodeReason::VisibilityPrivateUnused,
            evidence: "visibility=private".into(),
            classifier: "generic",
            confidence: Confidence::High,
        }];
        let coverage = vec![cov_file(
            "src/A.cls",
            vec![CoverageGap::ApexPropertyAccessor { count: 1 }],
        )];
        let n = apply_extraction_coverage_downgrade(&mut verdicts, &g, &coverage);
        assert_eq!(n, 0);
        assert!(matches!(verdicts[0].confidence, Confidence::High));
        assert_eq!(verdicts[0].evidence, "visibility=private");
    }

    #[test]
    fn extraction_coverage_downgrade_cannot_promote_below_high() {
        use graphengine_parsing::application::ports::CoverageGap;
        let g = build(
            vec![
                mk_file_with("f", "src/A.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "A.foo",
                    "src/A.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        // Simulate T7 already having clamped to Medium. The T8
        // downgrader must leave the verdict at Medium; the two
        // passes commute because both clamp to Medium on
        // `NoCallers`.
        let mut verdicts = vec![DeadCodeVerdict {
            node_id: "m".into(),
            reason: DeadCodeReason::NoCallers,
            evidence: "fan_in=0; git_signals_churn_downgrade".into(),
            classifier: "generic",
            confidence: Confidence::Medium,
        }];
        let coverage = vec![cov_file(
            "src/A.cls",
            vec![CoverageGap::ApexMapLiteralInitializer { count: 1 }],
        )];
        let n = apply_extraction_coverage_downgrade(&mut verdicts, &g, &coverage);
        assert_eq!(n, 0, "must not demote below High; only High→Medium clamp");
        assert!(matches!(verdicts[0].confidence, Confidence::Medium));
    }

    #[test]
    fn extraction_coverage_downgrade_matches_on_path_repo_rel_or_absolute() {
        use graphengine_parsing::application::ports::CoverageGap;
        let g = build(
            vec![
                mk_file_with("f", "src/A.cls", false, Some("apex"), vec!["plain"]),
                mk_fn_with(
                    "m",
                    "A.foo",
                    "src/A.cls",
                    vec![],
                    Some("public"),
                    Some("apex"),
                    vec!["plain"],
                ),
            ],
            vec![("f", "m", EdgeKind::Contains)],
        );
        // Coverage record key diverges from node.path_repo_rel —
        // the downgrader must refuse to match. This is the
        // contract: absent evidence is not negative evidence, and
        // a path-key mismatch counts as absent.
        let mut verdicts = vec![dead_verdict("m")];
        let coverage = vec![cov_file(
            "SomeOtherFile.cls",
            vec![CoverageGap::ApexPropertyAccessor { count: 1 }],
        )];
        let n = apply_extraction_coverage_downgrade(&mut verdicts, &g, &coverage);
        assert_eq!(n, 0);
        assert!(matches!(verdicts[0].confidence, Confidence::High));
    }
}
