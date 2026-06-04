//! Stable, content-based node IDs (T2).
//!
//! Computes `SHA256("fqn\0" || fqn || "\0body\0" || normalize(body, lang))`.
//! Falls back to `SHA256("fqn\0" || fqn)` when no body is available
//! (container nodes: file, folder, module, project, crate, and any synthetic
//! scaffolding that does not correspond to a real source range).
//!
//! # Why
//!
//! The legacy scheme (`SHA256(fqn || line || col)`) embedded the source
//! location in the hash. Any edit that shifted a symbol — a blank line
//! inserted above it, indentation reformat, auto-formatter pass — churned the
//! ID even though the symbol itself was unchanged. That broke:
//!
//! - Trend analysis (same symbol rev-over-rev → different IDs → cannot
//!   compare).
//! - Incremental re-parse (cannot re-use previous analysis keyed on ID).
//! - Feedback loops (user "confirms" a finding → the confirmed ID vanishes
//!   after the next formatter run).
//!
//! Content-based IDs only change when the symbol's name (FQN) or its
//! semantic body changes. That is the property T2 is required to deliver.
//!
//! # Normalization rule
//!
//! Derived from DISCOVERY_REPORT §STABLE_ID_NORMALIZATION but tightened to a
//! single, unambiguous token-stream rule after implementation review
//! discovered that the report's rules 2 and 3 as literally written were
//! mutually ambiguous (rule 2 says "collapse all whitespace"; rule 3 says
//! "re-join with \n", which implies newlines survive rule 2). A per-line
//! interpretation leaves trailing-comment / comment-on-own-line bodies
//! producing different IDs after comment stripping, which defeats the
//! purpose. The adopted contract is:
//!
//! 1. Strip all comments (block, line, doc — all three) outside string
//!    literals.
//! 2. Outside string literals, collapse every whitespace run (spaces, tabs,
//!    carriage returns, newlines) to a single space.
//! 3. Strip leading and trailing whitespace from the whole body.
//! 4. **Do not** touch string or char literal contents — they are semantic.
//! 5. **Do not** normalize identifiers — a rename is a semantic change that
//!    must churn IDs.
//!
//! Effect: the normalized form is a single-line token stream with exactly
//! one space between non-string tokens. Any edit that doesn't change
//! tokens, token order, or string-literal contents preserves the ID. The
//! DISCOVERY_REPORT will be amended to reflect this tightened contract
//! (the original report flagged itself as the design ancestor, not the
//! final word; see T2 notes in the implementation log).
//!
//! # Language-awareness
//!
//! Comment syntax varies across languages. To avoid over-normalization
//! (collapsing semantically different code to the same hash, which is strictly
//! worse than the old scheme), we only strip comment syntax we can recognise
//! unambiguously for the given language. The `language` hint drives this:
//!
//! | Family       | Line comment | Block comment | Languages
//! |--------------|--------------|---------------|-------------------------
//! | C-family     | `//`         | `/* ... */`   | rust, java, cs/csharp, c,
//! |              |              |               | cpp, go, javascript,
//! |              |              |               | typescript, apex, kotlin,
//! |              |              |               | swift, scala
//! | Hash-family  | `#`          | —             | python, ruby, bash, shell,
//! |              |              |               | zsh, yaml, perl
//! | Unknown/None | `//`, `/**/` | —             | (safe default — never
//! |              |              |               |  strips `#` so Rust
//! |              |              |               |  `#[attr]`, C `#define`,
//! |              |              |               |  etc. remain intact)
//!
//! # String-literal preservation
//!
//! Both the comment-stripping and whitespace-collapsing passes use a byte
//! scanner that tracks `"..."` and `'...'` string state (with `\`-escapes)
//! and passes string contents through untouched. This prevents the
//! catastrophic failure mode where two semantically different bodies (say,
//! `"a    b"` vs `"a b"` in a string literal) collapse to the same hash.
//!
//! # Known limitations (deliberately deferred)
//!
//! - Rust raw strings (`r#"..."#`), Python triple-quoted strings
//!   (`"""..."""`), JS/TS backticks (`` `...` ``), and C++ raw literals
//!   (`R"(...)"`) are **not** recognised as string literals by this scanner.
//!   Their contents are scanned as code, meaning a `//`, `/* */`, or
//!   (language-dependent) `#` inside such a literal WILL be treated as a
//!   comment and stripped. This is documented honestly rather than hidden: a
//!   future revision of this module can add raw-string awareness if the
//!   false-positive rate becomes visible on dogfood scans.
//! - The `#` character in Rust attribute syntax (`#[derive(Debug)]`) is
//!   never stripped because Rust is a C-family language; `#` is only
//!   stripped for hash-family languages (Python, etc.), which do not use
//!   `#[...]` attribute syntax.

