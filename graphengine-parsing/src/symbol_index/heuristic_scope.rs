//! Heuristic-resolution scope rules (Q3-followon).
//!
//! Tree-sitter + grep fallback resolution can match generic method names
//! (`.get()`, `.clone()`) across crate boundaries. When the workspace
//! includes dev-only crates such as `graphengine-parsing-test-support`,
//! production call sites can spuriously resolve into mock types — inflating
//! fan-in counts and creating fake SCCs in cycle detection.
//!
//! This module classifies file paths so **production callers never
//! heuristic-resolve into non-production callees**. LSP-verified edges
//! (Tier 3) are unaffected; this gates only the SimpleName / FqnSuffix
//! candidate pools inside [`super::SymbolIndex`].

/// True when `path` looks like test, benchmark, fixture, or dev-only
/// test-support crate code — i.e. not a heuristic-resolution target
/// for production callers.
pub fn is_non_production_heuristic_target(path: &str) -> bool {
    is_likely_test_file(path) || is_test_support_crate_path(path) || is_likely_auxiliary_path(path)
}

/// True when the caller context is allowed to resolve into mock /
/// test-support symbols (test files, benches, the test-support crate).
pub fn is_non_production_heuristic_context(path: &str) -> bool {
    is_non_production_heuristic_target(path)
}

/// Filter callee candidates for heuristic resolution. Production
/// contexts drop non-production targets; test contexts keep everything.
pub fn filter_heuristic_callees<T>(
    context_file: &str,
    candidates: Vec<T>,
    file_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    if is_non_production_heuristic_context(context_file) {
        return candidates;
    }
    candidates
        .into_iter()
        .filter(|c| !is_non_production_heuristic_target(file_of(c)))
        .collect()
}

fn is_likely_test_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.contains("/test_")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/__tests__/")
        || lower.contains("/__test__/")
        || lower.contains("/__mocks__/")
}

/// Dev-only mock crates shippable as workspace members (R2 moved mocks
/// here specifically so production builds don't link them — but scans
/// still parse the crate and its `.get()` / `.clone()` pollute the graph).
fn is_test_support_crate_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("graphengine-parsing-test-support")
        || lower.contains("-test-support/")
        || lower.contains("/test-support/")
        || lower.contains("parsing-test-support/")
}

/// Benches, fixtures, examples — same trust contract as analysis
/// `path_classification` auxiliary bucket (kept local to avoid a
/// parsing → analysis dependency).
fn is_likely_auxiliary_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("/benches/")
        || lower.contains("/bench/")
        || lower.contains("/examples/")
        || lower.contains("/example/")
        || lower.contains("/fixtures/")
        || lower.contains("/fixture/")
        || lower.contains("/testdata/")
        || lower.contains("/e2e/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_support_crate_is_non_production() {
        assert!(is_non_production_heuristic_target(
            "/repo/graphengine-parsing-test-support/src/mock_lsp_session.rs"
        ));
    }

    #[test]
    fn production_src_is_not_non_production() {
        assert!(!is_non_production_heuristic_target(
            "/repo/graphengine-parsing/src/infrastructure/lsp/resolver.rs"
        ));
    }

    #[test]
    fn production_context_filters_test_support_callees() {
        let candidates = vec![
            ("prod", "/repo/graphengine-parsing/src/lib.rs"),
            (
                "mock",
                "/repo/graphengine-parsing-test-support/src/mocks.rs",
            ),
        ];
        let filtered = filter_heuristic_callees(
            "/repo/graphengine-parsing/src/lib.rs",
            candidates,
            |(_, path)| *path,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "prod");
    }

    #[test]
    fn test_context_keeps_test_support_callees() {
        let candidates = vec![(
            "mock",
            "/repo/graphengine-parsing-test-support/src/mocks.rs",
        )];
        let filtered = filter_heuristic_callees(
            "/repo/graphengine-parsing/tests/foo.rs",
            candidates,
            |(_, path)| *path,
        );
        assert_eq!(filtered.len(), 1);
    }
}
