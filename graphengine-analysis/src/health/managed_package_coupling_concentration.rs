//! **ManagedPackageCouplingConcentration** structural finding.
//!
//! When a Salesforce codebase couples heavily to a single managed
//! package namespace (e.g., `npsp`, `fflib`, `pse`), the customer is
//! taking on a structural liability they **cannot refactor away**:
//!
//! - Managed package code is **opaque** — the consumer org cannot read,
//!   debug, or modify the called classes. Behaviour changes only when
//!   the package is upgraded by its publisher.
//! - **Upgrade risk** scales with the surface area of references.
//!   Every reference site is a potential break point if the package
//!   publisher renames a method, deprecates a class, or changes a field
//!   API name across versions.
//! - **License risk** ties to it as well — managed packages are usually
//!   paid SaaS dependencies. Heavy coupling makes leaving the package
//!   prohibitively expensive.
//! - **Knowledge concentration** — the team's understanding of "how
//!   Salesforce works" becomes inseparable from "how this package
//!   works", increasing onboarding cost and bus-factor risk.
//!
//! This finding fires when a single managed namespace has either:
//!
//! - **High absolute consumer count** — a configurable number of
//!   distinct Apex files reference the namespace, OR
//! - **High concentration ratio** — the namespace accounts for an
//!   outsized fraction of the Apex codebase's external-coupling
//!   footprint.
//!
//! Either condition alone is sufficient. The recommendation always
//! steers the user toward a facade / anti-corruption layer that wraps
//! the managed package behind an internal interface, isolating the
//! blast radius of a future package change.
//!
//! # Input model
//!
//! Like [`super::multiple_triggers_per_sobject`], this module accepts a
//! pre-aggregated [`ManagedPackageInventory`] built by the orchestrator
//! from
//! [`graphengine_parsing::syntax::language::apex::managed_packages::extract`].
//! Keeping the analysis side decoupled from parsing internals makes it
//! testable in isolation and lets the orchestrator (which already owns
//! per-file aggregation for triggers and SObjects) own this aggregation
//! too.

use std::collections::BTreeMap;

use crate::health::report::{Finding, FindingType, Severity};

/// One Apex consumer file referencing one managed-package namespace.
///
/// Multiple references to the same namespace from the same file should
/// be collapsed before constructing this — see
/// [`graphengine_parsing::syntax::language::apex::managed_packages::unique_namespaces`]
/// for the per-file dedup helper. The inventory expects each
/// `(namespace, consumer_file)` pair to appear at most once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPackageRef {
    /// Stable graph node id of the consumer (typically the Apex class
    /// or trigger file). Lands in `Finding::node_ids`.
    pub consumer_node_id: String,
    /// Display name of the consumer (typically the class name) for
    /// finding messages.
    pub consumer_display: String,
    /// The managed namespace, lowercased for consistent grouping.
    pub namespace: String,
    /// Optional source file path for UI / debugging.
    pub file_path: Option<String>,
}

/// Pre-aggregated inventory: one bucket per managed namespace, listing
/// every consumer that references it.
///
/// Use [`ManagedPackageInventory::group`] to construct from a flat list
/// of [`ManagedPackageRef`]s. The grouping is case-insensitive on the
/// namespace key (Apex matches namespaces case-insensitively).
#[derive(Debug, Clone, Default)]
pub struct ManagedPackageInventory {
    /// `BTreeMap` keeps namespace order deterministic so finding ids and
    /// descriptions are stable across runs.
    pub by_namespace: BTreeMap<String, Vec<ManagedPackageRef>>,
}

impl ManagedPackageInventory {
    /// Build an inventory by grouping references on a case-insensitive
    /// namespace key. Within each bucket, consumers are deduplicated by
    /// `consumer_node_id` (so re-running aggregation never inflates the
    /// count) and sorted by display name for stable output.
    pub fn group(refs: Vec<ManagedPackageRef>) -> Self {
        let mut by_namespace: BTreeMap<String, Vec<ManagedPackageRef>> = BTreeMap::new();
        for r in refs {
            let key = r.namespace.to_ascii_lowercase();
            let bucket = by_namespace.entry(key).or_default();
            if !bucket
                .iter()
                .any(|existing| existing.consumer_node_id == r.consumer_node_id)
            {
                bucket.push(r);
            }
        }
        for v in by_namespace.values_mut() {
            v.sort_by(|a, b| a.consumer_display.cmp(&b.consumer_display));
        }
        Self { by_namespace }
    }

