//! Curated registry of well-known Salesforce ecosystem managed packages.
//!
//! The [managed-package synthesizer](super::managed_packages) produces one
//! virtual `Module` node per external namespace referenced from source. In
//! isolation, that node carries only the raw lowercased namespace (e.g.
//! `"npsp"`) — enough to link `Import` edges, not enough to tell a
//! downstream risk scorer or UI that `npsp` is actually the
//! Nonprofit Success Pack published by Salesforce.org, while a random
//! third-party utility package looks identical.
//!
//! This module maintains a small, curated table of the most structurally
//! significant Salesforce ecosystem namespaces and exposes a case-
//! insensitive [`lookup`] that [`synthesize_module_node`](
//! super::managed_packages::synthesize_module_node) uses to enrich
//! synthesized external nodes with a stable `display_name`, `vendor`,
//! and `category`.
//!
//! # Scope (deliberately narrow)
//!
//! - Entries are hand-curated against Salesforce AppExchange popularity
//!   and ecosystem significance. Coverage is NOT exhaustive — this is a
//!   "known well-known packages" filter, not a universal registry.
//! - Unknown namespaces are surfaced as `is_known_ecosystem_package = false`
//!   with the raw namespace preserved. Downstream consumers that only
//!   read `namespace` keep working with zero changes.
//! - No user-facing configuration surface today. If customer demand
//!   emerges to register their own managed packages, add a
//!   `managed_package_overrides.yaml` loader that merges over this static
//!   table while keeping [`lookup`]'s signature unchanged.

use serde::Serialize;

/// Publisher/owner category of a known managed package.
///
/// Distinguishes Salesforce.org nonprofit/education products from the
/// Salesforce commercial product line from third-party ISV packages, so
/// risk scoring and UI labels can differentiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Vendor {
    /// Salesforce.org — the nonprofit / education subsidiary that
    /// publishes NPSP, EDA, and related offerings.
    SalesforceOrg,
    /// Salesforce (commercial product line: CPQ, Billing, Industry
    /// Clouds, Marketing Cloud, etc.).
    Salesforce,
    /// Independent Software Vendor (AppExchange partner) — any package
    /// not published by Salesforce or Salesforce.org directly.
    ThirdParty,
}

impl Vendor {
    /// Stable snake_case string for graph-node property serialization.
    pub fn as_property_str(self) -> &'static str {
        match self {
            Vendor::SalesforceOrg => "salesforce_org",
            Vendor::Salesforce => "salesforce",
            Vendor::ThirdParty => "third_party",
        }
    }
}

/// Functional category of a known managed package.
///
/// Used for downstream aggregation (e.g. "count of imports of CPQ-family
/// packages") and UI badging. Categories intentionally coarse — finer
/// taxonomy belongs to Phase 2 (Tooling API enrichment), not Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Nonprofit,
    Education,
    Cpq,
    IndustryCloud,
    Marketing,
    Analytics,
    Utility,
    Other,
}

impl Category {
    pub fn as_property_str(self) -> &'static str {
        match self {
            Category::Nonprofit => "nonprofit",
            Category::Education => "education",
            Category::Cpq => "cpq",
            Category::IndustryCloud => "industry_cloud",
            Category::Marketing => "marketing",
            Category::Analytics => "analytics",
            Category::Utility => "utility",
            Category::Other => "other",
        }
    }
}

/// A single curated entry. `namespace` is always stored lowercase so
/// lookups can match case-insensitively without allocating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownPackage {
    pub namespace: &'static str,
    pub display_name: &'static str,
    pub vendor: Vendor,
    pub category: Category,
}

