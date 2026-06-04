//! **MultipleTriggersPerSObject** structural finding.
//!
//! Salesforce best practice is "one trigger per SObject" — a widely
//! taught rule from Trailhead, Apex Developer Guide, and every major
//! Apex style guide. The stated rationale:
//!
//! - **Execution order is undefined** across multiple triggers on the
//!   same SObject. If two triggers both fire on `before insert`,
//!   Salesforce picks the order arbitrarily. Logic in one can silently
//!   clobber the other.
//! - **Recursion control** is painfully difficult when multiple
//!   triggers each try to guard against re-entry — each must either
//!   know about all the others or rely on a shared static flag, which
//!   is fragile.
//! - **Testing** becomes combinatorial; each trigger gets exercised on
//!   every DML op against the SObject.
//!
//! So this finding fires when a repository contains ≥ 2 Apex triggers
//! declared `on <SObject>`. It is a *structural* finding — it doesn't
//! care whether the triggers conflict today, only that the pattern is
//! in place.
//!
//! # False-positive suppression
//!
//! Several mature repos legitimately have multiple physical `.trigger`
//! files per SObject because they route everything through a handler
//! framework (see
//! [`graphengine_parsing::syntax::language::apex::trigger_framework`]).
//! When the parser detects such a framework, the finding caller passes
//! that detection into [`evaluate`] and the severity is downgraded
//! from Warning → Info (and the recommendation text is rewritten to
//! reflect "framework-aware" rather than "bad practice").
//!
//! # Input model
//!
//! The analysis graph (`graphengine-analysis`) does not yet have
//! dedicated `Trigger` / `SObject` node kinds — triggers today land as
//! `File` nodes with a `.trigger` extension. Rather than couple this
//! module to an evolving SQL schema, we take a pre-computed
//! [`TriggerInventory`] built by the orchestrator. This keeps the
//! finding testable in isolation and makes the schema-extraction logic
//! a separate concern.

use std::collections::BTreeMap;

use crate::health::report::{Finding, FindingType, Severity};

/// One trigger physically declared in the repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRef {
    /// Stable node id in the graph (ends up in `Finding::node_ids`).
    pub node_id: String,
    /// Display name (e.g. `AccountTrigger`).
    pub name: String,
    /// SObject the trigger is declared against (e.g. `Account`, `Contact`,
    /// `npsp__Allocation__c`). Case-preserved from the source.
    pub sobject_api_name: String,
    /// Optional source file path for debugging / UX.
    pub file_path: Option<String>,
}

/// Grouped map of `sobject_api_name → triggers-on-that-sobject`.
///
/// Key uses the case-preserved API name; lookups should be
/// case-insensitive because Apex is. Call [`TriggerInventory::group`]
/// to construct.
#[derive(Debug, Clone, Default)]
pub struct TriggerInventory {
    /// BTreeMap for deterministic iteration — finding ids and messages
    /// are stable across runs for the same repo.
    pub by_sobject: BTreeMap<String, Vec<TriggerRef>>,
}

impl TriggerInventory {
    /// Build an inventory by grouping triggers on a case-insensitive
    /// SObject key but **preserving** the first-seen display casing.
    pub fn group(triggers: Vec<TriggerRef>) -> Self {
        let mut by_sobject: BTreeMap<String, Vec<TriggerRef>> = BTreeMap::new();
        let mut canonical_case: BTreeMap<String, String> = BTreeMap::new();

        for t in triggers {
            let key = t.sobject_api_name.to_ascii_lowercase();
            let display_key = canonical_case
                .entry(key.clone())
                .or_insert_with(|| t.sobject_api_name.clone())
                .clone();
            by_sobject.entry(display_key).or_default().push(t);
        }

        // Stable sort within each bucket so finding descriptions are
        // deterministic across runs.
        for v in by_sobject.values_mut() {
            v.sort_by(|a, b| a.name.cmp(&b.name));
        }
        Self { by_sobject }
    }

    /// True if any SObject has 2+ triggers.
    pub fn has_violation(&self) -> bool {
        self.by_sobject.values().any(|v| v.len() >= 2)
    }
}

/// Whether the repository is using a known trigger-framework. Callers
/// compute this from
/// [`graphengine_parsing::syntax::language::apex::trigger_framework::detect`]
/// and forward the boolean — we intentionally don't re-import the
/// parsing crate here (analysis shouldn't depend on parsing internals).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameworkContext {
    pub uses_trigger_framework: bool,
    /// Optional short name (`"fflib"`, `"kevin-ohara"`, …) for the
    /// finding's recommendation text. `None` → "a trigger framework".
    pub framework_name: Option<&'static str>,
}

