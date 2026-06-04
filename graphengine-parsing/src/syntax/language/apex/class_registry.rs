//! Case-insensitive Apex type registry.
//!
//! Apex has three properties that a resolver must take seriously:
//!
//! 1. **Case-insensitive identifiers.** `Account`, `account`, and `ACCOUNT`
//!    all refer to the same type. Resolution MUST normalize.
//! 2. **Global flat namespace.** All top-level classes live in one scope
//!    per namespace. There are no imports; reference-by-short-name is
//!    the rule, not the exception.
//! 3. **Standard SObject + system type preload.** Standard objects like
//!    `Account` and system classes like `Database`, `Schema`, `System`,
//!    `Test` are never present in source — they are implicitly available
//!    and must be recognized as external references, not dropped as
//!    unknowns. A heuristic that doesn't know about `Database.query(...)`
//!    produces nonsense resolution-quality numbers.
//!
//! This module provides [`ApexClassRegistry`], a case-insensitive
//! dictionary seeded with the standard SObject / system-type registry and
//! enriched at scan time with user-defined types. It is consumed by the
//! heuristic resolver (when LSP is unavailable) and by the
//! resolution-quality telemetry that LSP-primary runs still emit — even
//! when LSP resolves an edge, the registry gives us a ground-truth
//! classifier for whether a missing resolution is a bug or a legitimate
//! dynamic / external reference.
//!
//! # Managed-package namespace awareness
//!
//! Salesforce enforces `namespace__ApiName` for identifiers that cross
//! package boundaries (e.g. `npsp__Opportunity`, `fflib__Application`).
//! The registry splits these on `__`, records the namespace, and groups
//! them as external virtual entries. Internal naming conventions like
//! `fflib_Something` (single underscore) are NOT namespaces — they are
//! part of the base identifier.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::containment_walker::DottedPathProvider;
use crate::domain::apex::class_symbols::ApexClassSymbols;

/// Source of truth for an [`ApexTypeEntry`]: what introduced this name
/// into the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApexTypeSource {
    /// Declared in one of the user's `.cls` or `.trigger` files.
    UserDeclared {
        /// Absolute path to the file that declared the type.
        path: PathBuf,
        /// `Some(Outer)` when this is an inner class. `None` for top-level.
        enclosing: Option<String>,
    },
    /// Preloaded from the static standard-SObject / system-type table.
    /// Always treated as `external = true` by downstream graph nodes.
    StandardPreload,
    /// Introduced by an `*.object-meta.xml` file, either custom or
    /// (rarely, in MDAPI exports) standard.
    ObjectMeta { path: PathBuf },
    /// External entry inferred from a `namespace__Name` identifier seen
    /// somewhere in user source. No on-disk declaration.
    ManagedPackage { namespace: String },
}

/// What kind of Apex declaration the entry represents. Kept narrow: the
/// registry is about *name resolution*, not about graph node modeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApexTypeKind {
    /// Ordinary `public class Foo {}`.
    Class,
    /// `public interface Foo {}`.
    Interface,
    /// `public enum Foo { A, B }`.
    Enum,
    /// `trigger Foo on Account (...)`.
    Trigger,
    /// One of the preloaded standard SObjects (Account, Contact, …).
    StandardSObject,
    /// User-defined SObject from `*.object-meta.xml`.
    CustomSObject,
    /// Preloaded system class or namespace (Database, Schema, System, …).
    SystemType,
    /// Virtual external node for a managed-package reference.
    ManagedPackageExternal,
}

impl ApexTypeKind {
    /// `true` when this entry represents something declared *inside* the
    /// user's source. Used to distinguish cycle / blast-radius targets
    /// from external references.
    pub fn is_user_defined(&self) -> bool {
        matches!(
            self,
            ApexTypeKind::Class
                | ApexTypeKind::Interface
                | ApexTypeKind::Enum
                | ApexTypeKind::Trigger
                | ApexTypeKind::CustomSObject
        )
    }

    /// `true` for every type that never has source in the user's repo
    /// and therefore renders as an `external=true` node.
    pub fn is_external(&self) -> bool {
        matches!(
            self,
            ApexTypeKind::StandardSObject
                | ApexTypeKind::SystemType
                | ApexTypeKind::ManagedPackageExternal
        )
    }
}

