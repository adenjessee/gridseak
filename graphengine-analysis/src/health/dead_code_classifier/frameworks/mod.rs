//! Framework-keyed dead-code rule sets.
//!
//! Each file in this module owns the rules for exactly one
//! framework tag as emitted by
//! [`graphengine_parsing::domain::frameworks`] and materialised
//! onto [`crate::health::graph::GraphNode::frameworks`]. The registry
//! in [`super`] walks a node's framework list, dispatches to each
//! matching rule set, and stops at the first rule that returns a
//! verdict. Cross-framework classification is data-driven: adding a
//! new framework only requires:
//!
//! 1. A new detector in `graphengine-parsing/src/domain/frameworks.rs`.
//! 2. A new rule-set module here.
//! 3. One registration line in [`register_default`] below.
//!
//! Rule sets intentionally DO NOT branch on the repo-level ecosystem.
//! The same rule set runs for any file with the matching framework
//! tag, even in polyglot repos where Apex and Python sit in the same
//! graph.
//!
//! # What lives here vs. what lives in the universal rules
//!
//! Universal rules (entry-point tag, `is_attribute_invoked`, parent
//! test, callback target) live in [`super::universal`] and run
//! *before* any framework-keyed rule. Framework modules only need to
//! encode heuristics that the universal rules can't express — for
//! example, the TDTM handler-naming convention, Django class-based
//! view methods without a fan-in edge, or the Apex `.trigger` file
//! dispatch contract.

pub mod aura;
pub mod celery;
pub mod django;
pub mod jest;
pub mod lwc;
pub mod plain;
pub mod restresource;
pub mod tdtm;
pub mod test_harness_common;
pub mod triggerdml;
pub mod vitest;

use super::FrameworkRuleSet;

/// Produce the default set of framework rule modules. The registry
/// uses this to seed its dispatch table. Keep the order stable so
/// verdict-classifier strings (`"apex-tdtm"`, `"python-django"`, …)
/// are deterministic across runs.
///
/// Grouping: Apex (tdtm, triggerdml, restresource) → Python
/// (django, celery) → JavaScript (lwc, aura, jest, vitest) →
/// plain terminal. Plain must remain last so it never shadows a
/// real framework-keyed rule.
pub fn register_default() -> Vec<Box<dyn FrameworkRuleSet>> {
    vec![
        Box::new(tdtm::TdtmRules),
        Box::new(triggerdml::TriggerDmlRules),
        Box::new(restresource::RestResourceRules),
        Box::new(django::DjangoRules),
        Box::new(celery::CeleryRules),
        Box::new(lwc::LwcRules),
        Box::new(aura::AuraRules),
        Box::new(jest::JestRules),
        Box::new(vitest::VitestRules),
        Box::new(plain::PlainRules),
    ]
}