/// Evaluate the MultipleTriggersPerSObject check and return a finding
/// per SObject with ≥ 2 triggers. Empty input or single-trigger
/// inventories produce no findings.
///
/// The caller should `findings.extend(...)` this into the main finding
/// list. Finding ids are stable (`mtps-<sobject>`) so triage overrides
/// can suppress them.
pub fn evaluate(inventory: &TriggerInventory, ctx: FrameworkContext) -> Vec<Finding> {
    let mut out = Vec::new();
    for (sobject, triggers) in &inventory.by_sobject {
        if triggers.len() < 2 {
            continue;
        }
        out.push(build_finding(sobject, triggers, ctx));
    }
    out
}

fn build_finding(sobject: &str, triggers: &[TriggerRef], ctx: FrameworkContext) -> Finding {
    let count = triggers.len();
    let names: Vec<&str> = triggers.iter().map(|t| t.name.as_str()).collect();
    let id = format!("mtps-{}", sobject.to_ascii_lowercase());
    let node_ids: Vec<String> = triggers.iter().map(|t| t.node_id.clone()).collect();

    let (severity, description, recommendation) = if ctx.uses_trigger_framework {
        let framework_display = ctx.framework_name.unwrap_or("a trigger framework");
        (
            Severity::Info,
            format!(
                "{count} triggers declared on {sobject} ({}) — framework-routed, not a defect",
                names.join(", ")
            ),
            Some(format!(
                "Detected {framework_display}. Multiple triggers are acceptable when all delegate to the framework; \
                 verify each of these triggers uses the handler dispatch pattern and contains no imperative logic."
            )),
        )
    } else {
        let sev = if count >= 4 {
            Severity::High
        } else {
            Severity::Warning
        };
        (
            sev,
            format!(
                "{count} triggers declared on {sobject} ({}) — execution order between them is undefined",
                names.join(", ")
            ),
            Some(
                "Consolidate into a single trigger per SObject that delegates to a handler class. \
                 Salesforce does not guarantee execution order among sibling triggers, which makes \
                 recursion control, test isolation, and side-effect reasoning unreliable."
                    .to_string(),
            ),
        )
    };

    Finding {
        id,
        finding_type: FindingType::HighCoupling, // nearest existing; see note below
        severity,
        description,
        detail: Some(
            "Salesforce's widely-taught best practice is 'one trigger per SObject'. Multiple triggers \
             on the same SObject fire in an undefined order, making recursion guards, side-effect \
             ordering, and test behaviour hard to reason about. Frameworks like fflib, \
             sfdc-trigger-framework, and NPSP TDTM exist specifically to consolidate this into a \
             single trigger + many handlers."
                .to_string(),
        ),
        node_ids,
        edge_ids: None,
        primary_node_id: triggers.first().map(|t| t.node_id.clone()),
        metric_name: Some("triggers_per_sobject".into()),
        metric_value: Some(count as f64),
        impact: None,
        blast_radius: None,
        recommendation,
        cycle_length: None,
        fan_in: None,
        coupling_score: None,
        internal_edges: None,
        external_edges: None,
        count: Some(count),
        hub_score: None,
        file_a: None,
        file_b: None,
        co_change_count: None,
        temporal_coupling_score: None,
        has_import_edge: None,
        confidence: None,
    }
}

