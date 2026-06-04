//! Plain (framework-neutral) rule set.
//!
//! Files tagged `plain` have no framework-specific dispatch
//! contract beyond what the language itself provides. The
//! framework-keyed dispatcher still runs this rule set so polyglot
//! repos get a consistent attribution string on the verdict (rather
//! than letting the terminal `GenericClassifier` fallback silently
//! handle every plain file).
//!
//! In practice `PlainRules` intentionally returns `None` for every
//! node; the registry then falls through to the generic
//! visibility-based classifier. Keeping this as its own module
//! (instead of a no-op inside the dispatcher) makes two things
//! explicit in the codebase:
//!
//! 1. `plain` is a first-class framework with an owner, not a
//!    silent default. Anyone adding per-language default rules
//!    (e.g. TypeScript `export` reachability) has an obvious home.
//! 2. Hand-audit tooling can count verdicts attributed to
//!    `plain-fallback` vs. a specific framework without re-deriving
//!    which case is which from the reason breakdown.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

pub struct PlainRules;

impl FrameworkRuleSet for PlainRules {
    fn framework(&self) -> &'static str {
        "plain"
    }
    fn name(&self) -> &'static str {
        "plain"
    }
    fn classify(&self, _ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        // No plain-specific heuristics. The terminal
        // `GenericClassifier` owns visibility-based verdicts.
        None
    }
}
