//! Trigger-framework detector.
//!
//! The widely-recommended Salesforce pattern is **one trigger per SObject**
//! that delegates to a handler class — popular frameworks include
//! `sfdc-trigger-framework` (Kevin O'Hara), `fflib-apex-common`, `Metaforce`,
//! `trigger-handler-framework`, NPSP's `TDTM` (`Trigger_Handler` custom
//! metadata-driven dispatcher), and several hand-rolled variants. These
//! frameworks let teams register many handler classes against a single
//! SObject *without* creating multiple physical `trigger` files, so a naive
//! "≥ 2 triggers on SObject X" check will fire false positives on repos
//! that are actually doing the right thing.
//!
//! This module inspects a repository's parsed Apex symbols and trigger
//! bodies to decide whether a recognised framework is in use. Output is
//! surfaced to the `MultipleTriggersPerSObject` analysis finding so it can
//! downgrade severity (or suppress entirely) when the "multiple triggers"
//! are intentional framework-scaffold artifacts rather than a real smell.
//!
//! Detection is conservative — when unsure, we return **no** framework
//! and let the finding fire. A false positive on the finding is a minor
//! annoyance; a false *negative* on framework detection silently hides a
//! real architectural problem and is strictly worse.
//!
//! # Detection signals (ordered by strength)
//!
//! 1. **Handler interface/base-class presence** — an interface named
//!    `ITrigger`, `TriggerHandler`, `ITriggerHandler`, etc., OR a class
//!    named `TriggerHandler` with recognizable lifecycle methods
//!    (`beforeInsert`, `afterUpdate`, …).
//! 2. **Dispatcher class** — a class that instantiates a handler and
//!    calls `run()` / `execute()` on it (captured in handler base
//!    classes above).
//! 3. **NPSP TDTM signature** — presence of `TDTM_Runnable`, `TDTM_Config`,
//!    `TDTM_TriggerHandler` symbols.
//! 4. **fflib signature** — presence of `fflib_SObjectDomain` or
//!    `SObjectDomain.triggerHandler` call patterns.
//! 5. **Trigger body delegation** — the trigger file's body contains
//!    a one-liner like `new AccountHandler().run();` or
//!    `TriggerDispatcher.Run(new AccountHandler());` rather than imperative
//!    business logic.
//!
//! A framework is considered *present* when signal 1 or 3 or 4 fires OR
//! when signals 2 + 5 both fire. Single weak signals (e.g. just a
//! `TriggerHandler` name with nothing else) are not enough.

use std::collections::HashSet;

/// Known trigger-framework identities. Used for telemetry and for the
/// finding message ("multiple triggers detected on Account, but the
/// fflib framework is in use — severity downgraded").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerFramework {
    /// Kevin O'Hara's `sfdc-trigger-framework` (the most common OSS one).
    KevinOHara,
    /// Financial Force's `fflib-apex-common` / `ApexCommons`.
    Fflib,
    /// NPSP's Trigger-Dispatcher Template Method (`TDTM_*`) framework.
    NpspTdtm,
    /// A framework matched on structural signals (handler interface +
    /// dispatcher) but whose exact identity we cannot attribute.
    Generic,
}

impl TriggerFramework {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerFramework::KevinOHara => "kevin-ohara",
            TriggerFramework::Fflib => "fflib",
            TriggerFramework::NpspTdtm => "npsp-tdtm",
            TriggerFramework::Generic => "generic",
        }
    }
}

/// Detection result. `frameworks` is a set because a repo can adopt
/// more than one (e.g., NPSP's TDTM alongside a newer fflib handler for
/// a new domain area).
#[derive(Debug, Clone, Default)]
pub struct DetectionResult {
    pub frameworks: HashSet<TriggerFramework>,
}

impl DetectionResult {
    pub fn is_empty(&self) -> bool {
        self.frameworks.is_empty()
    }

    pub fn contains(&self, fw: TriggerFramework) -> bool {
        self.frameworks.contains(&fw)
    }
}

/// Minimal fact-set the detector needs about the parsed repository.
/// Callers populate this from the `SyntaxResults` / symbol tables they
/// already built — the detector itself is source-agnostic so it's
/// trivially testable without touching Tree-sitter.
#[derive(Debug, Clone, Default)]
pub struct TriggerFrameworkFacts {
    /// Names of every Apex class / interface declared in the repo.
    /// Case-preserved; lookups are case-insensitive internally.
    pub class_names: Vec<String>,
    /// Names of every interface declared (already in `class_names` too,
    /// but explicit here lets us strengthen signal 1).
    pub interface_names: Vec<String>,
    /// For each trigger file in the repo, the body text. Bodies are
    /// kept short in well-factored repos (usually < 20 lines), so
    /// string search is fine and avoids a second parse.
    pub trigger_bodies: Vec<String>,
}

