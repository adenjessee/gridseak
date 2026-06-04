//! Per-ecosystem confidence ceiling for the `dead_code` metric.
//!
//! Cross-file resolution quality (LSP vs heuristic vs nothing) is only
//! *one* axis of dead-code-detection trustworthiness. The other,
//! historically conflated, axis is **framework-attribute extraction
//! coverage**: even with a perfectly resolved call graph, the parser
//! cannot tell that
//!
//!   - a Rust function annotated `#[tool]` is invoked by `rust-mcp-sdk`'s
//!     macro-generated dispatcher,
//!   - a Python function decorated `@app.route("/foo")` is invoked by
//!     Flask's URL router,
//!   - a Java method annotated `@Component` is wired by Spring's DI,
//!
//! because those dispatch paths don't exist as edges in the static call
//! graph — they're synthesised at compile / load / runtime by framework
//! machinery the parser doesn't traverse.
//!
//! Historically [`super::mod::run_analysis_with_config`] mapped
//! `ResolutionTier::Full → Confidence::High` unconditionally, which
//! produced the documented B1 bug: the `metric_confidence.dead_code`
//! badge said *high* for Rust repos using `#[tool]` attribute macros,
//! while the per-finding caveat said *verify before removing*. That's
//! internally inconsistent and erodes trust.
//!
//! This module separates the two axes and computes the dead-code metric
//! confidence as the *minimum* of (resolution-tier ceiling, extraction-
//! coverage ceiling). The reason string names whichever axis lowered
//! the level, so the agent has language to quote when the user asks
//! "why is this medium confidence?".
//!
//! Adding a new language to the *full* coverage list is a deliberate
//! act: it should happen only after the framework-attribute extraction
//! machinery actually traverses that language's dispatch patterns end
//! to end (see [`crate::health::dead_code_classifier`]).

use std::collections::BTreeMap;

use super::config::Ecosystem;
use super::report::{Confidence, MetricConfidence, ResolutionTier};

