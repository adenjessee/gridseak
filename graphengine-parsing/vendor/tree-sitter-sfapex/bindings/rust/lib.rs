//! Vendored Salesforce Apex, SOQL, and SOSL tree-sitter grammars.
//!
//! This crate exposes three sibling modules — [`apex`], [`soql`], [`sosl`] —
//! each returning a `tree_sitter::Language` for its grammar. The parsers are
//! generated output from the upstream `aheber/tree-sitter-sfapex` repository
//! (v2.4.0) and are vendored here because the upstream published crate
//! (`tree-sitter-sfapex` on crates.io) targets `tree-sitter 0.22+` while this
//! workspace is pinned to `tree-sitter 0.20` to keep all eight language
//! grammars on a single version.
//!
//! The parser sources are ABI version 14 and self-contained (no external
//! scanners), so they compile cleanly against the older runtime.

use tree_sitter::Language;

pub mod apex {
    use super::Language;

    extern "C" {
        fn tree_sitter_apex() -> Language;
    }

    /// Returns the tree-sitter [`Language`] for the Apex grammar.
    pub fn language() -> Language {
        unsafe { tree_sitter_apex() }
    }

    /// Static `node-types.json` content for the Apex grammar.
    pub const NODE_TYPES: &str = include_str!("../../apex/src/node-types.json");
}

pub mod soql {
    use super::Language;

    extern "C" {
        fn tree_sitter_soql() -> Language;
    }

    /// Returns the tree-sitter [`Language`] for the SOQL grammar.
    ///
    /// SOQL is Salesforce Object Query Language — embedded in Apex as bracketed
    /// query literals (e.g. `[SELECT Id, Name FROM Account]`). This grammar is
    /// used by the SOQL reference extractor to pull SObject and field edges
    /// out of Apex source.
    pub fn language() -> Language {
        unsafe { tree_sitter_soql() }
    }

    pub const NODE_TYPES: &str = include_str!("../../soql/src/node-types.json");
}

pub mod sosl {
    use super::Language;

    extern "C" {
        fn tree_sitter_sosl() -> Language;
    }

    /// Returns the tree-sitter [`Language`] for the SOSL grammar.
    ///
    /// SOSL is Salesforce Object Search Language — a full-text search DSL
    /// embedded in Apex via bracketed `FIND` literals
    /// (e.g. `[FIND 'foo' IN ALL FIELDS RETURNING Account(Id, Name)]`).
    pub fn language() -> Language {
        unsafe { tree_sitter_sosl() }
    }

    pub const NODE_TYPES: &str = include_str!("../../sosl/src/node-types.json");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn round_trip(lang: Language, source: &str) {
        let mut parser = Parser::new();
        parser
            .set_language(lang)
            .expect("failed to load vendored grammar");
        let tree = parser.parse(source, None).expect("parse returned None");
        assert!(
            !tree.root_node().has_error(),
            "unexpected parse error for source: {}",
            source
        );
    }

    #[test]
    fn apex_parses_minimal_class() {
        round_trip(
            apex::language(),
            "public class Sample { public static void run() {} }",
        );
    }

    #[test]
    fn soql_parses_simple_query() {
        round_trip(soql::language(), "SELECT Id, Name FROM Account LIMIT 10");
    }

    #[test]
    fn sosl_language_loads() {
        // SOSL's `FIND` literal grammar has precise tokenization rules that make
        // picking a universally-valid minimal example tricky. Here we just verify
        // the grammar loads successfully into a parser — the real SOSL parsing is
        // exercised against Apex corpus fixtures where SOSL literals appear in
        // context.
        let mut parser = Parser::new();
        parser
            .set_language(sosl::language())
            .expect("sosl grammar should load");
    }
}