    /// Total number of unique `(namespace, consumer)` pairs.
    pub fn total_references(&self) -> usize {
        self.by_namespace.values().map(|v| v.len()).sum()
    }

    /// Number of distinct managed namespaces referenced.
    pub fn unique_namespaces(&self) -> usize {
        self.by_namespace.len()
    }
}

/// Configurable thresholds for when a namespace is "concentrated".
///
/// The defaults come from typical SFDX repo profiles (small to mid-size
/// orgs, 10–500 Apex classes). Tune via the analysis config when an
/// ecosystem profile demands different sensitivity.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Minimum number of distinct consumer files that reference one
    /// namespace before we surface a finding on absolute count alone.
    pub absolute_warning: usize,
    /// Above this absolute count, severity escalates to `High`.
    pub absolute_high: usize,
    /// Above this absolute count, severity escalates to `Critical`.
    pub absolute_critical: usize,
    /// Concentration ratio (consumers of namespace / total Apex
    /// consumer files in the repo) above which we surface a finding
    /// even when the absolute count is small. Catches cases where a
    /// tiny codebase is *entirely* coupled to one external package.
    pub ratio_warning: f64,
    /// Above this ratio, severity escalates to `High`.
    pub ratio_high: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            absolute_warning: 10,
            absolute_high: 25,
            absolute_critical: 60,
            ratio_warning: 0.30,
            ratio_high: 0.60,
        }
    }
}

/// Evaluate the inventory and return one finding per namespace that
/// crosses either threshold axis.
///
/// `total_apex_consumer_files` is the size of the population over which
/// the concentration ratio is computed — typically the count of
/// production Apex `.cls` + `.trigger` files in the repo. Pass `0`
/// when the population isn't computable; ratio-based findings are
/// suppressed in that case (only absolute-count findings fire).
pub fn evaluate(
    inventory: &ManagedPackageInventory,
    total_apex_consumer_files: usize,
    thresholds: Thresholds,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for (namespace, refs) in &inventory.by_namespace {
        let count = refs.len();
        let ratio = if total_apex_consumer_files > 0 {
            count as f64 / total_apex_consumer_files as f64
        } else {
            0.0
        };

        let absolute_hit = count >= thresholds.absolute_warning;
        let ratio_hit = total_apex_consumer_files > 0 && ratio >= thresholds.ratio_warning;
        if !absolute_hit && !ratio_hit {
            continue;
        }
        out.push(build_finding(
            namespace,
            refs,
            ratio,
            total_apex_consumer_files,
            thresholds,
        ));
    }
    out
}

