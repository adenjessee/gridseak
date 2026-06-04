//! Shared path-based classification for non-production code.
//!
//! Uses **word-boundary matching** instead of exhaustive directory name lists.
//! A "word boundary" here means a delimiter (`-`, `_`, `.`) or string start/end.
//! This catches `runtime-tests`, `unit-tests`, `perf_tests`, `e2e-test`, etc.
//! without matching substrings like `contest`, `testimony`, or `attest`.
//!
//! Three categories:
//! - **Test:** directories/files whose names contain `test`/`tests`/`spec`/`specs`
//!   at a word boundary, or special dunder directories (`__tests__`, `__mocks__`).
//! - **Auxiliary:** directories whose names contain `example`/`examples`, `bench`/
//!   `benchmark`, `fixture`/`mock`/`stub`, `demo`, `sample`, `docs`/`doc` at word boundaries.
//! - **Tooling/config:** dotfile directories (`.vitest`, `.jest`, `.storybook`).

/// Segments in a file path that indicate test code (exact substring match).
/// These are dunder-style directories that are unambiguous.
const TEST_DIR_SEGMENTS: &[&str] = &["__tests__", "__test__", "__mocks__"];

/// File name infix/suffix patterns that indicate test files.
/// Matched against the file name portion (not directory), case-insensitively.
const TEST_FILE_PATTERNS: &[&str] = &[".test.", ".spec.", "_test.", ".tests."];

/// Words that indicate test directories when they appear at a word boundary
/// in any path component. Delimiters: `-`, `_`, `.`, start/end of string.
const TEST_BOUNDARY_WORDS: &[&str] = &["test", "tests", "spec", "specs"];

/// Words that indicate auxiliary (non-production) directories at word boundaries.
const AUXILIARY_BOUNDARY_WORDS: &[&str] = &[
    "example",
    "examples",
    "benchmark",
    "benchmarks",
    "bench",
    "benches",
    "fixture",
    "fixtures",
    "mock",
    "mocks",
    "stub",
    "stubs",
    "demo",
    "demos",
    "sample",
    "samples",
    "docs",
    "doc",
    "e2e",
    "testdata",
    "perf",
    "performance",
    "scripts",
];

/// Dotfile directory prefixes that indicate tooling/config (not production code).
const TOOLING_DOTFILE_PREFIXES: &[&str] = &[
    ".vitest",
    ".jest",
    ".storybook",
    ".husky",
    ".cypress",
    ".playwright",
    ".nyc",
    ".coverage",
];

/// Check if `word` appears at a word boundary within `component`.
/// Word boundaries are: start/end of string, `-`, `_`, `.`.
fn contains_word_boundary(component: &str, word: &str) -> bool {
    if component == word {
        return true;
    }
    let comp_bytes = component.as_bytes();
    let word_bytes = word.as_bytes();
    let wlen = word_bytes.len();
    if wlen > comp_bytes.len() {
        return false;
    }
    for start in 0..=(comp_bytes.len() - wlen) {
        if &comp_bytes[start..start + wlen] != word_bytes {
            continue;
        }
        let at_left_boundary = start == 0 || matches!(comp_bytes[start - 1], b'-' | b'_' | b'.');
        let end = start + wlen;
        let at_right_boundary =
            end == comp_bytes.len() || matches!(comp_bytes[end], b'-' | b'_' | b'.');
        if at_left_boundary && at_right_boundary {
            return true;
        }
    }
    false
}

/// Returns `true` if the path matches common test file conventions.
///
/// Checks dunder directories (`__tests__/`), file-name patterns (`.test.ts`,
/// `_test.go`, `test_foo.py`), and word-boundary matching on path components
/// (catches `runtime-tests`, `unit-tests`, `perf_tests`, etc.).
pub fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let normalized = lower.replace('\\', "/");

    // Dunder directories: always unambiguous
    for seg in TEST_DIR_SEGMENTS {
        if normalized.contains(seg) {
            return true;
        }
    }

    // File name patterns
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    for pattern in TEST_FILE_PATTERNS {
        if file_name.contains(pattern) {
            return true;
        }
    }

    // Python convention: file starts with test_ (only applies to file name)
    if file_name.starts_with("test_") {
        return true;
    }

    // Word-boundary matching on each path component
    for component in normalized.split('/') {
        if component.is_empty() {
            continue;
        }
        for word in TEST_BOUNDARY_WORDS {
            if contains_word_boundary(component, word) {
                return true;
            }
        }
    }

    false
}