/// A single entry in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexTypeEntry {
    /// Original-case fully-qualified name, exactly as declared. Used for
    /// rendering in reports; never compared case-sensitively.
    pub api_name: String,
    /// What the entry represents.
    pub kind: ApexTypeKind,
    /// How the entry got into the registry.
    pub source: ApexTypeSource,
    /// Managed-package namespace, when the API name includes `ns__`.
    /// `None` for plain identifiers.
    pub namespace: Option<String>,
    /// Fields / methods / constructors / inner-class names / parent /
    /// implemented-interfaces for this type (TR-A.0 type oracle).
    ///
    /// `None` for every entry until the Apex symbols pipeline is wired
    /// (TR-A.1+); `None` forever for preloaded system types and
    /// managed-package virtual entries, which don't have source we
    /// can extract shapes from. Consumers MUST treat `None` as
    /// "oracle unavailable, degrade to name-based heuristic" rather
    /// than "no members".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbols: Option<ApexClassSymbols>,
}

/// Recorded when two different declarations share a case-insensitive
/// name. Apex accepts this within a namespace only if the declarations
/// live in different scopes (different outer classes), but in practice
/// collisions are almost always bugs waiting to bite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexTypeCollision {
    /// Lowercase key that collided.
    pub key: String,
    /// The entry already present in the registry.
    pub existing: ApexTypeEntry,
    /// The entry that was attempted second.
    pub conflicting: ApexTypeEntry,
}

/// Case-insensitive Apex type registry.
///
/// Key invariant: every map lookup is performed on the lowercase form of
/// the input. Insertion preserves the first wins. A second insertion with
/// a different source is recorded in [`collisions`](Self::collisions)
/// instead of silently overwriting.
#[derive(Debug, Default, Clone)]
pub struct ApexClassRegistry {
    entries: HashMap<String, ApexTypeEntry>,
    collisions: Vec<ApexTypeCollision>,
}

impl ApexClassRegistry {
    /// Empty registry. Typically you want [`Self::with_standard_preload`]
    /// instead, which seeds in the standard SObject + system-type table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry preloaded with the standard SObject + system-type table.
    /// Insertion order is deterministic so the resulting entry ordering
    /// is stable across runs — important for report reproducibility.
    pub fn with_standard_preload() -> Self {
        let mut r = Self::new();
        for name in STANDARD_SOBJECTS {
            r.insert_preload(name, ApexTypeKind::StandardSObject);
        }
        for name in SYSTEM_TYPES {
            r.insert_preload(name, ApexTypeKind::SystemType);
        }
        r
    }

    /// Number of entries currently in the registry.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when the registry contains no entries at all. Practically
    /// only happens before [`Self::with_standard_preload`] has run.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow the list of collisions recorded so far. A non-empty list
    /// is surfaced as a scan diagnostic — duplicate case-insensitive
    /// names are rarely intentional in Apex.
    pub fn collisions(&self) -> &[ApexTypeCollision] {
        &self.collisions
    }

