//! Language-scoped reserved-keyword filter for symbol extraction.
//!
//! # Why this exists
//!
//! Tree-sitter grammars occasionally misattribute a reserved keyword as an
//! `identifier` child of a declaration node — usually in the presence of
//! parse-error recovery, or as a known grammar quirk on certain constructs.
//! When that happens we do NOT want to emit a Function / Struct / Module
//! node named `if` or `return`. This filter exists to catch those cases.
//!
//! # Why it MUST be language-scoped (R46)
//!
//! Until this module was rewritten, there was a single monolithic
//! `RESERVED_KEYWORDS` list that unioned keywords from every supported
//! language. That list contained the Rust keyword `match`, the TypeScript
//! type-keywords `object` / `type` / `in` / `as`, the Python keyword
//! `lambda`, and so on. Applied language-blind, it silently dropped every
//! Apex / Java / C# / TS / JS / Python method named `match` (and many
//! other names that are perfectly legal identifiers in those languages).
//!
//! The NPSP canary surfaced this as R46: `RD2_OpportunityMatcher` has two
//! methods named `match` whose entire declarations were dropped at
//! extraction — leading to zero outgoing call edges, zero fan-in edges to
//! their callees, and one `no_callers` false-positive in the rev-8 audit.
//!
//! The corrected contract:
//!
//! * A name is filtered only when it appears in the CURRENT LANGUAGE's
//!   reserved-keyword list.
//! * Each language has its own list, authored against the language's own
//!   spec — not against "every keyword anyone anywhere might use."
//! * Languages we haven't audited fall back to an empty list (no filtering)
//!   rather than the previous global union. A missing language entry is
//!   explicit and loud, not silent and destructive.
//!
//! # Caller discipline
//!
//! Both call sites (`symbol_extractor.rs` and `trait_context_detector.rs`)
//! reach this validator via `LanguageSpecificExtractor::language()`. The
//! function signature forces them to pass the language — there is no
//! default "assume everything is reserved" fallback.

/// Apex reserved words. Source: Salesforce Apex Developer Guide
/// ("Reserved Keywords"), cross-checked against the tree-sitter-sfapex
/// lexer. Notable absences (valid Apex identifiers despite appearing in
/// other languages' keyword lists): `match`, `type`, `in`, `as`, `is`,
/// `object`, `namespace`, `default`.
const APEX_KEYWORDS: &[&str] = &[
    "abstract",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "do",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "for",
    "global",
    "if",
    "implements",
    "instanceof",
    "interface",
    "new",
    "null",
    "override",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "transient",
    "true",
    "try",
    "virtual",
    "void",
    "webservice",
    "while",
];

/// Java reserved words (JLS §3.9). Excludes contextual / restricted
/// keywords (`record`, `sealed`, `non-sealed`, `permits`, `yield`,
/// `var`) which are legal identifiers outside their declaration contexts.
const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "void",
    "volatile",
    "while",
];

/// C# reserved keywords (ECMA-334 §6.4.4). Contextual keywords
/// (`async`, `await`, `dynamic`, `yield`, etc.) are intentionally not
/// filtered — they are legal identifiers outside their specific grammar
/// positions.
const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// TypeScript / JavaScript — ECMAScript 2024 reserved words
/// (`https://tc39.es/ecma262/#sec-keywords-and-reserved-words`). Type-only
/// keywords such as `any`, `unknown`, `never`, `infer`, `keyof`, `readonly`,
/// `type`, `interface`, `namespace`, `module`, `declare` are TypeScript
/// contextual keywords — they are legal identifiers in value positions and
/// are NOT filtered here. (If a grammar misattribution surfaces for one of
/// them, the fix is to narrow the grammar query, not widen this list.)
const TS_JS_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
];

/// Python reserved words (PEP 8 / Python 3.12 reference). `match` and
/// `case` are SOFT keywords in PEP 634 — legal identifiers outside
/// structural-pattern-match contexts — and are NOT filtered.
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Rust reserved words (Rust Reference — Keywords section, including
/// weak / reserved-for-future-use keywords that can appear as
/// grammar-misattributed identifiers).
const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while", "async", "await",
];

/// Go reserved words (Go Language Spec — Keywords).
const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// Return the reserved-keyword list for `language`, or `None` when the
/// language has no audited list (in which case no filtering is applied).
///
/// The matcher is **case-sensitive by design** — every language currently
/// supported treats keywords as case-sensitive tokens, and case-insensitive
/// comparison would cause false positives on CamelCase identifiers that
/// happen to spell a keyword in a different case.
fn keywords_for(language: &str) -> Option<&'static [&'static str]> {
    match language {
        "apex" => Some(APEX_KEYWORDS),
        "java" => Some(JAVA_KEYWORDS),
        "csharp" | "c_sharp" | "c#" => Some(CSHARP_KEYWORDS),
        "typescript" | "javascript" | "tsx" | "jsx" => Some(TS_JS_KEYWORDS),
        "python" => Some(PYTHON_KEYWORDS),
        "rust" => Some(RUST_KEYWORDS),
        "go" => Some(GO_KEYWORDS),
        _ => None,
    }
}

