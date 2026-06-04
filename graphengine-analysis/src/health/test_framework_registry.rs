//! Static lookup table of known test framework module names per language.
//!
//! Used by the structural test classification system (Tier 1) to determine
//! whether a file's import edges target a test framework. This list is small,
//! stable, and rarely changes — new testing frameworks appear roughly once per
//! year per ecosystem.

/// Returns true if `import_source` matches a known test framework module
/// for the given language.
///
/// `import_source` is the raw module specifier from the import edge
/// (e.g. `"jest"`, `"pytest"`, `"testing"`, `"@testing-library/react"`).
pub fn is_test_framework_import(language: &str, import_source: &str) -> bool {
    let normalized = import_source.trim_matches('"').trim_matches('\'');
    let lang = language.to_lowercase();

    match lang.as_str() {
        "typescript" | "javascript" => {
            JS_TS_EXACT.contains(&normalized)
                || JS_TS_PREFIXES.iter().any(|p| normalized.starts_with(p))
        }
        "python" => {
            PYTHON_EXACT.contains(&normalized)
                || PYTHON_PREFIXES.iter().any(|p| normalized.starts_with(p))
        }
        "go" => GO_EXACT.contains(&normalized),
        "rust" => false, // Rust tests use #[test] attributes, not imports
        "java" | "kotlin" => {
            JAVA_EXACT.contains(&normalized)
                || JAVA_PREFIXES.iter().any(|p| normalized.starts_with(p))
        }
        "csharp" => {
            CSHARP_EXACT.contains(&normalized)
                || CSHARP_PREFIXES.iter().any(|p| normalized.starts_with(p))
        }
        "ruby" => {
            RUBY_EXACT.contains(&normalized)
                || RUBY_PREFIXES.iter().any(|p| normalized.starts_with(p))
        }
        "swift" => SWIFT_EXACT.contains(&normalized),
        _ => false,
    }
}

/// Return all known framework module names/prefixes for a language (diagnostic use).
pub fn frameworks_for_language(language: &str) -> Vec<&'static str> {
    match language.to_lowercase().as_str() {
        "typescript" | "javascript" => {
            let mut v = JS_TS_EXACT.to_vec();
            v.extend_from_slice(JS_TS_PREFIXES);
            v
        }
        "python" => {
            let mut v = PYTHON_EXACT.to_vec();
            v.extend_from_slice(PYTHON_PREFIXES);
            v
        }
        "go" => GO_EXACT.to_vec(),
        "java" | "kotlin" => {
            let mut v = JAVA_EXACT.to_vec();
            v.extend_from_slice(JAVA_PREFIXES);
            v
        }
        _ => vec![],
    }
}

// ── TypeScript / JavaScript ───────────────────────────────────────────

static JS_TS_EXACT: &[&str] = &[
    "jest",
    "@jest/globals",
    "vitest",
    "mocha",
    "chai",
    "sinon",
    "cypress",
    "supertest",
    "nock",
    "msw",
    "jest-mock-extended",
    "expect",
    "node:test",
    "node:assert",
    "ava",
    "tap",
    "tape",
    "@playwright/test",
];

static JS_TS_PREFIXES: &[&str] = &["@testing-library/", "@jest/"];

// ── Python ────────────────────────────────────────────────────────────

static PYTHON_EXACT: &[&str] = &[
    "pytest",
    "unittest",
    "unittest.mock",
    "mock",
    "nose",
    "nose2",
    "hypothesis",
    "responses",
    "httpretty",
    "factory_boy",
    "faker",
    "freezegun",
    "pytest_mock",
];

static PYTHON_PREFIXES: &[&str] = &["pytest.", "unittest."];

// ── Go ────────────────────────────────────────────────────────────────

static GO_EXACT: &[&str] = &[
    "testing",
    "github.com/stretchr/testify",
    "github.com/stretchr/testify/assert",
    "github.com/stretchr/testify/require",
    "github.com/stretchr/testify/suite",
    "github.com/stretchr/testify/mock",
    "github.com/onsi/ginkgo",
    "github.com/onsi/gomega",
];

// ── Java / Kotlin ─────────────────────────────────────────────────────

