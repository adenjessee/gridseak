//! Apex `.trigger` file rule set.
//!
//! Apex triggers are declared as `.trigger` files and dispatched by
//! the Salesforce platform on DML events. The trigger body and any
//! methods it defines (in anonymous blocks) are never called in
//! Apex code, so the static call graph inherently lacks an in-repo
//! caller. File-level detection (`graphengine-parsing/src/domain/
//! frameworks.rs::detect_frameworks_by_path`) emits the
//! `triggerdml` framework tag for these files.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};

pub struct TriggerDmlRules;

impl FrameworkRuleSet for TriggerDmlRules {
    fn framework(&self) -> &'static str {
        "triggerdml"
    }
    fn name(&self) -> &'static str {
        "apex-triggerdml"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        // File-framework tag alone is sufficient evidence. Symbols
        // inside a .trigger file are invoked by the platform DML
        // pipeline unconditionally.
        Some((
            DeadCodeReason::FrameworkAnnotationUnresolved,
            format!(
                "fan_in={}; file tagged `triggerdml` (.trigger file); invoked by Salesforce platform DML event",
                ctx.fan_in
            ),
        ))
    }
}