/// How completely the parser+classifier pipeline traverses the
/// language's framework-attribute / framework-dispatch entry points.
/// A `Full` variant is the contract that "if this function is invoked
/// at runtime via the language's standard framework dispatch, we will
/// have an incoming edge or a classifier exemption for it." `Partial`
/// means we know there are specific dispatch patterns we don't see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameworkExtractionCoverage {
    /// Every framework-dispatch pattern for this language is either
    /// represented as a graph edge or handled by a classifier rule
    /// (so the dead-code verdict can't be a framework-dispatch false
    /// positive). `description` is a short list of the dispatch
    /// patterns we DO traverse, suitable for direct quoting in the
    /// confidence-reason string.
    Full { description: &'static str },
    /// Known dispatch patterns the analyzer does NOT traverse. The
    /// `missing` field is enumerated explicitly so the confidence
    /// reason can tell users / agents exactly which false-positive
    /// classes to expect.
    Partial { missing: &'static str },
    /// We don't know the language well enough to enumerate its
    /// framework-dispatch patterns. Treated more cautiously than
    /// `Partial` since we cannot even bound the risk.
    Unknown,
}

/// Per-ecosystem framework-extraction coverage report.
///
/// **Adding a language to `Full` coverage requires three things:**
/// 1. Per-framework classifiers under
///    [`crate::health::dead_code_classifier::frameworks`] that mark
///    the language's entry-point patterns.
/// 2. End-to-end test fixture demonstrating a framework-dispatched
///    function is NOT flagged dead.
/// 3. Updating the `description` field below to enumerate the
///    dispatch patterns now traversed (the agent will quote this).
pub fn framework_extraction_coverage(eco: Ecosystem) -> FrameworkExtractionCoverage {
    match eco {
        // Apex is the canonical full-coverage case: the dead_code
        // classifier knows @AuraEnabled, @InvocableMethod,
        // @RestResource, triggers, the `global`/`webservice` access
        // modifiers, and `implements Schedulable/Batchable/Queueable`.
        // Per-rule classifiers under
        // dead_code_classifier::frameworks::apex_* cover these.
        Ecosystem::Apex => FrameworkExtractionCoverage::Full {
            description: "Apex framework attributes (@AuraEnabled, \
                          @InvocableMethod, @RestResource, triggers, \
                          global/webservice, Schedulable/Batchable/Queueable)",
        },

        // Rust: attribute-macro dispatch is invisible to tree-sitter
        // (macros expand AFTER the AST is parsed). #[tool], #[mcp],
        // #[derive(...)], #[tokio::main], #[axum::handler], every
        // rust-mcp-sdk dispatcher — none of these produce static
        // call edges. Display/Debug fmt() dispatch via format!() is
        // similarly invisible. dyn Trait method dispatch produces
        // edges to a synthetic node, not the impl methods.
        Ecosystem::Rust => FrameworkExtractionCoverage::Partial {
            missing: "Rust attribute macros (#[tool], #[mcp], \
                      #[derive(...)], #[tokio::main], #[axum::handler]); \
                      Display/Debug trait dispatch via format!() / {} / \
                      {:?}; dyn Trait method dispatch",
        },

        // TS/JS: dynamic dispatch via this.method() and obj[key]() is
        // not statically resolvable. Decorators (Angular, NestJS) are
        // not traversed for framework-entry-point detection. JSX
        // higher-order components and render-props patterns can hide
        // component usage.
        Ecosystem::TypeScript | Ecosystem::JavaScript => FrameworkExtractionCoverage::Partial {
            missing: "Decorators (Angular @Component, NestJS @Controller); \
                          dynamic dispatch (this.method(), obj[key]()); \
                          JSX higher-order components and render props",
        },

        // Python: decorators are the dominant framework-dispatch
        // pattern (@app.route, @pytest.fixture, @property, @celery.task,
        // every Django/Flask/FastAPI route). Plumbed in
        // graphengine-parsing but not yet populated end-to-end into
        // dead-code classifiers.
        Ecosystem::Python => FrameworkExtractionCoverage::Partial {
            missing: "Decorators (@app.route, @pytest.fixture, @property, \
                      @celery.task); Django/Flask/FastAPI route handlers; \
                      dynamic dispatch via getattr()",
        },

        // Java: Spring is the default DI container in most modern Java
        // codebases; @Bean / @Service / @Component / @Autowired wiring
        // is invisible to the static call graph. JAX-RS, JPA, JUnit
        // annotations have the same shape.
        Ecosystem::Java => FrameworkExtractionCoverage::Partial {
            missing: "Spring annotations (@Bean, @Service, @Component, \
                      @Autowired, @Controller); JAX-RS endpoint annotations; \
                      JPA/JUnit reflection dispatch",
        },

        // Go: stdlib interface impls (Stringer, Error, http.Handler)
        // are detected by name. Custom interfaces with unique method
        // names will appear to have no callers even when implementations
        // are dispatched through them at runtime.
        Ecosystem::Go => FrameworkExtractionCoverage::Partial {
            missing: "Custom interface implementations (only common stdlib \
                      interfaces are detected by name); reflect-based \
                      dispatch",
        },

        // C#: ASP.NET MVC attribute routing, Unity custom UnityEvents,
        // reflection-invoked methods. Unity lifecycle methods (Start,
        // Update, Awake) are detected by name.
        Ecosystem::CSharp => FrameworkExtractionCoverage::Partial {
            missing: "ASP.NET attributes ([HttpGet], [ApiController]); \
                      Unity custom UnityEvent callbacks; reflection-invoked \
                      methods",
        },

        // No language signal at all → we cannot bound the risk. Treat
        // as worst case for the confidence ceiling, but be honest that
        // it's an unknown-unknown, not a known-unknown.
        Ecosystem::Unknown => FrameworkExtractionCoverage::Unknown,
    }
}

/// Compute the `dead_code` metric confidence as the more-conservative
/// of (resolution-tier-implied level, extraction-coverage-implied
/// level). The reason string is composed to name whichever axis lowered
/// the level, so consumers can quote it directly.
///
/// Returns a [`MetricConfidence`] suitable for insertion into the
/// per-metric confidence map.
pub fn compute(eco: Ecosystem, tier: ResolutionTier) -> MetricConfidence {
    let coverage = framework_extraction_coverage(eco);

    // Cap from resolution tier.
    let tier_level = match tier {
        ResolutionTier::Full => Confidence::High,
        ResolutionTier::HeuristicOnly => Confidence::Medium,
        ResolutionTier::None => Confidence::Low,
    };

    // Cap from extraction coverage. `Full` is the only case that
    // doesn't pull the level down; everything else caps at Medium
    // (Partial: we know what's missing) or Low (Unknown: we don't
    // even know what we don't know).
    let coverage_level = match &coverage {
        FrameworkExtractionCoverage::Full { .. } => Confidence::High,
        FrameworkExtractionCoverage::Partial { .. } => Confidence::Medium,
        FrameworkExtractionCoverage::Unknown => Confidence::Low,
    };

    let level = min_confidence(tier_level, coverage_level);

    let reason = build_reason(tier, &coverage, level);

    MetricConfidence { level, reason }
}

/// Per-bucket honesty caveats for `DeadCodeMetricDetail.reason_breakdown`.
///
/// Some reason buckets (`framework_annotation_unresolved`,
/// `dynamic_dispatch_target`, `callback_target_not_tracked`,
/// `declarative_wiring_unparsed`) can only be populated when the
/// parser extracts language-specific upstream signals
/// (`entry_point_tags`, `is_attribute_invoked`,
/// `is_callback_target`). When those signals aren't extracted for
/// the dominant ecosystem, those buckets always read 0 — not because
/// no nodes fit the category, but because the parser literally cannot
/// see them. Returning the caveat map lets consumers surface that
/// honestly instead of treating a 0 bucket as "no hits found."
///
/// Returns `None` when the ecosystem has full extraction coverage
/// (no buckets are unreliable) or is `Unknown` (we can't enumerate
/// gaps for a language we don't recognise; the wider `dead_code`
/// `metric_confidence.level` already reflects that uncertainty).
///
/// Returns a populated map otherwise, keyed by the snake_case reason
/// identifier from [`super::report::DeadCodeReason::as_str`].
pub fn reason_breakdown_caveats(eco: Ecosystem) -> Option<BTreeMap<String, String>> {
    match framework_extraction_coverage(eco) {
        FrameworkExtractionCoverage::Full { .. } => None,
        FrameworkExtractionCoverage::Unknown => None,
        FrameworkExtractionCoverage::Partial { .. } => {
            let mut caveats = BTreeMap::new();
            let lang_specifics = per_language_bucket_caveats(eco);

            // Buckets that depend on parser-extracted framework signals.
            // For any language without Full coverage, these are
            // structurally unable to be populated.
            caveats.insert(
                "framework_annotation_unresolved".into(),
                format!(
                    "May read 0 not because no nodes fit, but because the parser does not \
                     extract framework-attribute signals for this language. {}",
                    lang_specifics.framework_annotation
                ),
            );
            caveats.insert(
                "dynamic_dispatch_target".into(),
                format!(
                    "May read 0 because dynamic-dispatch detection is not implemented for \
                     this language yet. {}",
                    lang_specifics.dynamic_dispatch
                ),
            );
            caveats.insert(
                "callback_target_not_tracked".into(),
                format!(
                    "May read 0 because callback-target detection is not implemented for \
                     this language yet. {}",
                    lang_specifics.callback_target
                ),
            );
            caveats.insert(
                "declarative_wiring_unparsed".into(),
                format!(
                    "May read 0 because declarative-wiring detection is language-specific. {}",
                    lang_specifics.declarative_wiring
                ),
            );

            Some(caveats)
        }
    }
}

struct BucketCaveatText {
    framework_annotation: &'static str,
    dynamic_dispatch: &'static str,
    callback_target: &'static str,
    declarative_wiring: &'static str,
}

fn per_language_bucket_caveats(eco: Ecosystem) -> BucketCaveatText {
    match eco {
        Ecosystem::Rust => BucketCaveatText {
            framework_annotation: "Rust attribute macros (#[tool], #[mcp], \
                                   #[axum::handler], etc.) are not traversed.",
            dynamic_dispatch: "dyn Trait method dispatch is not yet detected.",
            callback_target: "Function-pointer / closure passing is not yet \
                              traced through call sites.",
            declarative_wiring: "Rust has no canonical declarative wiring \
                                 (build.rs, Cargo features, env vars) covered \
                                 by the analyzer.",
        },
        Ecosystem::TypeScript | Ecosystem::JavaScript => BucketCaveatText {
            framework_annotation: "Angular @Component / NestJS @Controller / Express \
                                   route decorators are not traversed.",
            dynamic_dispatch: "this.method() / obj[key]() dynamic dispatch is not \
                               statically resolvable and not yet flagged here.",
            callback_target: "Functions passed to .map / .filter / event listeners \
                              are not yet marked as callback targets.",
            declarative_wiring: "JSON / YAML route registries are not parsed.",
        },
        Ecosystem::Python => BucketCaveatText {
            framework_annotation: "Decorators (@app.route, @pytest.fixture, @property, \
                                   @celery.task, etc.) are plumbed but not yet \
                                   populated.",
            dynamic_dispatch: "getattr() / __getattr__ dynamic dispatch is not yet \
                               detected.",
            callback_target: "Functions passed as arguments (map, signal handlers) \
                              are not yet marked.",
            declarative_wiring: "Django urls.py string references / Flask blueprint \
                                 wiring are not yet parsed.",
        },
        Ecosystem::Java => BucketCaveatText {
            framework_annotation: "Spring @Bean / @Service / @Component / @Autowired / \
                                   JAX-RS annotations are not traversed.",
            dynamic_dispatch: "Reflection-based dispatch (Class.forName + invoke) is \
                               not detected.",
            callback_target: "Functional-interface arguments (Runnable, Consumer) are \
                              not yet marked.",
            declarative_wiring: "Spring XML / JPA configuration is not parsed.",
        },
        Ecosystem::Go => BucketCaveatText {
            framework_annotation: "Custom interface implementations beyond stdlib \
                                   shapes are name-matched only.",
            dynamic_dispatch: "reflect-based dispatch and interface{} unboxing are not \
                               detected.",
            callback_target: "Function values passed via channels or stored in maps \
                              are not yet marked.",
            declarative_wiring: "go:generate directives and code generation outputs are \
                                 not yet resolved.",
        },
        Ecosystem::CSharp => BucketCaveatText {
            framework_annotation: "ASP.NET [HttpGet] / [ApiController] / Unity custom \
                                   UnityEvents are not traversed.",
            dynamic_dispatch: "Reflection-based dispatch is not detected.",
            callback_target: "Delegate / event subscription is not yet marked as a \
                              callback target.",
            declarative_wiring: "Web.config / appsettings.json route bindings are not \
                                 parsed.",
        },
        Ecosystem::Apex | Ecosystem::Unknown => BucketCaveatText {
            framework_annotation: "",
            dynamic_dispatch: "",
            callback_target: "",
            declarative_wiring: "",
        },
    }
}

/// Confidence "min" — returns the more conservative of two levels.
/// `Confidence` derives `Ord` with declaration order `High < Medium <
/// Low`, so we use explicit ranks here to avoid relying on derive
/// order semantics that would flip if someone reorders the variants.
fn min_confidence(a: Confidence, b: Confidence) -> Confidence {
    fn rank(c: Confidence) -> u8 {
        match c {
            Confidence::High => 2,
            Confidence::Medium => 1,
            Confidence::Low => 0,
        }
    }
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
}

fn build_reason(
    tier: ResolutionTier,
    coverage: &FrameworkExtractionCoverage,
    final_level: Confidence,
) -> String {
    match (tier, coverage, final_level) {
        (ResolutionTier::None, _, _) => {
            "No import edges — dead-code detection is unreliable. Install the \
             appropriate LSP server for cross-file resolution before trusting \
             these verdicts."
                .into()
        }
        (ResolutionTier::HeuristicOnly, _, _) => {
            "Heuristic-only resolution — some call edges may be missing. \
             Install the language server for higher-accuracy resolution."
                .into()
        }
        (
            ResolutionTier::Full,
            FrameworkExtractionCoverage::Full { description },
            Confidence::High,
        ) => format!(
            "LSP-based resolution + complete framework-attribute extraction: {description}. \
             Both call-graph resolution and framework-dispatch coverage are at full fidelity."
        ),
        (ResolutionTier::Full, FrameworkExtractionCoverage::Partial { missing }, _) => {
            format!(
                "LSP-based resolution succeeded, but framework-attribute extraction is \
                 incomplete for this language. Not traversed: {missing}. Dead candidates \
                 may still be invoked via framework dispatch — verify before removing."
            )
        }
        (ResolutionTier::Full, FrameworkExtractionCoverage::Unknown, _) => {
            "LSP-based resolution succeeded, but the language is not recognised — \
             framework-extraction coverage is unknown. Treat verdicts as candidates, not \
             facts."
                .into()
        }
        // Other combinations fall through to the tier-only message
        // (we already returned above for tier != Full, so this is a
        // safety net for any future ResolutionTier additions).
        _ => "Dead-code metric confidence depends on cross-file resolution quality and \
              framework-attribute extraction coverage."
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_lsp_full_coverage_is_high() {
        let c = compute(Ecosystem::Apex, ResolutionTier::Full);
        assert_eq!(c.level, Confidence::High);
        assert!(c.reason.contains("complete framework-attribute extraction"));
        assert!(c.reason.contains("@AuraEnabled"));
    }

    #[test]
    fn full_lsp_partial_coverage_caps_at_medium() {
        // This is the B1 regression: Rust + LSP used to claim High.
        // It must now cap at Medium and tell the agent WHY.
        let c = compute(Ecosystem::Rust, ResolutionTier::Full);
        assert_eq!(
            c.level,
            Confidence::Medium,
            "Rust + LSP must cap at Medium until attribute-macro extraction lands"
        );
        assert!(
            c.reason.contains("attribute macros") || c.reason.contains("#[tool]"),
            "reason must name the specific Rust gap, not a generic message; got: {}",
            c.reason
        );
    }

    #[test]
    fn full_lsp_partial_coverage_caps_at_medium_for_python() {
        let c = compute(Ecosystem::Python, ResolutionTier::Full);
        assert_eq!(c.level, Confidence::Medium);
        assert!(
            c.reason.contains("Decorators") || c.reason.contains("@app.route"),
            "reason must name Python decorator gap; got: {}",
            c.reason
        );
    }

    #[test]
    fn full_lsp_partial_coverage_caps_at_medium_for_java() {
        let c = compute(Ecosystem::Java, ResolutionTier::Full);
        assert_eq!(c.level, Confidence::Medium);
        assert!(
            c.reason.contains("Spring") || c.reason.contains("@Component"),
            "reason must name Java Spring gap; got: {}",
            c.reason
        );
    }

    #[test]
    fn unknown_language_caps_at_low() {
        let c = compute(Ecosystem::Unknown, ResolutionTier::Full);
        assert_eq!(
            c.level,
            Confidence::Low,
            "unknown language is worse than partial: we can't even bound the risk"
        );
    }

    #[test]
    fn no_lsp_overrides_full_coverage() {
        // Even Apex + no imports cannot claim High — without resolution
        // the call graph is sparse, and framework-extraction can't
        // compensate.
        let c = compute(Ecosystem::Apex, ResolutionTier::None);
        assert_eq!(c.level, Confidence::Low);
        assert!(c.reason.contains("No import edges"));
    }

    #[test]
    fn heuristic_only_caps_at_medium_regardless_of_coverage() {
        let c = compute(Ecosystem::Apex, ResolutionTier::HeuristicOnly);
        assert_eq!(c.level, Confidence::Medium);
        assert!(c.reason.contains("Heuristic-only"));
    }

    #[test]
    fn min_confidence_picks_the_more_conservative_level() {
        assert_eq!(
            min_confidence(Confidence::High, Confidence::Medium),
            Confidence::Medium
        );
        assert_eq!(
            min_confidence(Confidence::Medium, Confidence::Low),
            Confidence::Low
        );
        assert_eq!(
            min_confidence(Confidence::High, Confidence::High),
            Confidence::High
        );
        assert_eq!(
            min_confidence(Confidence::Low, Confidence::High),
            Confidence::Low
        );
    }

    #[test]
    fn reason_breakdown_caveats_present_for_rust() {
        let caveats = reason_breakdown_caveats(Ecosystem::Rust)
            .expect("Rust has partial coverage; caveats must be present");
        assert!(
            caveats.contains_key("framework_annotation_unresolved"),
            "framework_annotation_unresolved caveat missing"
        );
        assert!(
            caveats.contains_key("dynamic_dispatch_target"),
            "dynamic_dispatch_target caveat missing"
        );
        assert!(
            caveats.contains_key("callback_target_not_tracked"),
            "callback_target_not_tracked caveat missing"
        );
        assert!(
            caveats.contains_key("declarative_wiring_unparsed"),
            "declarative_wiring_unparsed caveat missing"
        );
        let fau = caveats.get("framework_annotation_unresolved").unwrap();
        assert!(
            fau.contains("attribute macros") || fau.contains("#[tool]"),
            "framework_annotation_unresolved caveat must name the Rust-specific gap; got: {fau}"
        );
    }

    #[test]
    fn reason_breakdown_caveats_absent_for_apex() {
        // Apex has Full coverage; no buckets need a caveat. The
        // returned Option must be None so the caveat block isn't
        // serialised for Apex reports.
        assert!(
            reason_breakdown_caveats(Ecosystem::Apex).is_none(),
            "Apex has full coverage; caveats map should be None"
        );
    }

    #[test]
    fn reason_breakdown_caveats_absent_for_unknown() {
        // Unknown languages: the wider metric_confidence already
        // reflects the uncertainty; per-bucket caveats would be made
        // up. Better to omit and let consumers fall back to the
        // confidence level alone.
        assert!(
            reason_breakdown_caveats(Ecosystem::Unknown).is_none(),
            "Unknown ecosystem: per-bucket caveats are not enumerable"
        );
    }

    #[test]
    fn framework_extraction_coverage_is_total_over_ecosystem() {
        // Every ecosystem variant must return a coverage classification —
        // no panicking, no default-fallthrough. Useful guard when a new
        // language is added to config::Ecosystem.
        for eco in [
            Ecosystem::TypeScript,
            Ecosystem::JavaScript,
            Ecosystem::Rust,
            Ecosystem::Java,
            Ecosystem::Go,
            Ecosystem::Python,
            Ecosystem::CSharp,
            Ecosystem::Apex,
            Ecosystem::Unknown,
        ] {
            let _ = framework_extraction_coverage(eco);
        }
    }
}