    /// Look up an entry by identifier. The input is case-normalized
    /// before lookup; callers never need to pre-lowercase.
    ///
    /// `Outer.Inner` dotted lookups are attempted verbatim first; when
    /// they miss, the short name (`Inner`) is tried as a fallback so
    /// unqualified references to inner classes still resolve when the
    /// outer scope is implicit.
    pub fn lookup(&self, name: &str) -> Option<&ApexTypeEntry> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return None;
        }
        let key = trimmed.to_ascii_lowercase();
        if let Some(e) = self.entries.get(&key) {
            return Some(e);
        }
        // Short-name fallback for inner classes. `Outer.Inner` → `inner`.
        if let Some((_, last)) = trimmed.rsplit_once('.') {
            let short = last.to_ascii_lowercase();
            if short != key {
                return self.entries.get(&short);
            }
        }
        None
    }

    /// Iterate over every (lowercase key, entry) pair in the registry.
    /// Order is unspecified (HashMap). Do not depend on it for report
    /// output — sort explicitly at render time.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ApexTypeEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Insert a user-declared class / interface / enum / trigger. Returns
    /// `true` on success, `false` when the insertion collided with an
    /// existing entry. Collisions are recorded in
    /// [`collisions`](Self::collisions) regardless.
    pub fn insert_user_declared(
        &mut self,
        fqn: &str,
        kind: ApexTypeKind,
        path: PathBuf,
        enclosing: Option<String>,
    ) -> bool {
        let namespace = extract_managed_namespace(fqn);
        self.insert(ApexTypeEntry {
            api_name: fqn.to_string(),
            kind,
            source: ApexTypeSource::UserDeclared { path, enclosing },
            namespace,
            symbols: None,
        })
    }

    /// Insert an inner class / interface / enum using the Sprint E.2
    /// dotted shape so both `Outer.Inner` and short-name fallback
    /// lookups resolve deterministically.
    ///
    /// The primary key is the dotted `Outer.Inner` form, which means
    /// two inner classes with the same simple name in different outer
    /// classes (`OuterA.Inner` vs `OuterB.Inner`) coexist without
    /// triggering a case-insensitive collision. Short-name lookups
    /// (`Inner`) still resolve via [`Self::lookup`]'s dotted→short
    /// fallback, and will return whichever entry wins the HashMap
    /// iteration order — callers that need unambiguous resolution
    /// MUST qualify the name.
    pub fn insert_inner_user_declared(
        &mut self,
        outer: &str,
        inner: &str,
        kind: ApexTypeKind,
        path: PathBuf,
    ) -> bool {
        let dotted = format!("{outer}.{inner}");
        let namespace = extract_managed_namespace(&dotted);
        self.insert(ApexTypeEntry {
            api_name: dotted,
            kind,
            source: ApexTypeSource::UserDeclared {
                path,
                enclosing: Some(outer.to_string()),
            },
            namespace,
            symbols: None,
        })
    }

    /// Insert a custom SObject discovered via its `*.object-meta.xml`.
    pub fn insert_custom_sobject(&mut self, api_name: &str, path: PathBuf) -> bool {
        let namespace = extract_managed_namespace(api_name);
        self.insert(ApexTypeEntry {
            api_name: api_name.to_string(),
            kind: ApexTypeKind::CustomSObject,
            source: ApexTypeSource::ObjectMeta { path },
            namespace,
            symbols: None,
        })
    }

    /// Insert a virtual external entry for a managed-package reference
    /// like `npsp__Opportunity`. Safe to call repeatedly for the same
    /// name — the first insertion wins and the rest are idempotent
    /// no-ops (not collisions).
    pub fn observe_managed_package_reference(&mut self, fqn: &str) {
        let Some(ns) = extract_managed_namespace(fqn) else {
            return;
        };
        let key = fqn.to_ascii_lowercase();
        if self.entries.contains_key(&key) {
            return;
        }
        self.entries.insert(
            key,
            ApexTypeEntry {
                api_name: fqn.to_string(),
                kind: ApexTypeKind::ManagedPackageExternal,
                source: ApexTypeSource::ManagedPackage {
                    namespace: ns.clone(),
                },
                namespace: Some(ns),
                symbols: None,
            },
        );
    }

    /// Attach or replace the [`ApexClassSymbols`] oracle payload on an
    /// existing user-declared entry. Returns `true` when a matching
    /// entry was found and updated, `false` when no entry exists under
    /// `fqn` (case-insensitive). Preloaded system types, custom
    /// SObjects, and managed-package virtual entries are protected: the
    /// oracle payload only makes sense on user-declared classes, so
    /// calls targeting any other kind are refused.
    ///
    /// Intended callers: the Apex parse-pipeline shim that walks the
    /// per-file symbols from [`super::class_symbols_extractor`] and
    /// stitches them onto the registry after all class declarations
    /// for the file have been indexed. This keeps TR-A.0's surface
    /// tight: the extractor produces symbols, the registry stores
    /// them, resolvers read them — no resolver arm bypasses the
    /// registry with its own ad-hoc type oracle.
    pub fn attach_symbols(&mut self, fqn: &str, symbols: ApexClassSymbols) -> bool {
        let key = fqn.trim().to_ascii_lowercase();
        if key.is_empty() {
            return false;
        }
        let Some(entry) = self.entries.get_mut(&key) else {
            return false;
        };
        if !entry.kind.is_user_defined() {
            return false;
        }
        entry.symbols = Some(symbols);
        true
    }

    /// Case-insensitive lookup of the [`ApexClassSymbols`] attached to
    /// a declared type. Returns `None` when the type is unknown, when
    /// it is external (preload / managed-package / custom SObject), or
    /// when no extractor pass has attached a payload yet.
    pub fn symbols_for(&self, api_name: &str) -> Option<&ApexClassSymbols> {
        self.lookup(api_name)
            .and_then(|entry| entry.symbols.as_ref())
    }

    // ---- internal ---------------------------------------------------------

    fn insert_preload(&mut self, name: &'static str, kind: ApexTypeKind) {
        // Preload entries are guaranteed unique and pre-vetted; we skip
        // collision accounting to keep the preload clean.
        let key = name.to_ascii_lowercase();
        self.entries.insert(
            key,
            ApexTypeEntry {
                api_name: name.to_string(),
                kind,
                source: ApexTypeSource::StandardPreload,
                namespace: None,
                symbols: None,
            },
        );
    }

    fn insert(&mut self, entry: ApexTypeEntry) -> bool {
        let key = entry.api_name.to_ascii_lowercase();
        if let Some(existing) = self.entries.get(&key) {
            if existing == &entry {
                // Exact duplicate — not a collision, just a redundant
                // insertion (common when walking overlapping package
                // directories). Idempotent.
                return true;
            }
            self.collisions.push(ApexTypeCollision {
                key,
                existing: existing.clone(),
                conflicting: entry,
            });
            return false;
        }
        self.entries.insert(key, entry);
        true
    }
}