fn build_finding(
    namespace: &str,
    refs: &[ManagedPackageRef],
    ratio: f64,
    total_apex_consumer_files: usize,
    thresholds: Thresholds,
) -> Finding {
    let count = refs.len();
    let severity = pick_severity(count, ratio, thresholds);
    let id = format!("mpcc-{}", namespace);
    let node_ids: Vec<String> = refs.iter().map(|r| r.consumer_node_id.clone()).collect();
    let primary_node_id = refs.first().map(|r| r.consumer_node_id.clone());

    let description = if total_apex_consumer_files > 0 {
        format!(
            "{count} Apex files couple to managed namespace '{namespace}' ({:.0}% of {} consumer files) — \
             external dependency the codebase cannot refactor",
            ratio * 100.0,
            total_apex_consumer_files,
        )
    } else {
        format!(
            "{count} Apex files couple to managed namespace '{namespace}' — \
             external dependency the codebase cannot refactor",
        )
    };

    let detail = Some(format!(
        "Managed-package code lives outside the customer org's source control. The org cannot read, debug, \
         or modify the namespace '{namespace}'. Concentration here means upgrade risk, license-lock-in risk, \
         and architectural risk concentrate on a single external publisher. Visualize this as a virtual \
         Module node bridging the codebase to the external dependency. The {count} consumer files are listed \
         in node_ids."
    ));

    Finding {
        id,
        finding_type: FindingType::HighCoupling,
        severity,
        description,
        detail,
        node_ids,
        edge_ids: None,
        primary_node_id,
        metric_name: Some("managed_package_consumer_count".into()),
        metric_value: Some(count as f64),
        impact: None,
        blast_radius: None,
        recommendation: Some(format!(
            "Wrap '{namespace}' calls behind an internal facade (anti-corruption layer). Reference the facade \
             from application code so a future package upgrade, replacement, or removal touches one file \
             instead of {count}. Pair this with an integration-style Apex test that pins the package's public \
             surface area so version drift fails loudly."
        )),
        cycle_length: None,
        fan_in: Some(count),
        coupling_score: if total_apex_consumer_files > 0 { Some(ratio) } else { None },
        internal_edges: None,
        external_edges: Some(count),
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

fn pick_severity(count: usize, ratio: f64, t: Thresholds) -> Severity {
    if count >= t.absolute_critical {
        return Severity::Critical;
    }
    if count >= t.absolute_high || ratio >= t.ratio_high {
        return Severity::High;
    }
    Severity::Warning
}

// NOTE: `FindingType::HighCoupling` is reused here for the same reason
// `multiple_triggers_per_sobject` reuses it — adding a dedicated
// `ManagedPackageCouplingConcentration` variant would fork the JSON
// schema consumed by the desktop UI. The unique `metric_name`
// (`managed_package_consumer_count`) and `id` prefix (`mpcc-`) let the
// dashboard filter unambiguously. Upgrade to a dedicated variant is
// tracked in docs/workstreams/apex/INTEGRATION.md.

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn r(consumer: &str, ns: &str) -> ManagedPackageRef {
        ManagedPackageRef {
            consumer_node_id: format!("file::classes/{consumer}.cls"),
            consumer_display: consumer.to_string(),
            namespace: ns.to_ascii_lowercase(),
            file_path: Some(format!("classes/{consumer}.cls")),
        }
    }

    #[test]
    fn empty_inventory_produces_no_findings() {
        let inv = ManagedPackageInventory::default();
        assert!(evaluate(&inv, 100, Thresholds::default()).is_empty());
    }

    #[test]
    fn count_below_threshold_emits_nothing_when_ratio_also_low() {
        let inv = ManagedPackageInventory::group(vec![r("A", "npsp"), r("B", "npsp")]);
        // 2 consumers / 100 total = 2% ratio. Both axes below default threshold.
        let findings = evaluate(&inv, 100, Thresholds::default());
        assert!(findings.is_empty());
    }

    #[test]
    fn absolute_count_at_threshold_fires_warning() {
        let mut refs = Vec::new();
        for i in 0..10 {
            refs.push(r(&format!("Class{i}"), "npsp"));
        }
        let inv = ManagedPackageInventory::group(refs);
        let findings = evaluate(&inv, 1000, Thresholds::default());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.id, "mpcc-npsp");
        assert_eq!(f.count, Some(10));
        assert_eq!(f.fan_in, Some(10));
    }

    #[test]
    fn high_consumer_count_escalates_to_high() {
        let mut refs = Vec::new();
        for i in 0..30 {
            refs.push(r(&format!("Class{i}"), "fflib"));
        }
        let inv = ManagedPackageInventory::group(refs);
        let findings = evaluate(&inv, 1000, Thresholds::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn very_high_consumer_count_escalates_to_critical() {
        let mut refs = Vec::new();
        for i in 0..70 {
            refs.push(r(&format!("Class{i}"), "npsp"));
        }
        let inv = ManagedPackageInventory::group(refs);
        let findings = evaluate(&inv, 1000, Thresholds::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn ratio_threshold_fires_even_with_small_absolute_count() {
        // 4 consumers out of 10 total = 40% ratio, even though absolute
        // count is below the default warning threshold of 10.
        let inv = ManagedPackageInventory::group(vec![
            r("A", "npsp"),
            r("B", "npsp"),
            r("C", "npsp"),
            r("D", "npsp"),
        ]);
        let findings = evaluate(&inv, 10, Thresholds::default());
        assert_eq!(findings.len(), 1, "ratio 40% > 30% threshold should fire");
        assert_eq!(findings[0].id, "mpcc-npsp");
    }

    #[test]
    fn ratio_high_threshold_escalates_severity() {
        // 7 consumers / 10 total = 70% ratio, which is above the
        // ratio_high threshold even though absolute count (7) is below
        // absolute_high (25).
        let mut refs = Vec::new();
        for i in 0..7 {
            refs.push(r(&format!("Class{i}"), "npsp"));
        }
        let inv = ManagedPackageInventory::group(refs);
        let findings = evaluate(&inv, 10, Thresholds::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].severity,
            Severity::High,
            "70% > ratio_high (60%)"
        );
    }

    #[test]
    fn zero_total_files_suppresses_ratio_findings() {
        let inv = ManagedPackageInventory::group(vec![r("A", "npsp"), r("B", "npsp")]);
        // Without a population denominator we can't compute ratio, and
        // absolute count is below threshold — so no finding.
        let findings = evaluate(&inv, 0, Thresholds::default());
        assert!(findings.is_empty());
    }

    #[test]
    fn zero_total_files_still_fires_on_absolute_count() {
        let mut refs = Vec::new();
        for i in 0..15 {
            refs.push(r(&format!("Class{i}"), "npsp"));
        }
        let inv = ManagedPackageInventory::group(refs);
        let findings = evaluate(&inv, 0, Thresholds::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].coupling_score, None,
            "no ratio without population"
        );
    }

    #[test]
    fn deduplicates_consumer_within_namespace() {
        // Same consumer reported twice for the same namespace must
        // collapse to one entry — the orchestrator may pre-dedupe but
        // the inventory must be defensive.
        let inv =
            ManagedPackageInventory::group(vec![r("A", "npsp"), r("A", "npsp"), r("B", "npsp")]);
        assert_eq!(inv.by_namespace.get("npsp").unwrap().len(), 2);
    }

    #[test]
    fn case_insensitive_namespace_grouping() {
        let inv = ManagedPackageInventory::group(vec![
            ManagedPackageRef {
                consumer_node_id: "x".into(),
                consumer_display: "X".into(),
                namespace: "NPSP".into(),
                file_path: None,
            },
            ManagedPackageRef {
                consumer_node_id: "y".into(),
                consumer_display: "Y".into(),
                namespace: "npsp".into(),
                file_path: None,
            },
        ]);
        assert_eq!(inv.by_namespace.len(), 1, "NPSP and npsp must collapse");
        assert_eq!(inv.by_namespace.get("npsp").unwrap().len(), 2);
    }

    #[test]
    fn separate_namespaces_produce_separate_findings() {
        let mut refs = Vec::new();
        for i in 0..12 {
            refs.push(r(&format!("N{i}"), "npsp"));
        }
        for i in 0..12 {
            refs.push(r(&format!("F{i}"), "fflib"));
        }
        let inv = ManagedPackageInventory::group(refs);
        let findings = evaluate(&inv, 100, Thresholds::default());
        assert_eq!(findings.len(), 2);
        let mut ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["mpcc-fflib", "mpcc-npsp"]);
    }

    #[test]
    fn finding_ids_are_stable_across_runs() {
        let inv_a = ManagedPackageInventory::group(vec![
            r("Y", "npsp"),
            r("X", "npsp"),
            r("Z", "npsp"),
            r("W", "npsp"),
            r("V", "npsp"),
            r("U", "npsp"),
            r("T", "npsp"),
            r("S", "npsp"),
            r("R", "npsp"),
            r("Q", "npsp"),
        ]);
        let inv_b = ManagedPackageInventory::group(vec![
            r("Q", "npsp"),
            r("R", "npsp"),
            r("S", "npsp"),
            r("T", "npsp"),
            r("U", "npsp"),
            r("V", "npsp"),
            r("W", "npsp"),
            r("X", "npsp"),
            r("Y", "npsp"),
            r("Z", "npsp"),
        ]);
        let fa = evaluate(&inv_a, 100, Thresholds::default());
        let fb = evaluate(&inv_b, 100, Thresholds::default());
        assert_eq!(fa.len(), 1);
        assert_eq!(fb.len(), 1);
        assert_eq!(fa[0].id, fb[0].id);
        assert_eq!(fa[0].description, fb[0].description);
        assert_eq!(fa[0].node_ids, fb[0].node_ids);
    }

    #[test]
    fn total_references_and_unique_namespaces_helpers() {
        let inv =
            ManagedPackageInventory::group(vec![r("A", "npsp"), r("B", "npsp"), r("C", "fflib")]);
        assert_eq!(inv.total_references(), 3);
        assert_eq!(inv.unique_namespaces(), 2);
    }
}
