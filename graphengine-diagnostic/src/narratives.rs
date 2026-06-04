//! Risk-narrative + suggested-action templates (deterministic, no LLM).
//!
//! Implements `docs/02-strategy/DIAGNOSTIC_PRODUCT_SPEC.md` §2. Every `FindingType` maps
//! to three strings:
//! - a **risk narrative** — business-language explanation of why it matters
//! - a **suggested action** — first concrete step to take
//! - an **impact category** — which business pain it relates to
//!
//! Template interpolation uses the fields that `graphengine-analysis` already
//! populates on the `Finding` and `NodeAnnotation` structs.

use std::collections::BTreeMap;

use graphengine_analysis::health::report::{Finding, FindingType, NodeAnnotation};

/// Resolve a raw `node_id` to its human-visible display name.
///
/// Many engines store `node_id` as an opaque content hash (e.g. SHA-256 of
/// `path::function`). Interpolating those directly into user-facing narratives
/// produces gibberish like *"efd8156e… is called by 5 other functions."* The
/// canonical human label lives on `NodeAnnotation.display_name` with
/// `NodeAnnotation.fqn` as a second-best fallback. This helper centralizes
/// that lookup so no caller has to remember the rule.
pub fn display_name_for(raw_id: &str, annotations: &BTreeMap<String, NodeAnnotation>) -> String {
    if let Some(a) = annotations.get(raw_id) {
        if !a.display_name.is_empty() {
            return a.display_name.clone();
        }
        if !a.fqn.is_empty() {
            return a.fqn.clone();
        }
    }
    raw_id.to_string()
}

/// Resolve the target label used inside narrative + action text. Must match
/// [`crate::priority::finding_target`] so Fix-First entries and the narrative
/// text refer to the *same* thing by the *same* name.
///
/// Order: annotation-resolved id → raw id (for engines that use FQNs as ids,
/// e.g. modules) → metric_name → canonical fallback.
fn target_display(finding: &Finding, annotations: &BTreeMap<String, NodeAnnotation>) -> String {
    if let Some(id) = finding
        .primary_node_id
        .as_deref()
        .or_else(|| finding.node_ids.first().map(|s| s.as_str()))
    {
        let label = display_name_for(id, annotations);
        if !label.is_empty() {
            return label;
        }
    }
    if let Some(m) = finding.metric_name.clone() {
        return m;
    }
    "the affected target".to_string()
}