// NOTE: `FindingType::HighCoupling` is used here as the nearest existing
// variant. A dedicated `FindingType::MultipleTriggersPerSObject` would
// be more precise but adding variants to that enum requires touching
// the JSON schema contract consumed by the desktop UI. The description
// and metric_name are unique enough that dashboards can filter on them
// unambiguously. Upgrade to a dedicated variant is tracked in
// docs/workstreams/apex/INTEGRATION.md.

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tref(name: &str, sobj: &str) -> TriggerRef {
        TriggerRef {
            node_id: format!("file::triggers/{name}.trigger"),
            name: name.to_string(),
            sobject_api_name: sobj.to_string(),
            file_path: Some(format!("triggers/{name}.trigger")),
        }
    }

    #[test]
    fn single_trigger_per_sobject_emits_nothing() {
        let inv = TriggerInventory::group(vec![
            tref("AccountT", "Account"),
            tref("ContactT", "Contact"),
        ]);
        let findings = evaluate(&inv, FrameworkContext::default());
        assert!(findings.is_empty());
    }

    #[test]
    fn two_triggers_on_same_sobject_emits_warning() {
        let inv = TriggerInventory::group(vec![
            tref("AccountOne", "Account"),
            tref("AccountTwo", "Account"),
            tref("ContactT", "Contact"),
        ]);
        let findings = evaluate(&inv, FrameworkContext::default());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.count, Some(2));
        assert_eq!(f.id, "mtps-account");
        assert_eq!(f.node_ids.len(), 2);
        assert!(f.description.contains("AccountOne"));
        assert!(f.description.contains("AccountTwo"));
    }

    #[test]
    fn four_or_more_triggers_escalate_to_high() {
        let inv = TriggerInventory::group(vec![
            tref("A1", "Account"),
            tref("A2", "Account"),
            tref("A3", "Account"),
            tref("A4", "Account"),
        ]);
        let findings = evaluate(&inv, FrameworkContext::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn framework_detected_downgrades_to_info_and_rewrites_recommendation() {
        let inv = TriggerInventory::group(vec![
            tref("AccountOne", "Account"),
            tref("AccountTwo", "Account"),
        ]);
        let ctx = FrameworkContext {
            uses_trigger_framework: true,
            framework_name: Some("fflib"),
        };
        let findings = evaluate(&inv, ctx);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(
            f.severity,
            Severity::Info,
            "framework detected => info severity, not warning/high"
        );
        let rec = f.recommendation.as_deref().unwrap_or("");
        assert!(
            rec.contains("fflib"),
            "recommendation should mention the detected framework name: {rec}"
        );
        assert!(
            f.description.contains("framework-routed"),
            "description should make the framework context obvious to UI readers"
        );
    }

    #[test]
    fn case_insensitive_sobject_grouping_preserves_first_seen_casing() {
        let inv = TriggerInventory::group(vec![
            tref("T1", "Account"),
            tref("T2", "ACCOUNT"),
            tref("T3", "account"),
        ]);
        // All three triggers should be grouped under exactly one
        // bucket — Apex is case-insensitive, so `Account` == `ACCOUNT`.
        assert_eq!(
            inv.by_sobject.len(),
            1,
            "case-insensitive grouping must collapse to one bucket: {:?}",
            inv.by_sobject.keys().collect::<Vec<_>>()
        );
        let findings = evaluate(&inv, FrameworkContext::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].count, Some(3));
    }

    #[test]
    fn managed_package_sobjects_are_not_special_cased() {
        // An `npsp__Allocation__c` with two triggers is just as bad as
        // a standard SObject with two triggers. No suppression.
        let inv = TriggerInventory::group(vec![
            tref("A1", "npsp__Allocation__c"),
            tref("A2", "npsp__Allocation__c"),
        ]);
        let findings = evaluate(&inv, FrameworkContext::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "mtps-npsp__allocation__c");
    }

    #[test]
    fn multiple_sobjects_produce_separate_findings() {
        let inv = TriggerInventory::group(vec![
            tref("A1", "Account"),
            tref("A2", "Account"),
            tref("C1", "Contact"),
            tref("C2", "Contact"),
        ]);
        let findings = evaluate(&inv, FrameworkContext::default());
        assert_eq!(findings.len(), 2);
        let ids: Vec<_> = findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"mtps-account"));
        assert!(ids.contains(&"mtps-contact"));
    }

    #[test]
    fn finding_ids_are_stable_across_runs() {
        let a = TriggerInventory::group(vec![tref("x", "Account"), tref("y", "Account")]);
        let b = TriggerInventory::group(vec![tref("y", "Account"), tref("x", "Account")]);
        let fa = evaluate(&a, FrameworkContext::default());
        let fb = evaluate(&b, FrameworkContext::default());
        assert_eq!(fa.len(), 1);
        assert_eq!(fb.len(), 1);
        assert_eq!(fa[0].id, fb[0].id);
        assert_eq!(fa[0].description, fb[0].description);
    }

    #[test]
    fn inventory_has_violation_predicate() {
        let v = TriggerInventory::group(vec![tref("a", "Account"), tref("b", "Account")]);
        assert!(v.has_violation());
        let nv = TriggerInventory::group(vec![tref("a", "Account"), tref("b", "Contact")]);
        assert!(!nv.has_violation());
        let empty = TriggerInventory::default();
        assert!(!empty.has_violation());
    }

    #[test]
    fn empty_inventory_produces_no_findings() {
        let inv = TriggerInventory::default();
        assert!(evaluate(&inv, FrameworkContext::default()).is_empty());
    }
}
