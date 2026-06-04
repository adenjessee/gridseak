//! Call site extraction.
//!
//! Extracts function call sites from source code using tree-sitter queries
//! configured per language. Covers plain function calls (`foo(x)`), method
//! calls (`x.foo()`), constructor calls (`new Foo()` — R33 closes a
//! five-language drop here), chained-call receivers for type-aware
//! resolution, and Apex-only explicit-constructor-invocation keywords
//! (`this(...)` / `super(...)` — Phase A §8.3).
//!
//! # Arg-type inference hook
//!
//! When a YAML query captures the call's `argument_list` as `@args`, the
//! extractor calls
//! [`LanguageSpecificExtractor::infer_call_site_arg_types`] to populate
//! [`CallSite::arg_types`](crate::application::ports::CallSite::arg_types).
//! Apex is the only language that overrides this in Phase A — it enables
//! constructor / method overload disambiguation in the heuristic resolver
//! without re-parsing. All other languages inherit the default empty vec,
//! preserving byte-identical output for non-Apex parse DBs.

use crate::application::ports::SyntaxResults;
use crate::domain::apex::class_symbols::ApexTypeRef;
use crate::infrastructure::config::LanguageConfig;
use crate::syntax::language::LanguageSpecificExtractor;
use crate::syntax::utils::node_converter::node_to_range;
use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::debug;
use tree_sitter::Language;

/// Sentinel function name the shared extractor emits when an Apex
/// `explicit_constructor_invocation` captures the `this` keyword.
/// Resolved later by the Apex heuristic resolver against the enclosing
/// class's own constructors. Shared as a `const` so the resolver and
/// extractor never disagree on the spelling.
pub const SELF_CTOR_SENTINEL: &str = "__self::new";

/// Sentinel function name emitted for `super(...)` calls. The Apex
/// resolver rewrites this to the enclosing class's `parent_class`
/// constructor candidate set at resolution time.
pub const SUPER_CTOR_SENTINEL: &str = "__super::new";

/// Prefix prepended to the emitted call-site `function_name` when the
/// call shape is a constructor invocation (`new X(...)`,
/// `this(...)`, `super(...)`). Resolvers branch on this prefix to
/// enter their ctor-specific arm. Centralised here so the extractor
/// and all resolver arms import a single constant.
pub const CONSTRUCTOR_CALL_PREFIX: &str = "constructor_call";

/// Extracts call sites from source code.
pub struct CallSiteExtractor {
    language: Language,
    config: Arc<LanguageConfig>,
    language_extractor: Arc<dyn LanguageSpecificExtractor>,
}

impl CallSiteExtractor {
    pub fn new(
        language: Language,
        config: Arc<LanguageConfig>,
        language_extractor: Arc<dyn LanguageSpecificExtractor>,
    ) -> Self {
        Self {
            language,
            config,
            language_extractor,
        }
    }