/// Return the rendered risk narrative for this finding.
///
/// If interpolation data is missing, falls back to a generic sentence that is
/// still safe to ship (never panics, never returns empty).
pub fn risk_narrative(finding: &Finding, annotations: &BTreeMap<String, NodeAnnotation>) -> String {
    let target = target_display(finding, annotations);

    match finding.finding_type {
        FindingType::CircularDependency => format!(
            "A dependency cycle involving {} nodes in {} makes isolated testing and \
             safe refactoring harder. Changes in any part of this cycle can propagate \
             unpredictably.",
            finding.cycle_length.unwrap_or(finding.node_ids.len()),
            module_list_from_node_ids(&finding.node_ids, annotations)
        ),
        FindingType::BlastRadiusHotspot => format!(
            "{} is called by {} other functions and affects {} downstream nodes. A bug or \
             breaking change here has outsized impact across the codebase.",
            target,
            finding.fan_in.unwrap_or(0),
            finding.blast_radius.unwrap_or(0)
        ),
        FindingType::HighCoupling => format!(
            "Module {} depends heavily on external modules ({:.0}% external edges). This \
             makes it fragile to changes elsewhere and hard to understand in isolation.",
            target,
            finding.coupling_score.unwrap_or(0.0) * 100.0
        ),
        FindingType::PotentiallyUnreachable => format!(
            "{} functions have no incoming call edges and may be dead code. Dead code \
             increases cognitive load during onboarding and review without providing value.",
            finding.count.unwrap_or(finding.node_ids.len())
        ),
        FindingType::DeepCallChain => format!(
            "The deepest call chain is {} levels. Deep chains are harder to debug, test, and \
             reason about. Errors at the bottom surface as confusing failures at the top.",
            finding.metric_value.unwrap_or(0.0) as usize
        ),
        FindingType::GodFunction => {
            // For annotation lookup we still need the *raw* id (the key), not
            // the display name. Do the lookup separately.
            let raw_id = finding
                .primary_node_id
                .as_deref()
                .or_else(|| finding.node_ids.first().map(|s| s.as_str()))
                .unwrap_or("");
            let ann = annotations.get(raw_id);
            format!(
                "{} exceeds thresholds for complexity ({}), fan-out ({}), and size ({} LOC) \
                 simultaneously. Functions like this are the hardest to modify safely and the \
                 most likely to hide bugs.",
                target,
                ann.and_then(|a| a.cyclomatic_complexity).unwrap_or(0),
                ann.map(|a| a.fan_out).unwrap_or(0),
                ann.map(|a| a.loc).unwrap_or(0)
            )
        }
        FindingType::TemporalCoupling => format!(
            "{} and {} change together in {} commits but have no direct dependency. This \
             hidden coupling suggests a shared implicit contract that could break silently.",
            finding.file_a.clone().unwrap_or_else(|| "file A".into()),
            finding.file_b.clone().unwrap_or_else(|| "file B".into()),
            finding.co_change_count.unwrap_or(0)
        ),
        FindingType::LowCohesion => format!(
            "Module {} contains {} disconnected groups of functions. Low cohesion suggests \
             the module mixes unrelated responsibilities and should likely be split.",
            target,
            finding.count.unwrap_or(0)
        ),
        FindingType::LayerViolation => format!(
            "{} bypasses intermediate abstraction layers, calling directly into deep \
             infrastructure. This creates hidden dependencies that make the system harder to \
             evolve.",
            target
        ),
        FindingType::ZoneOfPain => format!(
            "Module {} is concrete (low abstractness) and heavily depended upon. Changes \
             here are risky because many consumers depend on concrete implementation details.",
            target
        ),
        FindingType::ZoneOfUselessness => format!(
            "Module {} is highly abstract but has few dependents. It may represent \
             over-engineering — abstractions that add complexity without being used.",
            target
        ),
        FindingType::InformationFlowBottleneck => format!(
            "{} has high information flow complexity (fan-in × fan-out = {}). It acts as a \
             data chokepoint — many inputs, many outputs — making it hard to reason about data \
             transformations.",
            target,
            finding.metric_value.unwrap_or(0.0) as usize
        ),
        FindingType::HubNode => format!(
            "{} bridges {} distinct modules. Cross-cutting hubs create coordination overhead \
             between teams and make independent deployability harder.",
            target,
            finding.count.unwrap_or(0)
        ),
        FindingType::LowEncapsulation => format!(
            "Module {} exposes {:.0}% of its functions externally. High exposure means \
             internal changes are more likely to break consumers.",
            target,
            finding.coupling_score.unwrap_or(0.0) * 100.0
        ),
        FindingType::ExcessiveComplexity => format!(
            "{} has cyclomatic complexity of {}, meaning {} independent execution paths. \
             Functions above 10 are significantly harder to test exhaustively.",
            target,
            finding.metric_value.unwrap_or(0.0) as usize,
            finding.metric_value.unwrap_or(0.0) as usize
        ),
        FindingType::BoundaryViolation => format!(
            "{} crosses an architectural boundary the project declared intentional. Repeated \
             violations of this kind erode the enforceability of the architecture.",
            target
        ),
        FindingType::EntryPoint => format!(
            "{} is an inferred entry point with unusually high outbound complexity. Entry \
             points need tight inbound validation because errors here propagate everywhere.",
            target
        ),
        FindingType::ResolutionDegraded => format!(
            "Cross-file call resolution used the heuristic fallback for {:.0}% of edges. \
             Cross-file findings below are computed over a partially-resolved call graph; \
             treat them as directional rather than exact.",
            finding.metric_value.unwrap_or(0.0) * 100.0
        ),
    }
}