use sha2::{Digest, Sha256};

const FQN_TAG: &[u8] = b"fqn\0";
const BODY_TAG: &[u8] = b"\0body\0";

/// Compute a stable node ID.
///
/// - `body = Some(text)` → hash includes the normalized body. Use for
///   function, method, class, struct, interface, enum, type, and variable
///   declarations where the source range covers actual code.
/// - `body = None` → hash is over FQN only. Use for container nodes (file,
///   folder, module, project, crate) where the "body" concept does not
///   apply, and for synthetic scaffolding.
///
/// `language` is a best-effort comment-syntax hint. `None` selects the
/// conservative C-family-only stripping behaviour (safe default).
pub fn compute_stable_id(fqn: &str, body: Option<&str>, language: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FQN_TAG);
    hasher.update(fqn.as_bytes());
    if let Some(raw) = body {
        let normalized = normalize_body(raw, language);
        hasher.update(BODY_TAG);
        hasher.update(normalized.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Apply the T2 normalization contract to a body string.
///
/// See the module-level documentation for the full rule list.
pub fn normalize_body(body: &str, language: Option<&str>) -> String {
    let hash_is_comment = lang_uses_hash_comments(language);
    let no_comments = strip_comments(body, hash_is_comment);
    collapse_whitespace_outside_strings(&no_comments)
}

fn lang_uses_hash_comments(language: Option<&str>) -> bool {
    let Some(lang) = language else {
        return false;
    };
    matches!(
        lang.to_ascii_lowercase().as_str(),
        "python" | "ruby" | "bash" | "shell" | "zsh" | "yaml" | "perl"
    )
}

/// Strip `//`, `/* */`, and (optionally) `#` line comments while preserving
/// the contents of `"..."` and `'...'` string literals byte-for-byte.
fn strip_comments(input: &str, hash_is_comment: bool) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    let len = bytes.len();
    while i < len {
        let b = bytes[i];
        match b {
            b'"' => {
                let end = scan_through_string(bytes, i, b'"');
                out.push_str(&input[i..end]);
                i = end;
            }
            b'\'' => {
                let end = scan_through_string(bytes, i, b'\'');
                out.push_str(&input[i..end]);
                i = end;
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                if i < len && i + 1 >= len {
                    i = len;
                }
            }
            b'#' if hash_is_comment => {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {
                let end = utf8_char_end(bytes, i);
                out.push_str(&input[i..end]);
                i = end;
            }
        }
    }
    out
}

/// Scan past a string literal starting at `start` delimited by `quote`.
/// Returns the index one past the closing quote (or end-of-input if the
/// literal is unterminated — malformed source is hashed as-is, not skipped).
fn scan_through_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    debug_assert_eq!(bytes[start], quote);
    let len = bytes.len();
    let mut i = start + 1;
    while i < len {
        match bytes[i] {
            b'\\' if i + 1 < len => i += 2,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    len
}

/// Walk one UTF-8 code point forward from byte index `i`. Defensively clamps
/// against mid-sequence bytes (shouldn't occur for valid UTF-8 but guards
/// against panics if upstream produces invalid bytes).
fn utf8_char_end(bytes: &[u8], i: usize) -> usize {
    let b = bytes[i];
    // ASCII (`b < 0x80`) and stray continuation bytes (`0x80..=0xBF`,
    // which should never start a code point in valid UTF-8) both advance
    // by one. Multi-byte leading bytes advance by 2/3/4 depending on the
    // top nibble. Combined into a single `match` so clippy stops asking.
    let size = match b {
        0x00..=0xBF => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    };
    (i + size).min(bytes.len())
}

/// Collapse every whitespace run outside string literals to a single space,
/// then trim leading / trailing spaces. String literal contents pass
/// through byte-for-byte.
fn collapse_whitespace_outside_strings(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut last_was_space = true; // suppresses a leading space
    let mut i = 0;
    let len = bytes.len();
    while i < len {
        let b = bytes[i];
        match b {
            b'"' => {
                let end = scan_through_string(bytes, i, b'"');
                out.push_str(&input[i..end]);
                last_was_space = false;
                i = end;
            }
            b'\'' => {
                let end = scan_through_string(bytes, i, b'\'');
                out.push_str(&input[i..end]);
                last_was_space = false;
                i = end;
            }
            b' ' | b'\t' | b'\r' | b'\n' => {
                if !last_was_space {
                    out.push(' ');
                }
                last_was_space = true;
                i += 1;
            }
            _ => {
                let end = utf8_char_end(bytes, i);
                out.push_str(&input[i..end]);
                last_was_space = false;
                i = end;
            }
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_deterministic() {
        let a = compute_stable_id("mod::foo", Some("x"), Some("rust"));
        let b = compute_stable_id("mod::foo", Some("x"), Some("rust"));
        assert_eq!(a, b);
    }

    #[test]
    fn id_is_64_char_hex() {
        let id = compute_stable_id("f", Some("x"), Some("rust"));
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn none_body_and_empty_body_produce_distinct_ids() {
        // The body-tag sentinel means an empty Some("") is different from
        // None. This is load-bearing for container nodes (None) versus
        // real-but-empty-body nodes (Some("")).
        let none = compute_stable_id("f", None, None);
        let empty = compute_stable_id("f", Some(""), None);
        assert_ne!(none, empty);
    }

    #[test]
    fn fqn_change_changes_id() {
        let a = compute_stable_id("mod::foo", Some("body"), Some("rust"));
        let b = compute_stable_id("mod::bar", Some("body"), Some("rust"));
        assert_ne!(a, b);
    }

    #[test]
    fn indentation_only_change_preserves_id_c_family() {
        let a = "fn f() {\n    let x = 1;\n    x + 1\n}";
        let b = "fn f() {\n        let x = 1;\n        x + 1\n}";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn line_comment_addition_preserves_id_c_family() {
        let a = "fn f() { let x = 1; x + 1 }";
        let b = "fn f() { // trailing\n    let x = 1; x + 1 }";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn block_comment_addition_preserves_id_c_family() {
        let a = "fn f() { let x = 1; let y = 2; }";
        let b = "fn f() { let x = 1; /* aside */ let y = 2; }";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn doc_comment_addition_preserves_id_c_family() {
        let a = "fn f() { 1 + 1 }";
        let b = "/// docstring\nfn f() { 1 + 1 }";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn crlf_and_lf_line_endings_produce_same_id() {
        let a = "line1\nline2\nline3";
        let b = "line1\r\nline2\r\nline3";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn trailing_whitespace_is_normalized_away() {
        let a = "let x = 1;\nlet y = 2;";
        let b = "let x = 1;   \t\nlet y = 2;   ";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn trailing_newlines_are_normalized_away() {
        let a = "let x = 1;";
        let b = "let x = 1;\n\n\n";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn string_literal_whitespace_is_preserved() {
        let a = r#"let s = "a    b";"#;
        let b = r#"let s = "a b";"#;
        assert_ne!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn string_literal_containing_comment_chars_is_preserved() {
        let a = r#"let s = "// not a comment";"#;
        let b = r#"let s = "";"#;
        assert_ne!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn escaped_quote_in_string_does_not_terminate_literal() {
        let a = r#"let s = "she said \"hi\"";"#;
        let b = r#"let s = "she said \"hi\"";"#;
        assert_eq!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
        // And that string content changes DO churn the ID.
        let c = r#"let s = "she said \"bye\"";"#;
        assert_ne!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(c), Some("rust"))
        );
    }

    #[test]
    fn identifier_rename_changes_id() {
        let a = "fn f() { let x = 1; }";
        let b = "fn f() { let y = 1; }";
        assert_ne!(
            compute_stable_id("f", Some(a), Some("rust")),
            compute_stable_id("f", Some(b), Some("rust"))
        );
    }

    #[test]
    fn python_hash_comment_stripped_with_lang_hint() {
        let a = "def f():\n    return 1";
        let b = "def f():\n    # comment\n    return 1";
        assert_eq!(
            compute_stable_id("f", Some(a), Some("python")),
            compute_stable_id("f", Some(b), Some("python"))
        );
    }

    #[test]
    fn python_hash_comment_not_stripped_without_lang_hint() {
        let a = "def f():\n    return 1";
        let b = "def f():\n    # comment\n    return 1";
        assert_ne!(
            compute_stable_id("f", Some(a), None),
            compute_stable_id("f", Some(b), None)
        );
    }

    #[test]
    fn rust_attribute_hash_is_preserved() {
        let raw = "#[derive(Debug)]\nstruct S;";
        let normalized = normalize_body(raw, Some("rust"));
        assert!(
            normalized.contains("#[derive(Debug)]"),
            "normalized='{normalized}'"
        );
    }

    #[test]
    fn fqn_only_fallback_ignores_language_hint() {
        let a = compute_stable_id("mod::foo", None, Some("rust"));
        let b = compute_stable_id("mod::foo", None, Some("python"));
        let c = compute_stable_id("mod::foo", None, None);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn stable_id_excludes_location_information() {
        // The whole point of T2: two identical symbols (same fqn, same body)
        // declared in different locations must hash to the same ID. This
        // test documents the contract.
        let body = "{ return 1; }";
        let a = compute_stable_id("pkg::f", Some(body), Some("apex"));
        let b = compute_stable_id("pkg::f", Some(body), Some("apex"));
        assert_eq!(a, b);
    }
}