static JAVA_EXACT: &[&str] = &[
    "org.junit.Test",
    "org.junit.Assert",
    "org.junit.jupiter.api.Test",
    "org.testng.annotations.Test",
];

static JAVA_PREFIXES: &[&str] = &[
    "org.junit.",
    "junit.",
    "org.testng.",
    "org.mockito.",
    "org.assertj.",
    "org.hamcrest.",
    "io.mockk.",
    "org.springframework.boot.test.",
    "org.springframework.test.",
];

// ── C# ────────────────────────────────────────────────────────────────

static CSHARP_EXACT: &[&str] = &[
    "Xunit",
    "Moq",
    "FluentAssertions",
    "NSubstitute",
    "Shouldly",
    "Microsoft.VisualStudio.TestTools.UnitTesting",
];

static CSHARP_PREFIXES: &[&str] = &["Xunit.", "NUnit."];

// ── Ruby ──────────────────────────────────────────────────────────────

static RUBY_EXACT: &[&str] = &["rspec", "minitest", "test-unit"];

static RUBY_PREFIXES: &[&str] = &["rspec/", "minitest/"];

// ── Swift ─────────────────────────────────────────────────────────────

static SWIFT_EXACT: &[&str] = &["XCTest"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_jest_detected() {
        assert!(is_test_framework_import("typescript", "jest"));
        assert!(is_test_framework_import("javascript", "jest"));
        assert!(is_test_framework_import("typescript", "@jest/globals"));
    }

    #[test]
    fn js_testing_library_prefix() {
        assert!(is_test_framework_import(
            "typescript",
            "@testing-library/react"
        ));
        assert!(is_test_framework_import(
            "javascript",
            "@testing-library/jest-dom"
        ));
    }

    #[test]
    fn js_non_test_module_not_detected() {
        assert!(!is_test_framework_import("typescript", "express"));
        assert!(!is_test_framework_import("javascript", "lodash"));
    }

    #[test]
    fn python_pytest_detected() {
        assert!(is_test_framework_import("python", "pytest"));
        assert!(is_test_framework_import("python", "unittest"));
        assert!(is_test_framework_import("python", "unittest.mock"));
    }

    #[test]
    fn python_pytest_prefix() {
        assert!(is_test_framework_import("python", "pytest.fixtures"));
    }

    #[test]
    fn python_non_test_module_not_detected() {
        assert!(!is_test_framework_import("python", "requests"));
        assert!(!is_test_framework_import("python", "flask"));
    }

    #[test]
    fn go_testing_detected() {
        assert!(is_test_framework_import("go", "testing"));
        assert!(is_test_framework_import(
            "go",
            "github.com/stretchr/testify/assert"
        ));
    }

    #[test]
    fn go_non_test_module_not_detected() {
        assert!(!is_test_framework_import("go", "net/http"));
        assert!(!is_test_framework_import("go", "github.com/go-chi/chi"));
    }

    #[test]
    fn rust_always_false() {
        assert!(!is_test_framework_import("rust", "test"));
        assert!(!is_test_framework_import("rust", "anything"));
    }

    #[test]
    fn java_junit_detected() {
        assert!(is_test_framework_import("java", "org.junit.Test"));
        assert!(is_test_framework_import(
            "kotlin",
            "org.junit.jupiter.api.Test"
        ));
        assert!(is_test_framework_import("java", "org.mockito.Mockito"));
    }

    #[test]
    fn csharp_xunit_detected() {
        assert!(is_test_framework_import("csharp", "Xunit"));
        assert!(is_test_framework_import("csharp", "Xunit.Assert"));
        assert!(is_test_framework_import("csharp", "NUnit.Framework"));
    }

    #[test]
    fn quoted_import_sources_handled() {
        assert!(is_test_framework_import("typescript", "\"jest\""));
        assert!(is_test_framework_import("python", "'pytest'"));
    }

    #[test]
    fn frameworks_list_nonempty_for_supported_languages() {
        assert!(!frameworks_for_language("typescript").is_empty());
        assert!(!frameworks_for_language("python").is_empty());
        assert!(!frameworks_for_language("go").is_empty());
        assert!(frameworks_for_language("unknown").is_empty());
    }
}