/// Adapter so the registry can drive [`super::containment_walker`]
/// without leaking its internal representation. The walker only needs
/// `symbols_for`; `parent_of` uses the trait's default implementation
/// that delegates back through `symbols_for`.
impl DottedPathProvider for ApexClassRegistry {
    fn symbols_for(&self, api_name: &str) -> Option<&ApexClassSymbols> {
        ApexClassRegistry::symbols_for(self, api_name)
    }
}

/// Extract the managed-package namespace from an Apex identifier.
///
/// Salesforce enforces `namespace__Name` for cross-package references. We
/// split on the first `__` and take the left side. If there's no `__`
/// prefix or the left side is empty, the identifier is local and
/// returns `None`.
///
/// Edge cases handled:
/// - `npsp__Opportunity` → `Some("npsp")`.
/// - `Foo__c` → `None` (the `__c` suffix is the custom-object marker,
///   not a namespace — the part to the left is not preceded by a
///   valid namespace token).
/// - `Foo_Bar` (single underscore) → `None` (not a namespace separator).
/// - `__Foo` (leading double underscore) → `None` (no namespace token).
pub fn extract_managed_namespace(api_name: &str) -> Option<String> {
    let trimmed = api_name.trim();
    // Dotted forms: namespace lives on the leftmost segment.
    // `npsp__Opportunity.Inner` → namespace = "npsp".
    let head = trimmed.split('.').next().unwrap_or(trimmed);

    let (ns, rest) = head.split_once("__")?;
    if ns.is_empty() || rest.is_empty() {
        return None;
    }

    // Per Salesforce's managed-package spec, a namespace prefix is a
    // "one- to 15-character alphanumeric identifier". No underscores,
    // no hyphens, no leading digit. Anything else — typically a
    // multi-word custom-field stem like `General_Accounting_Unit` —
    // cannot be a real managed namespace and must be rejected, or the
    // inventory fills up with pseudo-namespaces whose only signal is
    // that the API name happened to contain `__` followed by a
    // non-marker suffix (`__r`, `__s`, `__History`, `__Share`, …).
    if !is_valid_namespace_shape(ns) {
        return None;
    }

    // `__c`, `__mdt`, `__e`, `__b`, `__x`, `__r`, `__s`, and the
    // trailing system markers (`__History`, `__Share`, `__Feed`,
    // `__ChangeEvent`, `__Tag`) at the end of the head are Salesforce
    // custom-object / custom-field / relationship markers, not
    // namespaces. Distinguish by looking at what's to the right of the
    // first `__` — if it's one of these markers, the identifier is a
    // local custom entity, not a managed reference.
    if is_custom_object_marker(rest) {
        return None;
    }
    Some(ns.to_string())
}

/// True when `segment` is shaped like a Salesforce managed-package
/// namespace prefix: 1–15 ASCII alphanumerics, no underscores, not
/// starting with a digit, and either (a) starts with a lowercase
/// letter or (b) contains a digit. The last clause is a practical
/// heuristic: the entire installed-package ecosystem that consumers
/// typically cite in Apex — `npsp`, `npe01`, `npe03`, `npo02`, `fflib`,
/// `pse`, `pi`, `rh2`, `blng`, `sbqq` — follows this convention, and
/// rejecting PascalCase-only tokens (`Location`, `Opportunity`,
/// `Account`, `General`) is the single biggest FP reducer we can
/// apply without type information. A vendor shipping an exclusively
/// PascalCase namespace will be missed, but in the observed Salesforce
/// ecosystem that case is rare enough that the LSP path will cover it.
/// Case is preserved in the return — callers lowercase downstream.
fn is_valid_namespace_shape(segment: &str) -> bool {
    if segment.is_empty() || segment.len() > 15 {
        return false;
    }
    let first = segment.as_bytes()[0];
    if !first.is_ascii_alphabetic() {
        return false;
    }
    if !segment.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let starts_lowercase = first.is_ascii_lowercase();
    let has_digit = segment.bytes().any(|b| b.is_ascii_digit());
    starts_lowercase || has_digit
}