    /// Extract call sites from the AST.
    pub fn extract(
        &self,
        root_node: &tree_sitter::Node,
        content: &str,
        file_path: &str,
        results: &mut SyntaxResults,
    ) -> Result<()> {
        let Some(query_str) = self.config.get_query("call_sites") else {
            return Ok(());
        };

        debug!("Extracting call sites from file: {}", file_path);
        let query = tree_sitter::Query::new(self.language, query_str)
            .with_context(|| format!("Invalid call_sites query: {}", query_str))?;

        let mut cursor = tree_sitter::QueryCursor::new();
        let matches = cursor.matches(&query, *root_node, content.as_bytes());

        for mat in matches {
            let mut call_range = None;
            let mut function_name: Option<String> = None;
            let mut call_type = "function_call".to_string();
            let mut receiver_range = None;
            let mut receiver_text: Option<String> = None;
            let mut arg_types: Vec<ApexTypeRef> = Vec::new();

            for capture in mat.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                match capture_name.as_str() {
                    "call" => {
                        call_range = Some(node_to_range(&capture.node, file_path));
                    }
                    "func" => {
                        // Simple / scoped identifiers used by plain
                        // function-call shapes (e.g. `foo(x)`).
                        let node_text = capture
                            .node
                            .utf8_text(content.as_bytes())
                            .unwrap_or("")
                            .to_string();
                        function_name = Some(node_text);
                    }
                    "method_call" => {
                        call_range = Some(node_to_range(&capture.node, file_path));
                        call_type = "method_call".to_string();
                    }
                    "receiver" => {
                        receiver_range = Some(node_to_range(&capture.node, file_path));
                        // TR-A.3 — Apex field-type-aware dispatch needs
                        // the receiver's raw source text so it can
                        // look up the declared type in the local-var
                        // scope / enclosing-class fields / parent-
                        // chain. Whitespace is preserved verbatim;
                        // resolvers trim/normalise at compare time.
                        // Non-Apex languages still populate this
                        // string when their YAML query captures
                        // `@receiver` but no resolver arm consumes it
                        // today, so the extra string allocation does
                        // not change their graph shape.
                        if let Ok(text) = capture.node.utf8_text(content.as_bytes()) {
                            receiver_text = Some(text.to_string());
                        }
                    }
                    "method" => {
                        function_name = Some(
                            capture
                                .node
                                .utf8_text(content.as_bytes())
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                    "constructor_call" => {
                        call_range = Some(node_to_range(&capture.node, file_path));
                        call_type = "constructor_call".to_string();
                    }
                    "type" => {
                        // Generic type capture — used by non-constructor
                        // query shapes (e.g. Rust `Type::new()`). Keep
                        // the legacy `{type}::new` function name so
                        // existing resolver tests stay stable. Constructor
                        // calls go through the `"constructor"` arm below
                        // which also flips `call_type` to `constructor_call`.
                        let type_name = capture.node.utf8_text(content.as_bytes()).unwrap_or("");
                        function_name = Some(format!("{}::new", type_name));
                    }
                    "constructor" => {
                        // R33 closure — Apex / Java / C# / JavaScript /
                        // TypeScript all capture the type expression of
                        // `new X(...)` as `@constructor` in their YAML
                        // configs. Before this arm existed, the legacy
                        // `"type"` arm only fired on a different query
                        // shape and these call sites were silently
                        // dropped (no function name → the emit filter at
                        // the bottom of this loop elided them). Now every
                        // `new X(...)` becomes a `constructor_call:X::new`
                        // call site, which the resolvers (TR-A.1 onward
                        // for Apex; LSP / heuristic fallback for others)
                        // can dispatch on.
                        let type_name = capture.node.utf8_text(content.as_bytes()).unwrap_or("");
                        function_name = Some(format!("{}::new", type_name));
                        call_type = "constructor_call".to_string();
                    }
                    "chained_ctor_keyword" => {
                        // Apex §8.3 explicit-constructor-invocation —
                        // `this(...)` or `super(...)`. Emit a sentinel
                        // function name that the Apex resolver expands
                        // against the enclosing class's own (or parent's)
                        // constructors. Non-Apex YAMLs never emit this
                        // capture; the sentinel therefore only ever
                        // reaches the Apex resolver path.
                        let kw = capture.node.utf8_text(content.as_bytes()).unwrap_or("");
                        function_name = Some(match kw {
                            "this" => SELF_CTOR_SENTINEL.to_string(),
                            "super" => SUPER_CTOR_SENTINEL.to_string(),
                            _ => continue,
                        });
                        call_type = "constructor_call".to_string();
                    }
                    "args" => {
                        // Delegate to the language extractor. Non-Apex
                        // languages return an empty vec (default impl).
                        // Apex populates from literals / ctor-expressions
                        // via `arg_type_inferrer` — feeds TR-A.1's
                        // overload disambiguation without re-parsing.
                        arg_types = self
                            .language_extractor
                            .infer_call_site_arg_types(&capture.node, content.as_bytes());
                    }
                    "scope" => {
                        // Scoped constructor calls like `Type::new()`
                        // (Rust / TS imports).
                        let scope_name = capture.node.utf8_text(content.as_bytes()).unwrap_or("");
                        if let Some(ref mut name) = function_name {
                            *name = format!("{}::{}", scope_name, name);
                        }
                    }
                    "chained_call" => {
                        call_range = Some(node_to_range(&capture.node, file_path));
                        call_type = "chained_call".to_string();
                    }
                    "chained_method" => {
                        function_name = Some(
                            capture
                                .node
                                .utf8_text(content.as_bytes())
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                    _ => {}
                }
            }

            if let (Some(range), Some(name)) = (call_range, function_name) {
                let enhanced_name = if call_type != "function_call" {
                    format!("{}:{}", call_type, name)
                } else {
                    name
                };
                debug!(
                    "Adding call site: {} at {:?} in file {} (receiver: {:?}, receiver_text: {:?}, arg_types: {})",
                    enhanced_name,
                    range,
                    file_path,
                    receiver_range,
                    receiver_text,
                    arg_types.len()
                );
                results.add_call_site_with_receiver_text(
                    range,
                    enhanced_name,
                    receiver_range,
                    receiver_text,
                    arg_types,
                );
            }
        }

        Ok(())
    }
}