/// Returns `true` if the path is inside an auxiliary directory (examples, benchmarks,
/// fixtures, demos, docs, mocks, stubs). Uses word-boundary matching.
pub fn is_auxiliary_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let normalized = lower.replace('\\', "/");

    for component in normalized.split('/') {
        if component.is_empty() {
            continue;
        }

        // Dotfile tooling directories
        for prefix in TOOLING_DOTFILE_PREFIXES {
            if component == *prefix || component.starts_with(&format!("{prefix}.")) {
                return true;
            }
        }

        // Word-boundary matching for auxiliary words
        for word in AUXILIARY_BOUNDARY_WORDS {
            if contains_word_boundary(component, word) {
                return true;
            }
        }
    }

    false
}

/// Returns `true` if the path looks like a config/tooling file that isn't
/// production code. Catches dotfile configs, `*.config.*`, and known tooling files.
pub fn is_config_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let normalized = lower.replace('\\', "/");
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);

    // Dotfiles at project root: .prettierrc.js, .eslintrc.js, etc.
    if file_name.starts_with('.')
        && (file_name.ends_with(".js")
            || file_name.ends_with(".ts")
            || file_name.ends_with(".json")
            || file_name.ends_with(".yaml")
            || file_name.ends_with(".yml")
            || file_name.ends_with(".toml"))
    {
        return true;
    }

    // *.config.* pattern: jest.config.js, vitest.config.ts, eslint.config.js, etc.
    if file_name.contains(".config.") {
        return true;
    }

    // Known tool config files
    const KNOWN_CONFIG_FILES: &[&str] = &[
        "tsconfig.json",
        "jsconfig.json",
        "package.json",
        "package-lock.json",
        "gruntfile.js",
        "gulpfile.js",
        "rollup.config.js",
        "webpack.config.js",
        "babel.config.js",
        "postcss.config.js",
        "tailwind.config.js",
        "karma.conf.js",
    ];
    KNOWN_CONFIG_FILES.contains(&file_name)
}

/// Returns `true` if the path is non-production code (test OR auxiliary OR config).
/// Use this for broad exclusion across metrics.
pub fn is_non_production_path(path: &str) -> bool {
    is_test_path(path) || is_auxiliary_path(path) || is_config_path(path)
}