/// Salesforce system / custom-entity markers that appear **after** a
/// `__` in an API name. If the portion to the right of the first `__`
/// is one of these, the overall identifier is a local custom entity or
/// relationship reference — *not* a managed-package reference.
///
/// Sourced from the Salesforce "Custom Object" and "Standard Objects"
/// references plus the change-data-capture, feed, history, share, and
/// tag auxiliary objects. The list is intentionally additive — adding
/// a marker can only *reduce* false positives; it can never suppress a
/// legitimate managed-package namespace because real namespaces never
/// take these forms as their entire post-`__` tail.
fn is_custom_object_marker(segment: &str) -> bool {
    matches!(
        segment,
        "c"            // custom field / object
            | "mdt"    // custom metadata type
            | "e"      // platform event
            | "b"      // big object
            | "x"      // external object
            | "r"      // relationship reference
            | "s"      // geolocation sub-field (latitude__s / longitude__s)
            | "pc"     // person-account custom field
            | "History"
            | "Share"
            | "Feed"
            | "Tag"
            | "ChangeEvent"
    )
}

// =============================================================================
// Standard SObject + system-type preload tables.
//
// These lists are intentionally conservative: they cover the symbols a
// heuristic resolver will hit in 95%+ of real-world Apex scans, without
// trying to be exhaustive — that's LSP's job. Adding an entry here costs
// nothing at runtime (HashMap<String, ApexTypeEntry> lookup is O(1)).
// =============================================================================

/// The most commonly referenced standard SObjects. Covers sales cloud,
/// service cloud, marketing, platform admin, files, and content. Does
/// not include industry-cloud / vertical-specific objects — those
/// typically ship under managed-package namespaces that the registry
/// discovers dynamically instead.
const STANDARD_SOBJECTS: &[&str] = &[
    // Sales Cloud core
    "Account",
    "Contact",
    "Lead",
    "Opportunity",
    "OpportunityLineItem",
    "Quote",
    "QuoteLineItem",
    "Order",
    "OrderItem",
    "Contract",
    "Campaign",
    "CampaignMember",
    "Asset",
    "Product2",
    "Pricebook2",
    "PricebookEntry",
    // Service Cloud
    "Case",
    "CaseComment",
    "CaseHistory",
    "EmailMessage",
    "EmailTemplate",
    "Solution",
    // Activities
    "Task",
    "Event",
    "Note",
    "Attachment",
    // Users / security / org
    "User",
    "UserLicense",
    "Profile",
    "UserRole",
    "Group",
    "GroupMember",
    "PermissionSet",
    "PermissionSetAssignment",
    "PermissionSetLicenseAssign",
    "Organization",
    "Domain",
    "Site",
    "Network",
    "Territory2",
    "LoginHistory",
    // Records / metadata
    "RecordType",
    "BusinessProcess",
    "QueueSobject",
    "RecordTypeLocalization",
    // Files & content
    "ContentVersion",
    "ContentDocument",
    "ContentDocumentLink",
    "ContentWorkspace",
    "ContentFolder",
    "Document",
    "Folder",
    "StaticResource",
    // Chatter / feeds
    "FeedItem",
    "FeedComment",
    "CollaborationGroup",
    "CollaborationGroupMember",
    // Custom development artifacts (themselves SObjects)
    "ApexClass",
    "ApexTrigger",
    "ApexPage",
    "ApexComponent",
    "CustomObject",
    "CustomField",
    // Process / flow tracking
    "ProcessInstance",
    "ProcessInstanceStep",
    "ProcessInstanceWorkitem",
    "FlowDefinitionView",
    "FlowVersionView",
    // Reports / dashboards
    "Report",
    "Dashboard",
    // Events / big objects
    "AsyncApexJob",
    "CronTrigger",
    "AsyncOperationLog",
    "AsyncApexJobFlex",
    // Misc
    "BusinessHours",
    "Holiday",
    "Idea",
    "Scorecard",
];

