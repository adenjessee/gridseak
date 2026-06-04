//! Vitest test-runner harness rule set.
//!
//! Vitest's `vitest.setup.{js,ts,mjs,cjs}` and
//! `vitest.config.{js,ts,mjs,cjs}` share the bootstrap contract
//! with Jest: functions declared at the top level are invoked by
//! the runner during suite setup, not by user code. This rule
//! module is therefore structurally identical to [`super::jest`];
//! only the framework tag, classifier attribution, and the runner
//! label that surfaces in evidence strings differ.
//!
//! **Why two tags, not one.** The roadmap §13 "Framework-tag
//! sizing rule" treats runner identity as the contract for
//! test-runner harnesses: Jest and Vitest share the setup
//! contract today but diverge on module-mocking semantics
//! (`vi.mock` has module-level side effects that can turn mocked
//! exports into entry points), snapshot layout, and runner-side
//! globals. A future rule that keys on those behaviours belongs
//! in `vitest.rs` alone; that rule is a one-line addition with
//! two tags, and a conditional escape hatch inside a
//! hypothetical merged `js-test-harness.rs` with one tag — the
//! anti-pattern we eliminated in Wave 2. Customer-facing
//! attribution (`reason_breakdown` columns `jest` and `vitest`)
//! stays honest for the same reason.

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};
use super::test_harness_common;

pub struct VitestRules;

impl FrameworkRuleSet for VitestRules {
    fn framework(&self) -> &'static str {
        "vitest"
    }
    fn name(&self) -> &'static str {
        "js-vitest"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        let path = test_harness_common::parent_path(ctx).unwrap_or_default();
        Some((
            DeadCodeReason::FrameworkAnnotationUnresolved,
            test_harness_common::runner_entry_point_evidence("Vitest", &path, ctx.fan_in),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vitest_rule_identity_is_stable() {
        let r = VitestRules;
        assert_eq!(r.framework(), "vitest");
        assert_eq!(r.name(), "js-vitest");
    }
}