impl TriggerFrameworkFacts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_class<S: Into<String>>(&mut self, name: S) {
        self.class_names.push(name.into());
    }

    pub fn add_interface<S: Into<String>>(&mut self, name: S) {
        self.interface_names.push(name.into());
    }

    pub fn add_trigger_body<S: Into<String>>(&mut self, body: S) {
        self.trigger_bodies.push(body.into());
    }
}

/// Well-known handler base / interface names. All lowercase here
/// because comparisons in [`has_handler_interface`] lowercase the input.
const HANDLER_BASE_NAMES: &[&str] = &[
    "itrigger",
    "itriggerhandler",
    "triggerhandler",
    "triggerhandlerbase",
    "sobjecttriggerhandler",
    "triggercontext",
    "triggerdispatcher",
];

/// NPSP TDTM marker symbols. Presence of any of these is a strong
/// signal of the NPSP framework.
const NPSP_TDTM_MARKERS: &[&str] = &[
    "tdtm_runnable",
    "tdtm_config",
    "tdtm_triggerhandler",
    "tdtm_processcontrol",
];

/// fflib marker symbols.
const FFLIB_MARKERS: &[&str] = &[
    "fflib_sobjectdomain",
    "fflib_sobjectselector",
    "fflib_sobjectunitofwork",
];

/// Lifecycle method names that, if seen together on a single class,
/// strongly suggest that class is a handler base.
const LIFECYCLE_METHODS: &[&str] = &[
    "beforeinsert",
    "afterinsert",
    "beforeupdate",
    "afterupdate",
    "beforedelete",
    "afterdelete",
    "afterundelete",
];

/// Run detection against the supplied facts. Idempotent, allocation-light.
pub fn detect(facts: &TriggerFrameworkFacts) -> DetectionResult {
    let mut result = DetectionResult::default();

    let class_names_lc: Vec<String> = facts
        .class_names
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let interface_names_lc: Vec<String> = facts
        .interface_names
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    // Signal 3: NPSP TDTM — any marker triggers it.
    if NPSP_TDTM_MARKERS
        .iter()
        .any(|m| class_names_lc.iter().any(|c| c == m) || interface_names_lc.iter().any(|c| c == m))
    {
        result.frameworks.insert(TriggerFramework::NpspTdtm);
    }

    // Signal 4: fflib — any marker.
    if FFLIB_MARKERS
        .iter()
        .any(|m| class_names_lc.iter().any(|c| c == m))
    {
        result.frameworks.insert(TriggerFramework::Fflib);
    }

    // Signal 1: handler interface / base class.
    let has_handler_iface = HANDLER_BASE_NAMES
        .iter()
        .any(|name| interface_names_lc.iter().any(|c| c == name));
    let has_handler_base_class = HANDLER_BASE_NAMES
        .iter()
        .any(|name| class_names_lc.iter().any(|c| c == name));

    if has_handler_iface {
        // An interface is unambiguous — code only defines `ITrigger`
        // for a reason. This is enough on its own.
        //
        // Attribute to KevinOHara specifically when the interface is
        // exactly `TriggerHandler` and there's no fflib/tdtm signal
        // already (that's the exact name his public repo exports).
        if interface_names_lc.iter().any(|c| c == "itrigger")
            && class_names_lc.iter().any(|c| c == "triggerhandler")
        {
            result.frameworks.insert(TriggerFramework::KevinOHara);
        } else {
            result.frameworks.insert(TriggerFramework::Generic);
        }
    }

    // Signal 2 + Signal 5 combination: a `TriggerHandler` base class
    // AND trigger bodies that delegate rather than do the work
    // themselves.
    let delegating_triggers = facts
        .trigger_bodies
        .iter()
        .filter(|b| looks_like_delegation(b))
        .count();
    let delegating_ratio = if facts.trigger_bodies.is_empty() {
        0.0
    } else {
        delegating_triggers as f64 / facts.trigger_bodies.len() as f64
    };

    if (has_handler_base_class || has_handler_iface) && delegating_ratio >= 0.5 {
        result.frameworks.insert(TriggerFramework::Generic);
    }

    // Tie-breaker: if we saw lifecycle methods concentrated on a handler-named
    // base class, upgrade Generic → KevinOHara-pattern. We can't tell from
    // names alone, but the lifecycle vocabulary is that framework's convention.
    if class_names_lc.iter().any(|c| c == "triggerhandler")
        && LIFECYCLE_METHODS
            .iter()
            .filter(|m| class_names_lc.iter().any(|c| c.contains(*m)))
            .count()
            >= 2
    {
        result.frameworks.insert(TriggerFramework::KevinOHara);
    }

    result
}

