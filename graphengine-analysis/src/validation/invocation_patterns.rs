//! Invocation pattern detection for dead code candidates.
//!
//! Every dead code false positive falls into one of these invocation mechanisms
//! that the static call graph cannot capture. Detecting the pattern allows the UI
//! to group candidates by root cause and offer bulk actions.

use crate::health::config::{DeadCodeConfig, Ecosystem};
use crate::health::graph::{AnalysisGraph, GraphNode};

/// Invocation mechanism that explains why a function with zero callers
/// may not actually be dead code.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationPattern {
    TraitImpl,
    InterfaceImpl,
    DecoratorInvoked,
    CallbackTarget,
    DynamicDispatch,
    MacroGenerated,
    ExportedUncalled,
    FrameworkRegistered,
}

/// Detail about the specific invocation pattern detected.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvocationDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trait_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementing_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework_hint: Option<String>,
}

/// A dead code candidate with rich context for user validation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeadCodeCandidate {
    pub function_id: String,
    pub fqn: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub reason: String,
    pub is_exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_pattern: Option<InvocationPattern>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_detail: Option<InvocationDetail>,
    pub suggested_action: String,
    pub suggestion_reason: String,
}

/// Detect the invocation pattern for a dead code candidate, if any.
pub fn detect_invocation_pattern(
    node: &GraphNode,
    _graph: &AnalysisGraph,
    ecosystem: Ecosystem,
) -> (Option<InvocationPattern>, Option<InvocationDetail>) {
    // Trait impl (Rust)
    if node.is_trait_impl {
        return (
            Some(InvocationPattern::TraitImpl),
            Some(InvocationDetail {
                trait_name: node.trait_name.clone(),
                implementing_type: None,
                framework_hint: None,
            }),
        );
    }

    // Attribute-invoked (decorators in Python/Java, annotations in Rust)
    if node.is_attribute_invoked {
        return (
            Some(InvocationPattern::DecoratorInvoked),
            Some(InvocationDetail {
                trait_name: None,
                implementing_type: None,
                framework_hint: Some("decorator/annotation-invoked function".into()),
            }),
        );
    }

    // Callback target
    if node.is_callback_target {
        return (
            Some(InvocationPattern::CallbackTarget),
            Some(InvocationDetail {
                trait_name: None,
                implementing_type: None,
                framework_hint: Some("passed as callback/closure argument".into()),
            }),
        );
    }

    // Go interface implementations (name-based heuristic)
    if ecosystem == Ecosystem::Go {
        const GO_INTERFACE_METHODS: &[&str] = &[
            "ServeHTTP",
            "String",
            "Error",
            "Close",
            "Read",
            "Write",
            "MarshalJSON",
            "UnmarshalJSON",
            "Len",
            "Less",
            "Swap",
            "Format",
            "Scan",
            "Value",
            "GobEncode",
            "GobDecode",
        ];
        if GO_INTERFACE_METHODS.contains(&node.name.as_str()) {
            return (
                Some(InvocationPattern::InterfaceImpl),
                Some(InvocationDetail {
                    trait_name: None,
                    implementing_type: None,
                    framework_hint: Some(format!(
                        "Go interface method '{}' — likely implements a standard interface",
                        node.name
                    )),
                }),
            );
        }
    }

    // Exported but uncalled
    if node.is_exported() {
        return (
            Some(InvocationPattern::ExportedUncalled),
            Some(InvocationDetail {
                trait_name: None,
                implementing_type: None,
                framework_hint: Some("public/exported function with no internal callers".into()),
            }),
        );
    }

    // Framework handler heuristic
    let name_lower = node.name.to_lowercase();
    let handler_suffixes = [
        "handler",
        "controller",
        "middleware",
        "route",
        "resolver",
        "loader",
        "action",
        "endpoint",
        "servlet",
        "listener",
    ];
    let handler_prefixes = ["handle", "on_", "do_", "process_"];

    for suffix in &handler_suffixes {
        if name_lower.ends_with(suffix) {
            return (
                Some(InvocationPattern::FrameworkRegistered),
                Some(InvocationDetail {
                    trait_name: None,
                    implementing_type: None,
                    framework_hint: Some(format!(
                        "name ends with '{suffix}' — likely a framework-registered handler"
                    )),
                }),
            );
        }
    }
    for prefix in &handler_prefixes {
        if name_lower.starts_with(prefix) {
            return (
                Some(InvocationPattern::FrameworkRegistered),
                Some(InvocationDetail {
                    trait_name: None,
                    implementing_type: None,
                    framework_hint: Some(format!(
                        "name starts with '{prefix}' — likely a framework handler"
                    )),
                }),
            );
        }
    }

    (None, None)
}

