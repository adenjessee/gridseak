//! NPSP Table-Driven Trigger Management (TDTM) rule set.
//!
//! TDTM dispatches trigger handlers via reflection —
//! `Type.forName(className).newInstance()` — so the static call
//! graph never sees the edge from the trigger to the handler's
//! `run()` method. Any Apex file tagged with the `tdtm` framework
//! (see `graphengine-parsing/src/domain/frameworks.rs`) has a
//! high probability of containing such a handler. The rule matches
//! a conservative *class-name convention* rather than parsing the
//! NPSP trigger router; the authoritative resolver is Workstream B
//! (`docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`).
//!
//! # Heuristic scope (post TR-0.1.1)
//!
//! The FQN is decomposed before matching so parameter-type
//! fragments cannot contribute:
//!
//! 1. Drop the parameter tuple by splitting on the first `(`.
//! 2. From the remaining header (`<path>::<Outer>.<Inner>::<method>`),
//!    isolate the class and method segments via `rsplit("::")`.
//! 3. Only the **outermost class token** (dot-split class segment,
//!    first element) is checked against the TDTM convention.
//!    NPSP's TDTM router registers handlers by top-level class
//!    name (`Type.forName(outerClassName).newInstance()`); inner
//!    classes, even TDTM-named ones, are not reflectively
//!    dispatched — they are invoked by ordinary Apex from inside
//!    the outer class.
//! 4. On a TDTM-named outer class the match is further restricted
//!    to the two methods the NPSP router actually invokes
//!    reflectively: `run()` (the `TDTM_Runnable` contract method)
//!    and the class's **zero-arg constructor** (the `newInstance()`
//!    call). Every other method on a TDTM outer class —
//!    `onAfterUpdate()`, `getCascadeDeleteLoader()`, etc. — is
//!    called by ordinary Apex (override polymorphism, typed-field
//!    dispatch, sibling-method calls) and falls through to the
//!    generic classifier, where its true failure mode (R23 resolver
//!    gap) is surfaced as `no_callers` rather than falsely
//!    attributed to reflection.
//! 5. Fallback: a method named `run` on a class whose name
//!    contains `trigger` or `handler` (both checks scoped to class
//!    segments only, never the path or parameter tuple). This
//!    preserves non-NPSP trigger-handler conventions that do not
//!    use the TDTM token shape.
//!
//! This replaces the pre-R24 substring scan (full-FQN match) and
//! the pre-R31 `class_tokens.any(...)` cross-product match. The
//! full resolver (R26 / Phase D of
//! `docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`)
//! still supersedes this heuristic.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

pub struct TdtmRules;

impl FrameworkRuleSet for TdtmRules {
    fn framework(&self) -> &'static str {
        "tdtm"
    }
    fn name(&self) -> &'static str {
        "apex-tdtm"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;
        if looks_like_tdtm_handler(&node.fqn, &node.name) {
            return Some((
                DeadCodeReason::DynamicDispatchTarget,
                format!(
                    "fan_in={}; matches TDTM handler convention (fqn={}); called via Type.forName().newInstance()",
                    ctx.fan_in, node.fqn
                ),
            ));
        }
        None
    }
}

/// Return true when the FQN/name pair matches the NPSP TDTM
/// handler convention. See the module docstring for the full
/// decomposition and the post-R31 outer-class / method-identity
/// restriction.
fn looks_like_tdtm_handler(fqn: &str, name: &str) -> bool {
    // Strip the parameter tuple. Anything to the right of the first
    // `(` is a signature fragment (possibly containing scoped type
    // names like `TDTM_Runnable.DmlWrapper`) and must not contribute.
    let header = fqn.split_once('(').map(|(h, _)| h).unwrap_or(fqn);

    // Decompose `<path>::<Outer>.<Inner>::<method>`. `rsplit` walks
    // from the tail so the first yielded segment is the method name
    // and the second is the enclosing-type path.
    let mut segs = header.rsplit("::");
    let method_seg = segs.next().unwrap_or("").to_ascii_lowercase();
    let class_seg = segs.next().unwrap_or("").to_ascii_lowercase();
    let name_lc = name.to_ascii_lowercase();

    // Inner classes arrive dotted (`Outer.Inner`). NPSP's TDTM
    // router invokes the *outer* class via `Type.forName(...)`, so
    // only the outermost token decides TDTM membership. Matching
    // on any token (pre-R31) over-attributes inner-class methods
    // to reflection when they are in fact invoked by ordinary Apex
    // from inside the outer class.
    let class_tokens: Vec<&str> = class_seg.split('.').filter(|t| !t.is_empty()).collect();
    let outer_class_token = class_tokens.first().copied().unwrap_or("");
    let outer_is_tdtm = is_tdtm_token(outer_class_token);

    let method_is_run = method_seg == "run" || name_lc == "run";
    // Apex constructors share the class's identifier. Match only
    // when the method segment equals the *outermost* class token —
    // i.e. the zero-arg constructor that `newInstance()` invokes.
    // Inner-class constructors (`method_seg == inner_token`) do
    // not satisfy this because `outer_class_token` is the outer.
    let method_is_outer_ctor = !method_seg.is_empty() && method_seg == outer_class_token;

    let run_on_handler = method_is_run
        && class_tokens
            .iter()
            .any(|t| t.contains("trigger") || t.contains("handler"));

    (outer_is_tdtm && (method_is_run || method_is_outer_ctor)) || run_on_handler
}

