//! Language- and framework-agnostic pre-rules.
//!
//! These rules run *before* any framework-keyed rule set. They
//! handle signals that are valid evidence regardless of which
//! framework (if any) is tagged on the parent file:
//!
//! 1. **Entry-point tag**: the parser's language-specific
//!    extractor (`graphengine-parsing/.../apex/entry_points.rs` and
//!    analogous Python detectors) marked the symbol with a tag like
//!    `aura_enabled` or `rest_resource`. Presence of the tag is
//!    authoritative evidence of framework dispatch, whatever the
//!    file's framework tag list looks like.
//! 2. **`is_attribute_invoked`**: generic parser flag meaning
//!    "some decorator / attribute / annotation on this symbol
//!    looked framework-shaped to the extractor". Used by Python's
//!    Celery / FastAPI / Flask / pytest decorators; also the Apex
//!    extractor's fallback tag.
//! 3. **`is_callback_target`**: symbol was passed as a method
//!    reference elsewhere. Callers are not statically tracked.
//! 4. **Parent file `is_test`**: the symbol lives in a test file
//!    (parser's `classify_path` marked the file as a test). In the
//!    dead-code pipeline, a test-file node is already partitioned
//!    into `DeadCodeResult.test`; this rule produces the
//!    `TestOnlyReference` verdict for the `reason_breakdown`.
//!
//! The rules below run in priority order. The first match wins —
//! downstream framework-keyed rules never get to re-classify a
//! node that the universal pre-pass has already identified.

use crate::health::report::DeadCodeReason;

use super::{ClassifyContext, FrameworkRuleSet};

pub struct EntryPointTagRule;
pub struct AttributeInvokedRule;
pub struct CallbackTargetRule;
pub struct ParentIsTestRule;

impl FrameworkRuleSet for EntryPointTagRule {
    fn framework(&self) -> &'static str {
        "*"
    }
    fn name(&self) -> &'static str {
        "universal-entry-point-tag"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;
        if node.entry_point_tags.is_empty() {
            return None;
        }
        Some((
            DeadCodeReason::FrameworkAnnotationUnresolved,
            format!(
                "fan_in={}; entry_point_tags={:?}; invoked outside the codebase",
                ctx.fan_in, node.entry_point_tags
            ),
        ))
    }
}

impl FrameworkRuleSet for AttributeInvokedRule {
    fn framework(&self) -> &'static str {
        "*"
    }
    fn name(&self) -> &'static str {
        "universal-attribute-invoked"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;
        if !node.is_attribute_invoked {
            return None;
        }
        Some((
            DeadCodeReason::FrameworkAnnotationUnresolved,
            format!(
                "fan_in={}; is_attribute_invoked=true; decorator/attribute/annotation dispatch not resolved by parser",
                ctx.fan_in
            ),
        ))
    }
}

impl FrameworkRuleSet for CallbackTargetRule {
    fn framework(&self) -> &'static str {
        "*"
    }
    fn name(&self) -> &'static str {
        "universal-callback-target"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let node = ctx.graph.nodes.get(ctx.node_id)?;
        if !node.is_callback_target {
            return None;
        }
        Some((
            DeadCodeReason::CallbackTargetNotTracked,
            format!(
                "fan_in={}; is_callback_target=true; passed as method reference",
                ctx.fan_in
            ),
        ))
    }
}

impl FrameworkRuleSet for ParentIsTestRule {
    fn framework(&self) -> &'static str {
        "*"
    }
    fn name(&self) -> &'static str {
        "universal-parent-is-test"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let parent_is_test = ctx
            .graph
            .classification_of(ctx.node_id)
            .map(|f| f.is_test)
            .unwrap_or(false);
        if !parent_is_test {
            return None;
        }
        Some((
            DeadCodeReason::TestOnlyReference,
            format!(
                "fan_in={}; parent file is_test=true; visible only to test execution",
                ctx.fan_in
            ),
        ))
    }
}

/// Priority-ordered list of universal pre-rules. The dispatcher
/// walks this list first and stops at the first match.
pub fn register_default() -> Vec<Box<dyn FrameworkRuleSet>> {
    vec![
        Box::new(EntryPointTagRule),
        Box::new(AttributeInvokedRule),
        Box::new(CallbackTargetRule),
        Box::new(ParentIsTestRule),
    ]
}