/// Build the suggested action and reason for a dead code candidate.
pub fn suggest_action(
    pattern: &Option<InvocationPattern>,
    node: &GraphNode,
    ecosystem: Ecosystem,
) -> (String, String) {
    match pattern {
        Some(InvocationPattern::TraitImpl) => (
            "likely_entry_point".into(),
            format!(
                "trait implementation{} — probably invoked via dynamic dispatch",
                node.trait_name
                    .as_ref()
                    .map(|t| format!(" of '{t}'"))
                    .unwrap_or_default()
            ),
        ),
        Some(InvocationPattern::InterfaceImpl) => (
            "likely_entry_point".into(),
            format!(
                "'{}' is a common Go interface method — likely invoked via interface dispatch",
                node.name
            ),
        ),
        Some(InvocationPattern::DecoratorInvoked) => (
            "likely_entry_point".into(),
            "decorator/annotation-invoked — framework calls this at runtime".into(),
        ),
        Some(InvocationPattern::CallbackTarget) => (
            "likely_entry_point".into(),
            "passed as callback argument — invoked dynamically".into(),
        ),
        Some(InvocationPattern::ExportedUncalled) => {
            let action = if matches!(ecosystem, Ecosystem::Rust)
                && node.visibility.as_deref() == Some("pub_crate")
            {
                "review"
            } else {
                "likely_entry_point"
            };
            (
                action.into(),
                "exported/public function — may be part of the library's public API".into(),
            )
        }
        Some(InvocationPattern::FrameworkRegistered) => (
            "likely_entry_point".into(),
            "naming pattern suggests framework-registered handler".into(),
        ),
        Some(InvocationPattern::DynamicDispatch) => (
            "review".into(),
            "dynamic dispatch — cannot be resolved statically, needs manual review".into(),
        ),
        Some(InvocationPattern::MacroGenerated) => (
            "review".into(),
            "possibly macro-generated — macros expand before tree-sitter sees the AST".into(),
        ),
        None => (
            "review".into(),
            "no invocation pattern detected — manual review needed".into(),
        ),
    }
}

/// Collect dead code candidates with rich invocation context from the graph.
pub fn collect_dead_code_candidates(
    graph: &AnalysisGraph,
    dc_config: &DeadCodeConfig,
    ecosystem: Ecosystem,
) -> Vec<DeadCodeCandidate> {
    let dead_result = crate::health::dead_code::detect_dead_code(graph, dc_config);
    let mut candidates = Vec::new();

    // Invocation-pattern suggestions are only meaningful for
    // production code. Test/vendor dead code is intentional noise;
    // `DeadCodeResult.production` is the typed slice that guarantees
    // the scope without a second filter here.
    for id in &dead_result.production {
        let node = match graph.nodes.get(id) {
            Some(n) => n,
            None => continue,
        };

        let (pattern, detail) = detect_invocation_pattern(node, graph, ecosystem);
        let (suggested_action, suggestion_reason) = suggest_action(&pattern, node, ecosystem);

        candidates.push(DeadCodeCandidate {
            function_id: id.clone(),
            fqn: node.fqn.clone(),
            file: node.file_path.clone(),
            line: node.start_line,
            reason: "zero incoming call edges".into(),
            is_exported: node.is_exported(),
            invocation_pattern: pattern,
            invocation_detail: detail,
            suggested_action,
            suggestion_reason,
        });
    }

    candidates
}