/// Classify a path into a role string for the report.
/// Returns `None` if the path is classified as production.
pub fn classify_path(path: &str) -> Option<(&'static str, &'static str)> {
    if is_test_path(path) {
        Some(("test", "path heuristic: test pattern detected"))
    } else if is_config_path(path) {
        Some(("config", "path heuristic: config/tooling file"))
    } else if is_auxiliary_path(path) {
        let lower = path.to_lowercase();
        let normalized = lower.replace('\\', "/");
        for component in normalized.split('/') {
            for word in &["example", "examples"] {
                if contains_word_boundary(component, word) {
                    return Some(("example", "path heuristic: example directory"));
                }
            }
            for word in &["bench", "benches", "benchmark", "benchmarks"] {
                if contains_word_boundary(component, word) {
                    return Some(("benchmark", "path heuristic: benchmark directory"));
                }
            }
            for word in &["fixture", "fixtures"] {
                if contains_word_boundary(component, word) {
                    return Some(("fixture", "path heuristic: fixture directory"));
                }
            }
            for word in &["mock", "mocks", "stub", "stubs"] {
                if contains_word_boundary(component, word) {
                    return Some(("fixture", "path heuristic: mock/stub directory"));
                }
            }
            for word in &["docs", "doc"] {
                if contains_word_boundary(component, word) {
                    return Some(("docs", "path heuristic: documentation directory"));
                }
            }
            for prefix in TOOLING_DOTFILE_PREFIXES {
                if component == *prefix || component.starts_with(&format!("{prefix}.")) {
                    return Some(("config", "path heuristic: tooling config directory"));
                }
            }
        }
        Some(("auxiliary", "path heuristic: auxiliary directory"))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // contains_word_boundary
    // -----------------------------------------------------------------------

    #[test]
    fn word_boundary_exact_match() {
        assert!(contains_word_boundary("test", "test"));
        assert!(contains_word_boundary("tests", "tests"));
    }

    #[test]
    fn word_boundary_hyphen_delimited() {
        assert!(contains_word_boundary("runtime-tests", "tests"));
        assert!(contains_word_boundary("e2e-test", "test"));
        assert!(contains_word_boundary("unit-tests", "tests"));
        assert!(contains_word_boundary("perf-test", "test"));
    }

    #[test]
    fn word_boundary_underscore_delimited() {
        assert!(contains_word_boundary("integration_tests", "tests"));
        assert!(contains_word_boundary("unit_test", "test"));
    }

    #[test]
    fn word_boundary_dot_delimited() {
        assert!(contains_word_boundary(".test.config", "test"));
    }

    #[test]
    fn word_boundary_rejects_substrings() {
        assert!(!contains_word_boundary("contest", "test"));
        assert!(!contains_word_boundary("testimony", "test"));
        assert!(!contains_word_boundary("attest", "test"));
        assert!(!contains_word_boundary("protest", "test"));
        assert!(!contains_word_boundary("detest", "test"));
        assert!(!contains_word_boundary("testable", "test"));
    }

    #[test]
    fn word_boundary_prefix_position() {
        assert!(contains_word_boundary("test-utils", "test"));
        assert!(contains_word_boundary("test_helpers", "test"));
    }

    // -----------------------------------------------------------------------
    // is_test_path: existing true cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_file_with_dot_test_infix() {
        assert!(is_test_path("src/utils/url.test.ts"));
        assert!(is_test_path("src/utils/url.test.tsx"));
        assert!(is_test_path("src/utils/url.test.js"));
    }

    #[test]
    fn spec_file_with_dot_spec_infix() {
        assert!(is_test_path("src/auth/login.spec.ts"));
        assert!(is_test_path("lib/helper.spec.js"));
    }

    #[test]
    fn go_test_file() {
        assert!(is_test_path("pkg/router_test.go"));
        assert!(is_test_path("internal/handler_test.go"));
    }

    #[test]
    fn python_test_file_prefix() {
        assert!(is_test_path("tests/test_auth.py"));
        assert!(is_test_path("test_utils.py"));
    }

    #[test]
    fn python_test_file_suffix() {
        assert!(is_test_path("auth_test.py"));
    }

    #[test]
    fn dunder_tests_directory() {
        assert!(is_test_path("src/__tests__/auth.ts"));
        assert!(is_test_path("components/__tests__/Button.test.tsx"));
    }

    #[test]
    fn tests_directory() {
        assert!(is_test_path("tests/integration.rs"));
        assert!(is_test_path("tests/e2e/login.ts"));
    }

    #[test]
    fn test_directory_singular() {
        assert!(is_test_path("test/helpers/mock_db.ts"));
    }

    #[test]
    fn spec_directory() {
        assert!(is_test_path("spec/models/user_spec.rb"));
    }

    #[test]
    fn case_insensitive() {
        assert!(is_test_path("src/Utils/URL.Test.ts"));
        assert!(is_test_path("Tests/Integration.rs"));
    }

    // -----------------------------------------------------------------------
    // is_test_path: NEW word-boundary cases
    // -----------------------------------------------------------------------

    #[test]
    fn hyphenated_test_dirs() {
        assert!(is_test_path("runtime-tests/fastly/index.ts"));
        assert!(is_test_path("runtime-tests/node/server.ts"));
        assert!(is_test_path("unit-tests/auth.test.ts"));
        assert!(is_test_path("e2e-test/login.ts"));
        assert!(is_test_path("perf-tests/benchmark.go"));
        assert!(is_test_path("acceptance-tests/flow.py"));
        assert!(is_test_path("load-test/scenario.ts"));
    }

    #[test]
    fn underscored_test_dirs() {
        assert!(is_test_path("integration_tests/api.rs"));
        assert!(is_test_path("unit_test/helper.go"));
    }

    // -----------------------------------------------------------------------
    // is_test_path: false cases (production code)
    // -----------------------------------------------------------------------

    #[test]
    fn production_file_with_test_in_name() {
        assert!(!is_test_path("src/testable/widget.ts"));
    }

    #[test]
    fn test_runner_implementation() {
        // test_runner starts with "test_" in the file name — accepted false positive.
        assert!(is_test_path("src/test_runner.py"));
    }

    #[test]
    fn regular_production_files() {
        assert!(!is_test_path("src/auth/login.ts"));
        assert!(!is_test_path("src/database/users.ts"));
        assert!(!is_test_path("lib/utils.py"));
        assert!(!is_test_path("internal/router.go"));
        assert!(!is_test_path("src/main.rs"));
    }

    #[test]
    fn contest_or_protest_not_matched() {
        assert!(!is_test_path("src/contest/entry.ts"));
        assert!(!is_test_path("src/protest/handler.py"));
    }

    #[test]
    fn testimony_detest_attest_not_matched() {
        assert!(!is_test_path("src/testimony/report.ts"));
        assert!(!is_test_path("src/detest/processor.py"));
        assert!(!is_test_path("src/attest/validator.go"));
    }

    // -----------------------------------------------------------------------
    // is_auxiliary_path
    // -----------------------------------------------------------------------

    #[test]
    fn example_directories() {
        assert!(is_auxiliary_path("examples/cookie-sessions/index.js"));
        assert!(is_auxiliary_path("_examples/custom-handler/main.go"));
        assert!(is_auxiliary_path("example/basic/app.ts"));
    }

    #[test]
    fn benchmark_directories() {
        assert!(is_auxiliary_path("benchmarks/parse_bench.rs"));
        assert!(is_auxiliary_path("benchmark/throughput.go"));
        assert!(is_auxiliary_path("benches/parsing_benchmarks.rs"));
        assert!(is_auxiliary_path("graphengine-parsing/benches/foo.rs"));
    }

    #[test]
    fn fixture_directories() {
        assert!(is_auxiliary_path("fixtures/sample_data.json"));
        assert!(is_auxiliary_path("fixture/mock_response.ts"));
    }

    #[test]
    fn demo_directories() {
        assert!(is_auxiliary_path("demo/app.ts"));
        assert!(is_auxiliary_path("demos/basic/index.html"));
    }

    #[test]
    fn sample_directories() {
        assert!(is_auxiliary_path("samples/quickstart/main.go"));
        assert!(is_auxiliary_path("sample/hello.py"));
    }

    #[test]
    fn docs_directory() {
        assert!(is_auxiliary_path("docs/api/auth.md"));
        assert!(is_auxiliary_path("doc/architecture.rs"));
    }

    #[test]
    fn dotfile_tooling_directories() {
        assert!(is_auxiliary_path(".vitest/cache/data.json"));
        assert!(is_auxiliary_path(".jest/cache.json"));
        assert!(is_auxiliary_path(".storybook/main.ts"));
        assert!(is_auxiliary_path(".husky/pre-commit"));
    }

    #[test]
    fn not_auxiliary_production_code() {
        assert!(!is_auxiliary_path("src/auth/login.ts"));
        assert!(!is_auxiliary_path("lib/utils.py"));
        assert!(!is_auxiliary_path("internal/handler.go"));
        assert!(!is_auxiliary_path("pkg/router.go"));
    }

    // -----------------------------------------------------------------------
    // is_non_production_path
    // -----------------------------------------------------------------------

    #[test]
    fn non_production_combines_test_and_auxiliary() {
        assert!(is_non_production_path("tests/auth.test.ts"));
        assert!(is_non_production_path("examples/basic/app.ts"));
        assert!(is_non_production_path("benchmarks/perf.rs"));
        assert!(is_non_production_path("runtime-tests/node/index.ts"));
        assert!(!is_non_production_path("src/auth/login.ts"));
    }

    // -----------------------------------------------------------------------
    // classify_path
    // -----------------------------------------------------------------------

    #[test]
    fn classify_test_path() {
        let (role, _) = classify_path("tests/auth.test.ts").unwrap();
        assert_eq!(role, "test");
    }

    #[test]
    fn classify_example_path() {
        let (role, _) = classify_path("examples/basic/app.ts").unwrap();
        assert_eq!(role, "example");
    }

    #[test]
    fn classify_benchmark_path() {
        let (role, _) = classify_path("benchmarks/perf.rs").unwrap();
        assert_eq!(role, "benchmark");
    }

    #[test]
    fn classify_benches_path() {
        let (role, _) = classify_path("benches/parsing_benchmarks.rs").unwrap();
        assert_eq!(role, "benchmark");
        let (role, _) = classify_path("graphengine-parsing/benches/foo.rs").unwrap();
        assert_eq!(role, "benchmark");
    }

    #[test]
    fn classify_production_path() {
        assert!(classify_path("src/auth/login.ts").is_none());
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn empty_path() {
        assert!(!is_test_path(""));
        assert!(!is_auxiliary_path(""));
        assert!(!is_non_production_path(""));
    }

    #[test]
    fn root_file() {
        assert!(!is_test_path("main.rs"));
        assert!(!is_auxiliary_path("main.rs"));
    }

    #[test]
    fn backslash_paths_normalized() {
        assert!(is_test_path("src\\__tests__\\auth.ts"));
        assert!(is_auxiliary_path("examples\\basic\\app.ts"));
    }

    #[test]
    fn vitest_config_at_root() {
        // Module key for a root-level .vitest.config.ts
        assert!(is_auxiliary_path(".vitest.config"));
    }
}