/// A class-segment token evidences the TDTM naming convention iff
/// it uses one of the three documented shapes:
///
/// - prefix: `TDTM_<Something>` (e.g. `TDTM_Opportunity`).
/// - suffix: `<Something>_TDTM` (e.g. `ACCT_Accounts_TDTM`).
/// - exact: bare `TDTM` (rare, but cheap to cover).
///
/// Deliberately *not* `ends_with("tdtm")` without a separator —
/// that would catch coincidental identifiers and undo the R24 fix.
fn is_tdtm_token(token_lc: &str) -> bool {
    token_lc == "tdtm" || token_lc.starts_with("tdtm_") || token_lc.ends_with("_tdtm")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Positive cases: real parser-shape FQNs ----

    #[test]
    fn tdtm_prefix_class_with_run_method_matches() {
        assert!(looks_like_tdtm_handler(
            "/ws/force-app/classes/TDTM_Opportunity.cls::TDTM_Opportunity::run()",
            "run"
        ));
    }

    #[test]
    fn tdtm_prefix_class_without_path_still_matches() {
        assert!(looks_like_tdtm_handler("::TDTM_Opportunity::run()", "run"));
    }

    #[test]
    fn tdtm_suffix_class_run_with_tdtm_typed_params_matches() {
        // R24-regression coverage with a shape that is actually
        // reflectively dispatched: `run(...)` on a `<Name>_TDTM`
        // outer class whose signature carries a `TDTM_Runnable.*`
        // scoped type in the parameter tuple. Proves parameter
        // types still cannot smuggle themselves into the match,
        // while keeping the assertion aligned with reality (post-
        // R31 only `run()` / ctor on outer TDTM classes match).
        assert!(looks_like_tdtm_handler(
            "::ACCT_Accounts_TDTM::run(List,List,TDTM_Runnable.Action,Schema.DescribeSObjectResult)",
            "run"
        ));
    }

    #[test]
    fn tdtm_outer_class_zero_arg_constructor_matches() {
        // R31: the `newInstance()` call reflectively invokes the
        // zero-arg constructor of the outer TDTM class. The Apex
        // parser emits constructors with method name == class name.
        assert!(looks_like_tdtm_handler(
            "::TDTM_Opportunity::TDTM_Opportunity()",
            "TDTM_Opportunity"
        ));
    }

    #[test]
    fn tdtm_outer_class_zero_arg_constructor_suffix_shape_matches() {
        // Same shape, `<Name>_TDTM` instead of `TDTM_<Name>`.
        assert!(looks_like_tdtm_handler(
            "::ACCT_Accounts_TDTM::ACCT_Accounts_TDTM()",
            "ACCT_Accounts_TDTM"
        ));
    }

    #[test]
    fn run_on_handler_named_class_matches_via_fallback() {
        assert!(looks_like_tdtm_handler(
            "::SomeTriggerHandler::run()",
            "run"
        ));
    }

    // ---- Negative cases: R24 repros (must NOT match) ----

    #[test]
    fn r24_account_adapter_with_tdtm_param_does_not_match() {
        // The canonical R24 repro: non-handler method that merely
        // accepts a TDTM scoped type.
        assert!(!looks_like_tdtm_handler(
            "::AccountAdapter::onAfterUpdate(TDTM_Runnable.DmlWrapper)",
            "onAfterUpdate"
        ));
    }

    #[test]
    fn r24_sibling_shape_util_with_tdtm_param_does_not_match() {
        assert!(!looks_like_tdtm_handler(
            "::DmlWrapperUtil::process(TDTM_Runnable.DmlWrapper)",
            "process"
        ));
    }

    #[test]
    fn pre_canonical_generic_with_tdtm_type_argument_does_not_match() {
        // Parser canonicalises `List<TDTM_Runnable>` to `List`, but
        // this guards against future regressions if the signature
        // emitter ever preserves type arguments.
        assert!(!looks_like_tdtm_handler(
            "::Svc::doWork(List<TDTM_Runnable>)",
            "doWork"
        ));
    }

    #[test]
    fn pre_canonical_nested_generic_with_tdtm_type_argument_does_not_match() {
        assert!(!looks_like_tdtm_handler(
            "::Svc::doWork(Map<Id,TDTM_Runnable.DmlWrapper>)",
            "doWork"
        ));
    }

    #[test]
    fn plain_helper_with_tdtm_typed_param_does_not_match() {
        assert!(!looks_like_tdtm_handler(
            "::Svc::helper(TDTM_Runnable)",
            "helper"
        ));
    }

    #[test]
    fn bare_run_on_non_handler_class_does_not_match() {
        assert!(!looks_like_tdtm_handler("::Svc::run()", "run"));
    }

    #[test]
    fn unrelated_name_does_not_match() {
        assert!(!looks_like_tdtm_handler("::Svc::helper()", "helper"));
    }

    // ---- Negative cases: R31 repros (must NOT match) ----

    #[test]
    fn r31_inner_class_method_inside_outer_tdtm_does_not_match() {
        // Inner-class non-`run` method nested inside a `<Name>_TDTM`
        // outer class. NPSP invokes `new CascadeDeleteLoader().load(...)`
        // from inside the outer class via ordinary Apex; the TDTM
        // router does not reflectively dispatch inner-class methods.
        assert!(!looks_like_tdtm_handler(
            "::CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)",
            "load"
        ));
    }

    #[test]
    fn r31_inner_class_constructor_inside_outer_tdtm_does_not_match() {
        // Inner-class constructor nested inside a `<Name>_TDTM`
        // outer class. Same reasoning as the method case; inner
        // ctors are invoked with `new Inner()` from outer-class
        // Apex, not by `Type.forName().newInstance()`.
        assert!(!looks_like_tdtm_handler(
            "::RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()",
            "FirstCascadeUndeleteLoader"
        ));
    }

    #[test]
    fn r31_outer_tdtm_non_run_non_ctor_method_does_not_match() {
        // Outer TDTM class, arbitrary helper method. Invoked by
        // override polymorphism (`CDL_CascadeDeleteLookups.cls`
        // calls this via a typed-field dispatch), not by
        // reflection. Pre-R31 this falsely matched because the
        // outer-class token satisfied `is_tdtm_token`.
        assert!(!looks_like_tdtm_handler(
            "::CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()",
            "getCascadeDeleteLoader"
        ));
    }

    #[test]
    fn r31_inner_class_with_tdtm_named_inner_does_not_match() {
        // Even when the *inner* class itself carries a TDTM
        // token (`Outer.TDTM_Inner`), NPSP's router registers
        // outer-class names only. Flipping the old
        // `inner_class_tdtm_prefix_matches` positive assertion
        // (which encoded the pre-R31 `any()` behaviour) to the
        // post-R31 reality.
        assert!(!looks_like_tdtm_handler(
            "::PlainOuter.TDTM_Inner::run()",
            "run"
        ));
    }

    // ---- Direct token-rule unit tests ----

    #[test]
    fn tdtm_token_rule_accepts_documented_shapes() {
        assert!(is_tdtm_token("tdtm"));
        assert!(is_tdtm_token("tdtm_opportunity"));
        assert!(is_tdtm_token("acct_accounts_tdtm"));
    }

    #[test]
    fn tdtm_token_rule_rejects_coincidental_matches() {
        // No separator → not accepted. `contactmergetdtm` is an
        // outlier NPSP convention; the fallback (`run` on
        // trigger/handler class) would not catch it either. That's
        // an accepted Phase-0 limitation; Phase B's resolver
        // replaces this heuristic with edge-based detection.
        assert!(!is_tdtm_token("contactmergetdtm"));
        assert!(!is_tdtm_token("atdtmx"));
        assert!(!is_tdtm_token("some_other_class"));
    }
}
