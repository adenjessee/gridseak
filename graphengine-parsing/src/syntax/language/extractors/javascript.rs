//! JavaScript-specific [`LanguageSpecificExtractor`] implementation.
//!
//! Defers the full surface to [`TypeScriptExtractor`] — the tree-sitter
//! grammar, complexity rules, and receiver-type heuristics are identical
//! between JS and TS for our purposes. Only `language()` differs.

use tree_sitter::Node;

use crate::syntax::language::extractor::LanguageSpecificExtractor;
use crate::syntax::language::extractors::typescript::TypeScriptExtractor;

#[derive(Debug, Default)]
pub struct JavaScriptExtractor {
    inner: TypeScriptExtractor,
}

impl LanguageSpecificExtractor for JavaScriptExtractor {
    fn language(&self) -> &str {
        "javascript"
    }

    fn is_function_definition(&self, kind: &str) -> bool {
        self.inner.is_function_definition(kind)
    }

    fn is_cyclomatic_decision_point(&self, kind: &str) -> bool {
        self.inner.is_cyclomatic_decision_point(kind)
    }

    fn is_cognitive_structural(&self, kind: &str) -> bool {
        self.inner.is_cognitive_structural(kind)
    }

    fn is_flow_break(&self, kind: &str) -> bool {
        self.inner.is_flow_break(kind)
    }

    fn is_logical_operator_node(&self, node: &Node, source: &[u8]) -> bool {
        self.inner.is_logical_operator_node(node, source)
    }

    fn is_continuation_if(&self, node: &Node) -> bool {
        self.inner.is_continuation_if(node)
    }

    fn is_trait_object_type(&self, type_string: &str) -> bool {
        self.inner.is_trait_object_type(type_string)
    }
}
