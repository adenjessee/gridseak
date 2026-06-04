//! Salesforce Aura component rule set.
//!
//! Aura bundles live in `aura/<Component>/...` and mirror LWC:
//! JavaScript methods defined in `<Name>Controller.js`,
//! `<Name>Helper.js`, `<Name>Renderer.js`, and any non-canonical
//! helper inside the bundle are invoked from the component's
//! `.cmp` / `.app` / `.evt` markup via attribute bindings
//! (`press="{!c.handleSave}"`, `action="{!c.init}"`, interpolations,
//! etc.). The markup parsers are a Phase C deliverable
//! (`docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` R25),
//! so most Aura JS methods that are logically reachable appear as
//! zero-fan-in in the static call graph today.
//!
//! This rule set ships as a diagnostic stub identical in shape to
//! [`super::lwc`]: any dead symbol on a file tagged `aura` is
//! stamped with [`DeadCodeReason::DeclarativeWiringUnparsed`] so
//! the verdict is accurate about *why* no caller was found,
//! rather than silently producing a `visibility_private_unused`
//! verdict that would encourage deletion of a markup-bound
//! handler. The dispatch relies on the broad segment-match
//! detector in `graphengine-parsing/src/domain/frameworks.rs`
//! (see the roadmap §13 "Framework-tag sizing rule").

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

pub struct AuraRules;

impl FrameworkRuleSet for AuraRules {
    fn framework(&self) -> &'static str {
        "aura"
    }
    fn name(&self) -> &'static str {
        "js-aura"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        if ctx.fan_in != 0 {
            return None;
        }
        Some((
            DeadCodeReason::DeclarativeWiringUnparsed,
            format!(
                "fan_in={}; symbol lives in an Aura bundle; .cmp/.app/.evt event-handler and attribute bindings are not yet parsed (FOLLOWUP_RISKS R25/R28)",
                ctx.fan_in
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aura_rule_identity_is_stable() {
        let r = AuraRules;
        assert_eq!(r.framework(), "aura");
        assert_eq!(r.name(), "js-aura");
    }
}
