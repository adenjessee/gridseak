//! Shared helpers for test-runner harness rule sets
//! (Jest, Vitest, and any future `frameworks/<runner>.rs` module).
//!
//! Jest and Vitest share a setup / config file contract today —
//! functions declared at the top level of `<runner>.setup.*` and
//! `<runner>.config.*` are invoked by the runner during suite
//! bootstrap, not by user code. Their classifier verdicts therefore
//! share evidence wording, and the path-lookup glue is identical.
//!
//! Per the roadmap §13 "Framework-tag sizing rule", Jest and
//! Vitest keep distinct framework tags (and distinct rule
//! modules) so classifier attribution stays honest and future
//! runner-specific behaviour (e.g. Vitest's `vi.mock` module-level
//! side effects) has a natural home. Everything truly common —
//! the path accessor and the evidence template — lives here so
//! "if it changes for one runner, it changes for both unless you
//! explicitly opt out."
//!
//! Visibility is `pub(super)` deliberately. Cross-module API
//! surface for the classifier is intentionally small; only the
//! sibling rule modules under `frameworks/` may reach in here.

use super::super::ClassifyContext;

/// Look up the repo-relative path of the parent file for a
/// classified node. Returns `None` when the node has no
/// classification info (e.g. a synthetic symbol introduced by
/// extractor glue). Preferred over `file_path` because
/// `path_repo_rel` is stable across workspace roots.
pub(super) fn parent_path(ctx: &ClassifyContext<'_>) -> Option<String> {
    let p = ctx.graph.classification_of(ctx.node_id)?;
    p.path_repo_rel.clone().or_else(|| p.file_path.clone())
}

/// Produce the canonical evidence clause for a dead symbol that
/// the runner will invoke during suite bootstrap.
///
/// Shape: `"fan_in={N}; function in {path}; invoked by the {Runner}
/// runner during suite bootstrap (not by user code)"`.
///
/// The caller supplies the display name of the runner
/// (`"Jest"`, `"Vitest"`, …). Using a display-cased constant
/// rather than deriving one from the framework tag keeps the
/// customer-facing evidence stable even if a tag string ever
/// needs to change.
pub(super) fn runner_entry_point_evidence(
    runner: &'static str,
    path: &str,
    fan_in: usize,
) -> String {
    format!(
        "fan_in={}; function in {}; invoked by the {} runner during suite bootstrap (not by user code)",
        fan_in, path, runner
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_clause_mentions_runner_path_and_fanin() {
        let ev = runner_entry_point_evidence("Jest", "jest.setup.js", 0);
        assert!(ev.contains("fan_in=0"));
        assert!(ev.contains("jest.setup.js"));
        assert!(ev.contains("Jest"));
        assert!(ev.contains("runner"));
    }

    #[test]
    fn evidence_clause_differentiates_runners() {
        let j = runner_entry_point_evidence("Jest", "jest.setup.js", 0);
        let v = runner_entry_point_evidence("Vitest", "vitest.setup.ts", 0);
        assert!(j.contains("Jest") && !j.contains("Vitest"));
        assert!(v.contains("Vitest") && !v.contains("Jest"));
    }
}