/// Does a trigger body look like a one-liner delegation to a handler?
///
/// Heuristic: short (<= 8 non-empty lines), contains either
/// `<Something>Handler` construction, or a dispatcher `.run(` /
/// `.execute(` call, and does NOT contain imperative-style statements
/// (`for (`, `if (`, `insert `, `update `, `delete `, `upsert `, `[SELECT`).
pub fn looks_like_delegation(body: &str) -> bool {
    let non_empty_lines: Vec<&str> = body
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with("/*"))
        .collect();

    if non_empty_lines.len() > 12 {
        return false;
    }

    let joined = non_empty_lines.join(" ");
    let joined_lc = joined.to_ascii_lowercase();

    let imperative_markers = [
        "for (", "for(", " if (", " if(", "insert ", "update ", "delete ", "upsert ", "[select",
    ];
    if imperative_markers.iter().any(|m| joined_lc.contains(m)) {
        return false;
    }

    let delegation_markers = [
        "handler()",
        "handler().",
        ".run(",
        ".execute(",
        "triggerdispatcher.",
        "trigger_handler",
        "new triggerhandler",
    ];
    delegation_markers.iter().any(|m| joined_lc.contains(m))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_facts_produce_no_frameworks() {
        let facts = TriggerFrameworkFacts::new();
        let r = detect(&facts);
        assert!(r.is_empty());
    }

    #[test]
    fn npsp_tdtm_marker_is_strong_signal() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_class("TDTM_Runnable");
        f.add_class("AccountTrigger");
        let r = detect(&f);
        assert!(r.contains(TriggerFramework::NpspTdtm));
    }

    #[test]
    fn fflib_marker_is_strong_signal() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_class("fflib_SObjectDomain");
        let r = detect(&f);
        assert!(r.contains(TriggerFramework::Fflib));
    }

    #[test]
    fn kevin_ohara_attribution_requires_both_iface_and_class() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_interface("ITrigger");
        f.add_class("TriggerHandler");
        let r = detect(&f);
        assert!(
            r.contains(TriggerFramework::KevinOHara),
            "ITrigger + TriggerHandler is the Kevin O'Hara signature: {:?}",
            r
        );
    }

    #[test]
    fn handler_interface_alone_is_generic_framework() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_interface("ITriggerHandler");
        let r = detect(&f);
        assert!(
            r.contains(TriggerFramework::Generic),
            "any trigger-handler interface is enough to downgrade MultipleTriggersPerSObject severity"
        );
    }

    #[test]
    fn case_insensitive_matching() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_class("FFLIB_SOBJECTDOMAIN");
        let r = detect(&f);
        assert!(r.contains(TriggerFramework::Fflib));
    }

    #[test]
    fn delegation_body_recognised() {
        assert!(looks_like_delegation(
            "trigger AccountTrigger on Account (before insert) {\n  new AccountHandler().run();\n}"
        ));
        assert!(looks_like_delegation(
            "trigger C on Contact (after update) {\n TriggerDispatcher.Run(new ContactHandler()); \n}"
        ));
    }

    #[test]
    fn imperative_body_not_recognised_as_delegation() {
        // Real business logic in the trigger => NOT delegation.
        let body = r#"
            trigger Acct on Account (before insert) {
                for (Account a : Trigger.new) {
                    if (a.Name == null) a.Name = 'DEFAULT';
                }
                insert new Task();
            }
        "#;
        assert!(!looks_like_delegation(body));
    }

    #[test]
    fn long_body_not_recognised_as_delegation() {
        let body = (0..30)
            .map(|i| format!("statement{};", i))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!looks_like_delegation(&body));
    }

    #[test]
    fn combined_handler_class_and_delegation_triggers_generic() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_class("TriggerHandler");
        f.add_trigger_body("new AccountHandler().run();");
        f.add_trigger_body("new ContactHandler().run();");
        let r = detect(&f);
        assert!(
            r.contains(TriggerFramework::Generic) || r.contains(TriggerFramework::KevinOHara),
            "handler base + delegating triggers should detect a framework: {:?}",
            r
        );
    }

    #[test]
    fn framework_string_identifiers_are_stable() {
        // Telemetry/analysis findings serialize these. Breaking them
        // would silently break downstream dashboards.
        assert_eq!(TriggerFramework::KevinOHara.as_str(), "kevin-ohara");
        assert_eq!(TriggerFramework::Fflib.as_str(), "fflib");
        assert_eq!(TriggerFramework::NpspTdtm.as_str(), "npsp-tdtm");
        assert_eq!(TriggerFramework::Generic.as_str(), "generic");
    }

    #[test]
    fn multiple_frameworks_can_coexist() {
        let mut f = TriggerFrameworkFacts::new();
        f.add_class("TDTM_Runnable");
        f.add_class("fflib_SObjectDomain");
        let r = detect(&f);
        assert!(r.contains(TriggerFramework::NpspTdtm));
        assert!(r.contains(TriggerFramework::Fflib));
        assert_eq!(r.frameworks.len(), 2);
    }

    #[test]
    fn no_false_positive_on_plain_apex_class_named_handler() {
        // A class literally named `SomethingHandler` with no trigger-
        // framework signature must NOT trip the detector — many Apex
        // codebases use the word "Handler" for every service class.
        let mut f = TriggerFrameworkFacts::new();
        f.add_class("EmailHandler");
        f.add_class("PaymentHandler");
        let r = detect(&f);
        assert!(
            r.is_empty(),
            "arbitrary *Handler classes must not trigger detection: {:?}",
            r
        );
    }
}