/// `true` iff `name` is a reserved keyword **in the specified language**.
///
/// Callers that do not have a language in context should not exist — if
/// you find one, pass the `LanguageSpecificExtractor::language()` value
/// from the surrounding extractor. There is deliberately no `_any`
/// variant: the whole reason R46 happened was that a language-blind
/// "is this reserved somewhere?" call silently dropped real Apex methods
/// because `match` is a Rust keyword.
pub fn is_reserved_keyword(name: &str, language: &str) -> bool {
    match keywords_for(language) {
        Some(list) => list.contains(&name),
        None => false,
    }
}

/// Validate a symbol name against the language's reserved-keyword list.
/// Returns `Some(name)` when the name is safe to emit, `None` when it
/// matches a reserved keyword and should be skipped.
pub fn validate_symbol_name<'a>(name: &'a str, language: &str) -> Option<&'a str> {
    if is_reserved_keyword(name, language) {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apex_filters_genuine_apex_keywords() {
        assert!(is_reserved_keyword("if", "apex"));
        assert!(is_reserved_keyword("for", "apex"));
        assert!(is_reserved_keyword("class", "apex"));
        assert!(is_reserved_keyword("return", "apex"));
    }

    #[test]
    fn r46_apex_does_not_filter_cross_language_contamination() {
        // The R46 bug: all of these were filtered by the old monolithic
        // list even though they are legal Apex identifiers. Every one of
        // them must now pass through for Apex.
        for legal_apex_ident in [
            "match",
            "type",
            "in",
            "as",
            "is",
            "of",
            "object",
            "symbol",
            "namespace",
            "default",
            "lambda",
            "def",
            "fn",
            "trait",
            "impl",
            "pub",
            "crate",
            "self",
            "Self",
            "loop",
            "where",
            "function",
            "var",
            "let",
            "const",
            "yield",
            "await",
            "async",
            "import",
            "export",
            "readonly",
            "undefined",
            "any",
            "unknown",
            "never",
        ] {
            assert!(
                !is_reserved_keyword(legal_apex_ident, "apex"),
                "`{legal_apex_ident}` is a legal Apex identifier but was filtered",
            );
        }
    }

    #[test]
    fn java_does_not_filter_match() {
        // `match` is a Java contextual keyword (pattern matching) but is
        // explicitly a legal Java identifier. Do not filter.
        assert!(!is_reserved_keyword("match", "java"));
    }

    #[test]
    fn csharp_does_not_filter_match() {
        assert!(!is_reserved_keyword("match", "csharp"));
        assert!(!is_reserved_keyword("match", "c_sharp"));
    }

    #[test]
    fn python_soft_keywords_match_and_case_are_not_filtered() {
        // PEP 634: `match` and `case` are soft keywords. Still valid as
        // Python identifiers.
        assert!(!is_reserved_keyword("match", "python"));
        assert!(!is_reserved_keyword("case", "python"));
    }

    #[test]
    fn rust_filters_match() {
        assert!(is_reserved_keyword("match", "rust"));
        assert!(is_reserved_keyword("fn", "rust"));
        assert!(is_reserved_keyword("impl", "rust"));
    }

    #[test]
    fn ts_js_does_not_filter_type_or_any() {
        // `type`, `any`, `unknown`, etc. are TS contextual keywords only;
        // do not filter them as identifiers.
        assert!(!is_reserved_keyword("type", "typescript"));
        assert!(!is_reserved_keyword("any", "typescript"));
        assert!(!is_reserved_keyword("unknown", "typescript"));
        assert!(!is_reserved_keyword("never", "typescript"));
    }

    #[test]
    fn unknown_language_no_filtering() {
        // Languages without an audited list fall through to no filtering.
        // Better to emit a nodes we don't want than to silently drop ones
        // we do want.
        assert!(!is_reserved_keyword("match", "cobol"));
        assert!(!is_reserved_keyword("if", "cobol"));
    }

    #[test]
    fn validate_symbol_name_returns_none_on_reserved_and_name_on_valid() {
        assert_eq!(validate_symbol_name("if", "apex"), None);
        assert_eq!(validate_symbol_name("match", "apex"), Some("match"));
        assert_eq!(validate_symbol_name("myFunc", "apex"), Some("myFunc"));
        assert_eq!(validate_symbol_name("match", "rust"), None);
    }

    #[test]
    fn case_sensitive_matching() {
        // `IF` is not `if`; we must not filter CamelCase names that happen
        // to spell a keyword in a different case.
        assert!(is_reserved_keyword("if", "apex"));
        assert!(!is_reserved_keyword("If", "apex"));
        assert!(!is_reserved_keyword("IF", "apex"));
    }
}
