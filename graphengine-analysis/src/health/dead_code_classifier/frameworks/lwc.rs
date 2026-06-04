//! Lightning Web Components (LWC) rule set.
//!
//! LWC bundles are `.../lwc/<component>/<component>.{html,js}`.
//! JavaScript methods are invoked from the component's HTML
//! template (`onclick={handleClick}`, `{expression}` interpolations,
//! `@api` fields bound from parent components). The HTML templates
//! are not currently parsed, so most LWC JS methods that are
//! logically reachable appear as `no_callers` in the static graph.
//!
//! Because parsing LWC templates is an entire new workstream
//! (R25 in `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md`),
//! this rule set deliberately ships as a **diagnostic stub**: it
//! stamps `DeclarativeWiringUnparsed` on any dead LWC symbol so the
//! verdict is accurate about *why* no caller was found, rather than
//! silently claiming the method has "no callers" (which would
//! encourage dead-code removal of methods that are in fact wired
//! from HTML).

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

pub struct LwcRules;

impl FrameworkRuleSet for LwcRules {
    fn framework(&self) -> &'static str {
        "lwc"
    }
    fn name(&self) -> &'static str {
        "js-lwc"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        if ctx.fan_in != 0 {
            return None;
        }
        Some((
            DeadCodeReason::DeclarativeWiringUnparsed,
            format!(
                "fan_in={}; symbol lives in LWC bundle; HTML template bindings are not yet parsed (FOLLOWUP_RISKS R25)",
                ctx.fan_in
            ),
        ))
    }
}