/// Curated list of well-known Salesforce ecosystem managed-package
/// namespaces. Ordering is not semantically meaningful — `lookup` does a
/// linear scan because the table is tiny.
///
/// Sources: Salesforce AppExchange listings, Salesforce.org product
/// pages, and Salesforce Help documentation for namespace prefixes.
/// Entries chosen for structural impact (large code surface, common
/// dependency target) rather than alphabetical completeness.
const KNOWN_PACKAGES: &[KnownPackage] = &[
    // ----- Salesforce.org (nonprofit / education) -----
    KnownPackage {
        namespace: "npsp",
        display_name: "Nonprofit Success Pack",
        vendor: Vendor::SalesforceOrg,
        category: Category::Nonprofit,
    },
    KnownPackage {
        namespace: "npe01",
        display_name: "NPSP Contacts & Organizations",
        vendor: Vendor::SalesforceOrg,
        category: Category::Nonprofit,
    },
    KnownPackage {
        namespace: "npe03",
        display_name: "NPSP Recurring Donations",
        vendor: Vendor::SalesforceOrg,
        category: Category::Nonprofit,
    },
    KnownPackage {
        namespace: "npe4",
        display_name: "NPSP Relationships",
        vendor: Vendor::SalesforceOrg,
        category: Category::Nonprofit,
    },
    KnownPackage {
        namespace: "npe5",
        display_name: "NPSP Affiliations",
        vendor: Vendor::SalesforceOrg,
        category: Category::Nonprofit,
    },
    KnownPackage {
        namespace: "npo02",
        display_name: "NPSP Households",
        vendor: Vendor::SalesforceOrg,
        category: Category::Nonprofit,
    },
    KnownPackage {
        namespace: "hed",
        display_name: "Education Data Architecture (EDA)",
        vendor: Vendor::SalesforceOrg,
        category: Category::Education,
    },
    // ----- Salesforce (commercial) -----
    KnownPackage {
        namespace: "sbqq",
        display_name: "Salesforce CPQ",
        vendor: Vendor::Salesforce,
        category: Category::Cpq,
    },
    KnownPackage {
        namespace: "blng",
        display_name: "Salesforce Billing",
        vendor: Vendor::Salesforce,
        category: Category::Cpq,
    },
    KnownPackage {
        namespace: "pi",
        display_name: "Marketing Cloud Account Engagement (Pardot)",
        vendor: Vendor::Salesforce,
        category: Category::Marketing,
    },
    KnownPackage {
        namespace: "et4ae5",
        display_name: "Marketing Cloud Connect",
        vendor: Vendor::Salesforce,
        category: Category::Marketing,
    },
    KnownPackage {
        namespace: "fsc",
        display_name: "Financial Services Cloud",
        vendor: Vendor::Salesforce,
        category: Category::IndustryCloud,
    },
    KnownPackage {
        namespace: "hc",
        display_name: "Health Cloud",
        vendor: Vendor::Salesforce,
        category: Category::IndustryCloud,
    },
    KnownPackage {
        namespace: "fferpcore",
        display_name: "FinancialForce ERP Core",
        vendor: Vendor::ThirdParty,
        category: Category::IndustryCloud,
    },
    KnownPackage {
        namespace: "vlocity_cmt",
        display_name: "Vlocity / Salesforce Industries (Communications, Media, Telco)",
        vendor: Vendor::Salesforce,
        category: Category::IndustryCloud,
    },
    KnownPackage {
        namespace: "vlocity_ins",
        display_name: "Vlocity / Salesforce Industries (Insurance)",
        vendor: Vendor::Salesforce,
        category: Category::IndustryCloud,
    },
    KnownPackage {
        namespace: "sf_com_apps",
        display_name: "Salesforce B2C Commerce Connector",
        vendor: Vendor::Salesforce,
        category: Category::IndustryCloud,
    },
    KnownPackage {
        namespace: "ccrz",
        display_name: "Salesforce B2B Commerce",
        vendor: Vendor::Salesforce,
        category: Category::IndustryCloud,
    },
    // ----- Prominent third-party ISV packages -----
    KnownPackage {
        namespace: "rh2",
        display_name: "Rollup Helper",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
    KnownPackage {
        namespace: "dlrs",
        display_name: "Declarative Lookup Rollup Summaries",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
    KnownPackage {
        namespace: "apxt_bss",
        display_name: "Apttus / Conga CPQ & Billing",
        vendor: Vendor::ThirdParty,
        category: Category::Cpq,
    },
    KnownPackage {
        namespace: "cpm",
        display_name: "Conga Composer",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
    KnownPackage {
        namespace: "docgen",
        display_name: "Conga DocGen",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
    KnownPackage {
        namespace: "echosign_dev1",
        display_name: "Adobe Acrobat Sign (formerly EchoSign)",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
    KnownPackage {
        namespace: "dsfs",
        display_name: "DocuSign for Salesforce",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
    KnownPackage {
        namespace: "geopointe",
        display_name: "Geopointe",
        vendor: Vendor::ThirdParty,
        category: Category::Analytics,
    },
    KnownPackage {
        namespace: "fflib",
        display_name: "Apex Enterprise Patterns (fflib-apex-common)",
        vendor: Vendor::ThirdParty,
        category: Category::Utility,
    },
];

/// Case-insensitive lookup. Returns `None` for any namespace not in the
/// curated table — callers should treat that as "unknown third-party
/// package, treat as `is_known_ecosystem_package = false`".
///
/// Linear scan is intentional: the table is small (~25 entries) and the
/// callsite runs once per external-reference synthesis, so a map-based
/// build + cache would cost more than it saves.
pub fn lookup(namespace: &str) -> Option<&'static KnownPackage> {
    let needle = namespace.trim();
    if needle.is_empty() {
        return None;
    }
    KNOWN_PACKAGES
        .iter()
        .find(|pkg| pkg.namespace.eq_ignore_ascii_case(needle))
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_hits_known_package() {
        let pkg = lookup("npsp").expect("npsp must be registered");
        assert_eq!(pkg.display_name, "Nonprofit Success Pack");
        assert_eq!(pkg.vendor, Vendor::SalesforceOrg);
        assert_eq!(pkg.category, Category::Nonprofit);
    }

    #[test]
    fn case_insensitive_match_works() {
        assert!(lookup("NPSP").is_some());
        assert!(lookup("Npsp").is_some());
        assert!(lookup("nPsP").is_some());
        assert!(lookup("SBQQ").is_some());
    }

    #[test]
    fn unknown_namespace_returns_none() {
        assert!(lookup("totallyunknownpkg").is_none());
        assert!(lookup("").is_none());
        assert!(lookup("   ").is_none());
    }

    #[test]
    fn whitespace_is_trimmed_before_lookup() {
        assert!(lookup("  npsp  ").is_some());
    }

    #[test]
    fn vendor_and_category_property_strings_are_stable_snake_case() {
        assert_eq!(Vendor::SalesforceOrg.as_property_str(), "salesforce_org");
        assert_eq!(Vendor::Salesforce.as_property_str(), "salesforce");
        assert_eq!(Vendor::ThirdParty.as_property_str(), "third_party");

        assert_eq!(Category::Nonprofit.as_property_str(), "nonprofit");
        assert_eq!(Category::Cpq.as_property_str(), "cpq");
        assert_eq!(Category::IndustryCloud.as_property_str(), "industry_cloud");
        assert_eq!(Category::Marketing.as_property_str(), "marketing");
    }

    #[test]
    fn every_registry_entry_has_lowercase_namespace() {
        // Guards against future entries accidentally being added
        // uppercase — would silently miss all lookups.
        for pkg in KNOWN_PACKAGES {
            assert_eq!(
                pkg.namespace,
                pkg.namespace.to_ascii_lowercase(),
                "registry entry `{}` is not lowercase",
                pkg.namespace
            );
            assert!(
                !pkg.display_name.trim().is_empty(),
                "registry entry `{}` has an empty display_name",
                pkg.namespace
            );
        }
    }

    #[test]
    fn registry_has_no_duplicate_namespaces() {
        // Earlier entry would shadow later ones — catch accidentally
        // registering the same namespace twice with different metadata.
        let mut namespaces: Vec<&str> = KNOWN_PACKAGES.iter().map(|p| p.namespace).collect();
        namespaces.sort();
        let len_before = namespaces.len();
        namespaces.dedup();
        assert_eq!(
            namespaces.len(),
            len_before,
            "duplicate namespace in KNOWN_PACKAGES"
        );
    }
}