/// Return the rendered suggested-action template for this finding.
pub fn suggested_action(
    finding: &Finding,
    annotations: &BTreeMap<String, NodeAnnotation>,
) -> String {
    let target = target_display(finding, annotations);

    match finding.finding_type {
        FindingType::CircularDependency => {
            "Break the cycle by extracting shared types into a common module or inverting one \
             dependency direction. Start with the edge that has the lowest fan-in."
                .to_string()
        }
        FindingType::BlastRadiusHotspot => format!(
            "Add comprehensive tests around {}. Consider extracting an interface to limit \
             coupling surface. Avoid changing this function without targeted regression tests.",
            target
        ),
        FindingType::HighCoupling => {
            "Identify which external dependencies are essential vs incidental. Move incidental \
             imports behind a facade or into a shared abstraction layer."
                .to_string()
        }
        FindingType::PotentiallyUnreachable => {
            "Verify these functions are truly unused. If confirmed dead, remove them to reduce \
             cognitive load and maintenance surface."
                .to_string()
        }
        FindingType::DeepCallChain => {
            "Flatten the chain by extracting mid-level facades, or invert control so the deep \
             layers return values instead of invoking the next level directly."
                .to_string()
        }
        FindingType::GodFunction => format!(
            "Split {} into smaller functions with single responsibilities. Extract the \
             distinct logic branches into separate well-named helpers.",
            target
        ),
        FindingType::TemporalCoupling => format!(
            "Investigate why {} and {} change together. If they share implicit state, make \
             the dependency explicit. If they belong together, colocate them.",
            finding
                .file_a
                .clone()
                .unwrap_or_else(|| "the two files".into()),
            finding.file_b.clone().unwrap_or_default()
        ),
        FindingType::LowCohesion => format!(
            "Consider splitting {} into {} focused modules, one per responsibility cluster.",
            target,
            finding.count.unwrap_or(2)
        ),
        FindingType::LayerViolation => {
            "Route the call through the appropriate intermediate layer. If no suitable \
             abstraction exists, create one."
                .to_string()
        }
        FindingType::ZoneOfPain => {
            "Introduce a stable abstraction in front of this module so consumers depend on an \
             interface, not on concrete details. Move risky changes behind that seam."
                .to_string()
        }
        FindingType::ZoneOfUselessness => {
            "Inline or delete abstractions that have zero or near-zero consumers. Unused \
             abstractions are pure maintenance cost."
                .to_string()
        }
        FindingType::InformationFlowBottleneck => format!(
            "Split {} into pipeline stages with narrower contracts. Bottlenecks usually \
             collapse once each input/output pair has a dedicated owner.",
            target
        ),
        FindingType::HubNode => format!(
            "Treat {} as a public API. Add a contract test, version it, and freeze its surface \
             while you evolve the modules behind it.",
            target
        ),
        FindingType::LowEncapsulation => {
            "Mark internal helpers non-public. Expose only the functions that external \
             consumers genuinely need."
                .to_string()
        }
        FindingType::ExcessiveComplexity => format!(
            "Decompose {} along its decision branches. Target cyclomatic complexity ≤ 10 per \
             function as a reviewable ceiling.",
            target
        ),
        FindingType::BoundaryViolation => {
            "Add the violation to the project's boundary configuration as an intentional \
             exception, or refactor the call to respect the boundary."
                .to_string()
        }
        FindingType::EntryPoint => format!(
            "Put input validation and request-scoped context at {}. Treat it as the last line \
             of defense before unvalidated data enters the rest of the system.",
            target
        ),
        FindingType::ResolutionDegraded => {
            "Ensure the language-server / LSP prerequisite is installed and accessible. Rerun \
             the scan with full resolution to sharpen cross-file findings."
                .to_string()
        }
    }
}

/// Stable impact-category string (appears on the risk card as a tag).
pub fn impact_category(finding_type: FindingType) -> &'static str {
    match finding_type {
        FindingType::CircularDependency => "Regression risk, refactoring cost",
        FindingType::BlastRadiusHotspot => "Regression risk, change safety",
        FindingType::HighCoupling => "Onboarding difficulty, change cost",
        FindingType::PotentiallyUnreachable => "Onboarding drag, maintenance cost",
        FindingType::DeepCallChain => "Debugging cost, test difficulty",
        FindingType::GodFunction => "Change safety, review cost",
        FindingType::TemporalCoupling => "Hidden regression risk",
        FindingType::LowCohesion => "Maintainability, onboarding",
        FindingType::LayerViolation => "Architecture decay, refactoring cost",
        FindingType::ZoneOfPain => "Change fragility",
        FindingType::ZoneOfUselessness => "Maintenance waste",
        FindingType::InformationFlowBottleneck => "Debugging cost",
        FindingType::HubNode => "Team coordination cost",
        FindingType::LowEncapsulation => "API stability risk",
        FindingType::ExcessiveComplexity => "Test coverage gaps, bug risk",
        FindingType::BoundaryViolation => "Architecture decay",
        FindingType::EntryPoint => "Input-validation risk",
        FindingType::ResolutionDegraded => "Analysis confidence",
    }
}

/// Best-effort humanization of the "which modules are touched" sentence.
///
/// Resolves each raw node id through `annotations` first (so hash-style ids
/// become `crate::module::function`-style FQNs), then collapses to unique
/// first-segment module names, deduped and sorted.
fn module_list_from_node_ids(
    ids: &[String],
    annotations: &BTreeMap<String, NodeAnnotation>,
) -> String {
    let mut modules: Vec<String> = ids
        .iter()
        .map(|n| display_name_for(n, annotations))
        .filter_map(|n| n.split("::").next().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    modules.sort();
    modules.dedup();
    if modules.is_empty() {
        "multiple modules".to_string()
    } else if modules.len() == 1 {
        modules.into_iter().next().unwrap()
    } else {
        modules.join(", ")
    }
}
