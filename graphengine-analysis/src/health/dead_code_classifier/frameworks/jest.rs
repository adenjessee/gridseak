//! Jest test-runner harness rule set.
//!
//! The path detector in `graphengine-parsing/src/domain/frameworks.rs`
//! tags `jest.setup.{js,ts,mjs,cjs}` and
//! `jest.config.{js,ts,mjs,cjs}` files with the framework tag
//! `jest`. Functions declared inside those files are invoked by
//! the Jest runner during suite bootstrap — global setup, custom
//! environment factories, `beforeAll` chains wired via the setup
//! file, test-match predicates exported from config, etc. — not
//! by user code. From the static call graph's point of view they
//! look like `fan_in=0` dead code; semantically they are
//! framework entry points.
//!
//! Verdict: [`DeadCodeReason::FrameworkAnnotationUnresolved`].
//! The shape matches [`super::celery`] and [`super::triggerdml`]
//! ("invoked outside the codebase"). It deliberately does NOT
//! return [`DeadCodeReason::TestOnlyReference`]: that reason is
//! for symbols whose *callers* are test files, which is a
//! different shape.
//!
//! Evidence wording is produced by
//! [`super::test_harness_common::runner_entry_point_evidence`]
//! so Jest and Vitest share the setup-contract clause verbatim
//! (the shared helper is the single place that clause is
//! authored). Rationale for keeping Jest and Vitest as distinct
//! framework tags + rule modules is documented in the roadmap
//! §13 "Framework-tag sizing rule".

use crate::health::report::DeadCodeReason;

use super::super::{ClassifyContext, FrameworkRuleSet};
use super::test_harness_common;

pub struct JestRules;

impl FrameworkRuleSet for JestRules {
    fn framework(&self) -> &'static str {
        "jest"
    }
    fn name(&self) -> &'static str {
        "js-jest"
    }
    fn classify(&self, ctx: &ClassifyContext<'_>) -> Option<(DeadCodeReason, String)> {
        // File-framework tag alone is sufficient evidence. The
        // detector only stamps `jest` on the narrow set of
        // setup/config filenames, so any dead symbol inside one
        // of those files is a runner-invoked entry point.
        let path = test_harness_common::parent_path(ctx).unwrap_or_default();
        Some((
            DeadCodeReason::FrameworkAnnotationUnresolved,
            test_harness_common::runner_entry_point_evidence("Jest", &path, ctx.fan_in),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jest_rule_identity_is_stable() {
        let r = JestRules;
        assert_eq!(r.framework(), "jest");
        assert_eq!(r.name(), "js-jest");
    }
}