/// The system namespaces and classes every Apex compiler exposes. These
/// are not SObjects — they're classes / namespaces in the `System`
/// package that user code dots into without ever importing (Apex has
/// no import syntax).
const SYSTEM_TYPES: &[&str] = &[
    // Core System namespace members
    "System",
    "UserInfo",
    "Limits",
    "Test",
    "TestSetup",
    "Database",
    "Schema",
    "Messaging",
    "Approval",
    "Auth",
    "ConnectApi",
    "Reports",
    "Packaging",
    "Cache",
    "EventBus",
    "Dom",
    "Flow",
    "Canvas",
    "Site",
    "Functions",
    "DataSource",
    "Quickbooks",
    "Label",
    "Url",
    "Network",
    "UserSettingsPersonal",
    "Apex",
    // Primitives / wrappers (case-insensitive resolution uses these too)
    "Blob",
    "Boolean",
    "Date",
    "Datetime",
    "Decimal",
    "Double",
    "Id",
    "Integer",
    "Long",
    "Object",
    "String",
    "Time",
    // Collections
    "List",
    "Map",
    "Set",
    "Iterator",
    "Iterable",
    "Comparable",
    // Web / HTTP
    "Http",
    "HttpRequest",
    "HttpResponse",
    "RestRequest",
    "RestResponse",
    "Cookie",
    "PageReference",
    "ApexPages",
    "SelectOption",
    // Crypto / encoding / regex
    "Crypto",
    "EncodingUtil",
    "Pattern",
    "Matcher",
    "JSON",
    "JSONParser",
    "JSONGenerator",
    "JSONToken",
    // Math / runtime
    "Math",
    "Savepoint",
    "Type",
    // Exception hierarchy (hit often in catches and tests)
    "Exception",
    "DmlException",
    "QueryException",
    "ListException",
    "MathException",
    "NullPointerException",
    "NoAccessException",
    "LimitException",
    "TypeException",
    "CalloutException",
    "AsyncException",
    "SecurityException",
    // Trigger context type (filtered specially in receiver_detector,
    // but still present so lookups don't drop it into Unknown).
    "Trigger",
];

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn preload_seeds_standard_sobjects_and_system_types() {
        let reg = ApexClassRegistry::with_standard_preload();
        // Spot-check across every subcategory.
        assert!(reg.lookup("Account").is_some());
        assert!(reg.lookup("Contact").is_some());
        assert!(reg.lookup("User").is_some());
        assert!(reg.lookup("Case").is_some());
        assert!(reg.lookup("Database").is_some());
        assert!(reg.lookup("System").is_some());
        assert!(reg.lookup("Test").is_some());
        assert!(reg.lookup("JSON").is_some());
        assert!(reg.lookup("Exception").is_some());
        assert!(reg.lookup("Http").is_some());
        assert!(reg.lookup("Trigger").is_some());
    }

    #[test]
    fn lookup_is_case_insensitive_in_all_directions() {
        let reg = ApexClassRegistry::with_standard_preload();
        assert_eq!(
            reg.lookup("Account").map(|e| e.api_name.as_str()),
            Some("Account"),
        );
        assert_eq!(
            reg.lookup("account").map(|e| e.api_name.as_str()),
            Some("Account"),
        );
        assert_eq!(
            reg.lookup("ACCOUNT").map(|e| e.api_name.as_str()),
            Some("Account"),
        );
        assert_eq!(
            reg.lookup("AcCoUnT").map(|e| e.api_name.as_str()),
            Some("Account"),
        );
    }

    #[test]
    fn inner_class_dotted_insert_resolves_by_both_paths() {
        // Sprint E.2: two inner classes with the same simple name in
        // different outer classes must both land in the registry and
        // be retrievable via their dotted form.
        let mut reg = ApexClassRegistry::new();
        let a_ok = reg.insert_inner_user_declared(
            "OuterA",
            "Inner",
            ApexTypeKind::Class,
            PathBuf::from("/x/OuterA.cls"),
        );
        let b_ok = reg.insert_inner_user_declared(
            "OuterB",
            "Inner",
            ApexTypeKind::Class,
            PathBuf::from("/x/OuterB.cls"),
        );
        assert!(a_ok && b_ok);
        assert!(
            reg.collisions().is_empty(),
            "dotted inner-class keys must not collide: {:?}",
            reg.collisions()
        );

        // Each dotted name resolves to its own entry.
        let a = reg.lookup("OuterA.Inner").unwrap();
        let b = reg.lookup("OuterB.Inner").unwrap();
        assert_eq!(a.api_name, "OuterA.Inner");
        assert_eq!(b.api_name, "OuterB.Inner");
        assert!(matches!(
            &a.source,
            ApexTypeSource::UserDeclared { enclosing, .. }
                if enclosing.as_deref() == Some("OuterA")
        ));
    }

    #[test]
    fn inner_class_short_name_fallback() {
        let mut reg = ApexClassRegistry::new();
        reg.insert_user_declared(
            "Inner",
            ApexTypeKind::Class,
            PathBuf::from("/x/Foo.cls"),
            Some("Outer".to_string()),
        );
        // Dotted lookup misses because we only indexed the short name,
        // but the fallback must then find it via the short form.
        assert_eq!(
            reg.lookup("Outer.Inner").map(|e| e.api_name.as_str()),
            Some("Inner")
        );
        // Plain short-name lookup also works.
        assert_eq!(
            reg.lookup("Inner").map(|e| e.api_name.as_str()),
            Some("Inner")
        );
    }

    #[test]
    fn duplicate_user_declared_type_records_collision() {
        let mut reg = ApexClassRegistry::new();
        let ok = reg.insert_user_declared(
            "Helper",
            ApexTypeKind::Class,
            PathBuf::from("/a/Helper.cls"),
            None,
        );
        assert!(ok);

        let conflict = reg.insert_user_declared(
            "HELPER",
            ApexTypeKind::Interface,
            PathBuf::from("/b/Helper.cls"),
            None,
        );
        assert!(!conflict, "case-insensitive collision must return false");
        assert_eq!(reg.collisions().len(), 1);

        // First-wins semantics — the original Class entry survives.
        let e = reg.lookup("helper").unwrap();
        assert_eq!(e.kind, ApexTypeKind::Class);
        assert_eq!(e.api_name, "Helper");
    }

    #[test]
    fn identical_duplicate_is_idempotent_not_a_collision() {
        let mut reg = ApexClassRegistry::new();
        let path = PathBuf::from("/a/Helper.cls");
        let ok1 = reg.insert_user_declared("Helper", ApexTypeKind::Class, path.clone(), None);
        let ok2 = reg.insert_user_declared("Helper", ApexTypeKind::Class, path.clone(), None);
        assert!(ok1 && ok2);
        assert!(
            reg.collisions().is_empty(),
            "identical re-insertion must not count as a collision"
        );
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn preload_types_are_marked_external() {
        let reg = ApexClassRegistry::with_standard_preload();
        let entry = reg.lookup("Account").unwrap();
        assert_eq!(entry.kind, ApexTypeKind::StandardSObject);
        assert!(entry.kind.is_external());
        assert!(!entry.kind.is_user_defined());
        assert_eq!(entry.source, ApexTypeSource::StandardPreload);
    }

    #[test]
    fn user_class_is_user_defined_not_external() {
        let mut reg = ApexClassRegistry::new();
        reg.insert_user_declared(
            "MyService",
            ApexTypeKind::Class,
            PathBuf::from("/x/MyService.cls"),
            None,
        );
        let entry = reg.lookup("MyService").unwrap();
        assert!(entry.kind.is_user_defined());
        assert!(!entry.kind.is_external());
    }

    #[test]
    fn custom_sobject_carries_source_path() {
        let mut reg = ApexClassRegistry::new();
        let obj_path =
            PathBuf::from("/repo/force-app/main/default/objects/Foo__c/Foo__c.object-meta.xml");
        reg.insert_custom_sobject("Foo__c", obj_path.clone());
        let e = reg.lookup("Foo__c").unwrap();
        assert_eq!(e.kind, ApexTypeKind::CustomSObject);
        assert!(matches!(&e.source, ApexTypeSource::ObjectMeta { path } if path == &obj_path));
    }

    #[test]
    fn managed_package_namespace_extracted_and_recorded() {
        let mut reg = ApexClassRegistry::new();
        reg.observe_managed_package_reference("npsp__Opportunity");
        let entry = reg.lookup("npsp__Opportunity").unwrap();
        assert_eq!(entry.kind, ApexTypeKind::ManagedPackageExternal);
        assert_eq!(entry.namespace.as_deref(), Some("npsp"));
        assert!(matches!(
            &entry.source,
            ApexTypeSource::ManagedPackage { namespace } if namespace == "npsp"
        ));
    }

    #[test]
    fn observing_same_managed_package_reference_twice_is_idempotent() {
        let mut reg = ApexClassRegistry::new();
        reg.observe_managed_package_reference("npsp__Opportunity");
        reg.observe_managed_package_reference("NPSP__Opportunity"); // case-variant
        assert_eq!(reg.len(), 1);
        assert!(reg.collisions().is_empty());
    }

    #[test]
    fn extract_managed_namespace_handles_edge_cases() {
        assert_eq!(
            extract_managed_namespace("npsp__Opportunity"),
            Some("npsp".to_string())
        );
        assert_eq!(
            extract_managed_namespace("fflib__Application"),
            Some("fflib".to_string())
        );
        // Namespace-qualified inner class: namespace still on the left.
        assert_eq!(
            extract_managed_namespace("npsp__Opportunity.InnerClass"),
            Some("npsp".to_string())
        );

        // Custom-object markers must not be mistaken for namespaces.
        assert_eq!(extract_managed_namespace("Foo__c"), None);
        assert_eq!(extract_managed_namespace("My_Setting__mdt"), None);
        assert_eq!(extract_managed_namespace("Order_Placed__e"), None);

        // Relationship / geolocation / system-object markers — these
        // were the dominant class of NPSP false positives pre-fix.
        assert_eq!(extract_managed_namespace("Account__r"), None);
        assert_eq!(
            extract_managed_namespace("Target_Object_Mapping__r"),
            None,
            "multi-word custom-field stems followed by __r are local relationship fields, not managed namespaces",
        );
        assert_eq!(extract_managed_namespace("Location__latitude__s"), None);
        assert_eq!(extract_managed_namespace("Location__longitude__s"), None);
        assert_eq!(extract_managed_namespace("Opportunity__History"), None);
        assert_eq!(extract_managed_namespace("Account__Share"), None);
        assert_eq!(extract_managed_namespace("Case__Feed"), None);
        assert_eq!(extract_managed_namespace("Contact__ChangeEvent"), None);
        assert_eq!(extract_managed_namespace("Account__Tag"), None);
        assert_eq!(extract_managed_namespace("Region__pc"), None);

        // Per Salesforce spec namespaces are 1–15 alphanumeric chars
        // only. Anything with `_` in the left segment cannot be a
        // managed namespace (this is what caused `general_accounting_unit`,
        // `target_object_mapping`, `opportunity` to leak into the NPSP
        // baseline before the shape check was added).
        assert_eq!(
            extract_managed_namespace("General_Accounting_Unit__Foo__c"),
            None,
            "underscores in the namespace segment violate the managed-package spec — must reject",
        );
        assert_eq!(
            extract_managed_namespace("Target_Object_Mapping__Foo__c"),
            None,
        );
        assert_eq!(
            extract_managed_namespace("1foo__Bar"),
            None,
            "namespace must start with a letter per Salesforce spec",
        );
        assert_eq!(
            extract_managed_namespace("abcdefghijklmnop__Bar"),
            None,
            "namespace may be at most 15 characters",
        );

        // Single underscores are naming convention, not namespace.
        assert_eq!(extract_managed_namespace("fflib_Application"), None);

        // Degenerate inputs.
        assert_eq!(extract_managed_namespace(""), None);
        assert_eq!(extract_managed_namespace("__Foo"), None);
        assert_eq!(extract_managed_namespace("Foo__"), None);
        assert_eq!(extract_managed_namespace("Foo"), None);
    }

    #[test]
    fn preload_size_is_nontrivial_and_stable() {
        // Sanity: we expect >= 90 preloaded entries so the registry is
        // genuinely useful out of the box, not a token gesture.
        let reg = ApexClassRegistry::with_standard_preload();
        assert!(
            reg.len() >= 90,
            "preload table should cover ~100 entries, got {}",
            reg.len()
        );
    }

    #[test]
    fn lookup_ignores_leading_trailing_whitespace() {
        let reg = ApexClassRegistry::with_standard_preload();
        assert!(reg.lookup("  Account  ").is_some());
        assert!(reg.lookup("\tAccount\n").is_some());
    }

    #[test]
    fn empty_lookup_returns_none() {
        let reg = ApexClassRegistry::with_standard_preload();
        assert!(reg.lookup("").is_none());
        assert!(reg.lookup("   ").is_none());
    }

    #[test]
    fn iter_yields_every_entry() {
        let reg = ApexClassRegistry::with_standard_preload();
        let count = reg.iter().count();
        assert_eq!(count, reg.len());
    }
}
