//! Entry point heuristic rules for dead code and depth analysis.
//!
//! These rules identify functions that are likely called externally (framework handlers,
//! test functions, main entrypoints, barrel exports) and should NOT be flagged as dead code.
//!
//! # Reasoning trace
//!
//! Historically `is_entry_point` returned a bare `bool` — the engine
//! knew it had exempted a function from dead-code analysis but no one
//! else could see *why*. The dead-code reason classifier needs that
//! information to produce an honest verdict, so [`classify_entry_point`]
//! returns an [`EntryPointReason`] enum that names the rule that
//! fired. [`is_entry_point`] is a thin `.is_some()` shim that keeps
//! the old API for depth / hotspot callers that don't need the
//! reason. See also R11 in `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md`.

use super::config::DeadCodeConfig;
use super::graph::AnalysisGraph;

/// Which entry-point heuristic fired for a given node. Each variant
/// corresponds to exactly one `if` branch in [`classify_entry_point`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryPointReason {
    /// Parent file is marked as test scope (test framework harness).
    ParentFileIsTest,
    /// Parent file is vendored third-party code.
    ParentFileIsVendor,
    /// Parent file is build output (generated bundles).
    ParentFileIsBuildOutput,
    /// Parent file is generated code.
    ParentFileIsGenerated,
    /// Function lives in a barrel (re-export) file (`index.ts`, `mod.rs`, `__init__.py`, …).
    BarrelFile,
    /// Function lives in an application entry-point file (`main.rs`, `app.ts`, `manage.py`, …).
    EntrypointFile,
    /// Name matches a framework-handler convention (`Handler`, `Controller`, `middleware`, …).
    FrameworkHandlerConvention,
    /// Name matches a lifecycle method (`constructor`, `componentDidMount`, `__init__`, …).
    LifecycleMethod,
    /// PascalCase name → React/JSX component.
    JsxComponent,
    /// JSX runtime shim (`jsx`, `jsxs`, `Fragment`, …).
    JsxRuntime,
    /// Class/object property accessor (`url`, `body`, `routes`).
    PropertyAccessor,
    /// JSX intrinsic element handler (`<div />`, `<span />`, …).
    JsxIntrinsicElement,
    /// Test-support mock / stub / fake function.
    MockFunction,
    /// Rust trait impl — at least one other code path dispatches via the trait.
    TraitImpl,
    /// Decorated / annotated by a framework (Apex `@AuraEnabled`, Python `@app.route`, …).
    AttributeInvoked,
    /// Passed as a function reference / callback target.
    CallbackTarget,
    /// Exported from a library — reachable outside the compilation unit.
    ExportedSymbol,
    /// Matched a user-supplied extra pattern from config.
    ExtraPatternMatch(String),
}

impl EntryPointReason {
    /// Short stable identifier used by the classifier's evidence
    /// strings. Pairs with [`super::dead_code_classifier::DeadCodeVerdict`].
    pub fn as_str(&self) -> &str {
        match self {
            Self::ParentFileIsTest => "parent_file_is_test",
            Self::ParentFileIsVendor => "parent_file_is_vendor",
            Self::ParentFileIsBuildOutput => "parent_file_is_build_output",
            Self::ParentFileIsGenerated => "parent_file_is_generated",
            Self::BarrelFile => "barrel_file",
            Self::EntrypointFile => "entrypoint_file",
            Self::FrameworkHandlerConvention => "framework_handler_convention",
            Self::LifecycleMethod => "lifecycle_method",
            Self::JsxComponent => "jsx_component",
            Self::JsxRuntime => "jsx_runtime",
            Self::PropertyAccessor => "property_accessor",
            Self::JsxIntrinsicElement => "jsx_intrinsic_element",
            Self::MockFunction => "mock_function",
            Self::TraitImpl => "trait_impl",
            Self::AttributeInvoked => "attribute_invoked",
            Self::CallbackTarget => "callback_target",
            Self::ExportedSymbol => "exported_symbol",
            Self::ExtraPatternMatch(_) => "extra_pattern_match",
        }
    }
}

/// Returns which entry-point rule (if any) fired for the given node.
/// `None` means no heuristic recognised the node — it is a dead-code
/// candidate proper. This is the single decision tree; every other
/// caller is a thin wrapper over this function.
///
/// Heuristics are applied in the order below; the first match wins
/// so that the reported reason is deterministic when a function
/// satisfies several categories simultaneously.
pub fn classify_entry_point(
    graph: &AnalysisGraph,
    node_id: &str,
    dc: &DeadCodeConfig,
) -> Option<EntryPointReason> {
    let node = graph.nodes.get(node_id)?;

    if !node.kind.is_function_like() {
        return None;
    }

    // Universal: test/vendor/generated files (always on, not gated)
    if let Some(parent_file) = graph.classification_of(node_id) {
        if parent_file.is_test {
            return Some(EntryPointReason::ParentFileIsTest);
        }
        if parent_file.is_vendor {
            return Some(EntryPointReason::ParentFileIsVendor);
        }
        if parent_file.is_build_output {
            return Some(EntryPointReason::ParentFileIsBuildOutput);
        }
        if parent_file.is_generated {
            return Some(EntryPointReason::ParentFileIsGenerated);
        }
    }

    if dc.barrel_files && is_in_barrel_file(graph, node_id) {
        return Some(EntryPointReason::BarrelFile);
    }

    if dc.entrypoint_files && is_in_entrypoint_file(graph, node_id) {
        return Some(EntryPointReason::EntrypointFile);
    }

    if dc.framework_handlers && is_framework_handler(&node.name, &node.fqn) {
        return Some(EntryPointReason::FrameworkHandlerConvention);
    }

    if dc.lifecycle_methods && is_lifecycle_method(&node.name) {
        return Some(EntryPointReason::LifecycleMethod);
    }

    if dc.jsx_components && is_jsx_component(&node.name) {
        return Some(EntryPointReason::JsxComponent);
    }

    if dc.jsx_runtime && is_jsx_runtime(&node.name) {
        return Some(EntryPointReason::JsxRuntime);
    }

    if dc.property_accessors && is_property_accessor(&node.name, &node.fqn) {
        return Some(EntryPointReason::PropertyAccessor);
    }

    if dc.jsx_intrinsic && is_jsx_intrinsic_element(&node.fqn) {
        return Some(EntryPointReason::JsxIntrinsicElement);
    }

    if dc.mock_functions && is_mock_function(&node.name) {
        return Some(EntryPointReason::MockFunction);
    }

    if dc.trait_impls && node.is_trait_impl {
        return Some(EntryPointReason::TraitImpl);
    }

    if node.is_attribute_invoked {
        return Some(EntryPointReason::AttributeInvoked);
    }

    if node.is_callback_target {
        return Some(EntryPointReason::CallbackTarget);
    }

    if dc.exported_symbols && node.is_exported() {
        return Some(EntryPointReason::ExportedSymbol);
    }

    // Extra patterns from config (regex, case-insensitive)
    for pattern in &dc.extra_entry_point_patterns {
        if let Ok(re) = regex::Regex::new(&format!("(?i){}", pattern)) {
            if re.is_match(&node.name) || re.is_match(&node.fqn) {
                return Some(EntryPointReason::ExtraPatternMatch(pattern.clone()));
            }
        }
    }

    None
}

/// Boolean shim over [`classify_entry_point`] for callers that only
/// care whether a node is exempt from dead-code detection (depth,
/// hotspot, layering, etc.).
pub fn is_entry_point(graph: &AnalysisGraph, node_id: &str, dc: &DeadCodeConfig) -> bool {
    classify_entry_point(graph, node_id, dc).is_some()
}

fn is_in_barrel_file(graph: &AnalysisGraph, node_id: &str) -> bool {
    let parent_file = match graph.classification_of(node_id) {
        Some(f) => f,
        None => return false,
    };

    let path = parent_file
        .path_repo_rel
        .as_deref()
        .or(parent_file.file_path.as_deref())
        .unwrap_or("");

    let filename = path.rsplit('/').next().unwrap_or(path);

    matches!(
        filename,
        "index.ts"
            | "index.js"
            | "index.tsx"
            | "index.jsx"
            | "index.mts"
            | "index.mjs"
            | "mod.rs"
            | "lib.rs"
            | "__init__.py"
            | "package-info.java"
            | "GlobalUsings.cs"
            | "AssemblyInfo.cs"
    )
}

fn is_in_entrypoint_file(graph: &AnalysisGraph, node_id: &str) -> bool {
    let parent_file = match graph.classification_of(node_id) {
        Some(f) => f,
        None => return false,
    };

    let path = parent_file
        .path_repo_rel
        .as_deref()
        .or(parent_file.file_path.as_deref())
        .unwrap_or("");

    let filename = path.rsplit('/').next().unwrap_or(path);

    let entrypoint_names = [
        "main.ts",
        "main.rs",
        "main.js",
        "main.tsx",
        "app.ts",
        "app.js",
        "app.tsx",
        "server.ts",
        "server.js",
        "main.py",
        "app.py",
        "manage.py",
        "Main.java",
        "Application.java",
        "App.java",
        "Program.cs",
        "Startup.cs",
        "App.cs",
    ];

    if entrypoint_names.contains(&filename) {
        return true;
    }

    // Check if it's a `main` function specifically
    if let Some(node) = graph.nodes.get(node_id) {
        if node.name == "main" {
            return true;
        }
    }

    false
}

const FRAMEWORK_HANDLER_SUFFIXES: &[&str] = &[
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
    "subscriber",
    "consumer",
    "producer",
    "interceptor",
    "guard",
    "pipe",
    "filter",
    "decorator",
];

fn is_framework_handler(name: &str, fqn: &str) -> bool {
    let name_lower = name.to_lowercase();
    let fqn_lower = fqn.to_lowercase();

    for suffix in FRAMEWORK_HANDLER_SUFFIXES {
        if name_lower.ends_with(suffix) || fqn_lower.ends_with(suffix) {
            return true;
        }
    }

    // Common framework prefixes
    let prefixes = ["handle", "on_", "do_", "process_"];
    for prefix in &prefixes {
        if name_lower.starts_with(prefix) {
            return true;
        }
    }

    false
}

const LIFECYCLE_METHODS: &[&str] = &[
    "constructor",
    "init",
    "initialize",
    "setup",
    "teardown",
    "mount",
    "unmount",
    "dispose",
    "destroy",
    "finalize",
    // React
    "componentDidMount",
    "componentWillUnmount",
    "componentDidUpdate",
    "componentDidCatch",
    "getDerivedStateFromProps",
    "getSnapshotBeforeUpdate",
    "shouldComponentUpdate",
    // Angular
    "ngOnInit",
    "ngOnDestroy",
    "ngOnChanges",
    "ngAfterViewInit",
    "ngAfterContentInit",
    // Vue
    "created",
    "mounted",
    "beforeDestroy",
    "destroyed",
    "beforeMount",
    "beforeUpdate",
    "updated",
    // Rust
    "drop",
    "new",
    "default",
    "from",
    "try_from",
    "into",
    // Python runtime-invoked dunder methods (data model)
    "__init__",
    "__del__",
    "__new__",
    "__enter__",
    "__exit__",
    "__call__",
    "__repr__",
    "__str__",
    "__bool__",
    "__len__",
    "__iter__",
    "__next__",
    "__getitem__",
    "__setitem__",
    "__delitem__",
    "__contains__",
    "__eq__",
    "__ne__",
    "__lt__",
    "__le__",
    "__gt__",
    "__ge__",
    "__hash__",
    "__getattr__",
    "__setattr__",
    "__delattr__",
    "__get__",
    "__set__",
    "__delete__",
    "__getstate__",
    "__setstate__",
    "__reduce__",
    "__reduce_ex__",
    "__nonzero__",
    "__format__",
    "__bytes__",
    "__complex__",
    "__int__",
    "__float__",
    "__index__",
    "__add__",
    "__sub__",
    "__mul__",
    "__truediv__",
    "__floordiv__",
    "__mod__",
    "__pow__",
    "__and__",
    "__or__",
    "__xor__",
    "__lshift__",
    "__rshift__",
    "__neg__",
    "__pos__",
    "__abs__",
    "__invert__",
    "__iadd__",
    "__isub__",
    "__imul__",
    "__radd__",
    "__rsub__",
    "__rmul__",
    "__missing__",
    "__copy__",
    "__deepcopy__",
    "__sizeof__",
    "__class_getitem__",
    "__init_subclass__",
    "__instancecheck__",
    "__subclasscheck__",
    "__subclasshook__",
    "__set_name__",
    "__mro_entries__",
    "__post_init__",
    "__aenter__",
    "__aexit__",
    "__aiter__",
    "__anext__",
    "__await__",
    // C# / .NET lifecycle + Unity
    "Awake",
    "Start",
    "Update",
    "FixedUpdate",
    "LateUpdate",
    "OnEnable",
    "OnDisable",
    "OnDestroy",
    "OnApplicationQuit",
    "OnCollisionEnter",
    "OnTriggerEnter",
    "ConfigureServices",
    "Configure",
    "Main",
    "Dispose",
    // Go interface methods
    "ServeHTTP",
    "String",
    "Error",
    "Close",
    "Read",
    "Write",
    "MarshalJSON",
    "UnmarshalJSON",
];

fn is_lifecycle_method(name: &str) -> bool {
    LIFECYCLE_METHODS.contains(&name)
}

/// PascalCase functions in JS/TS are React/JSX components by convention.
/// The JSX compiler requires component names to start with uppercase.
fn is_jsx_component(name: &str) -> bool {
    let first = match name.chars().next() {
        Some(c) => c,
        None => return false,
    };
    // Must start with uppercase letter and have at least 2 chars
    // (single uppercase letters like 'T' are usually type parameters, not components)
    first.is_ascii_uppercase()
        && name.len() >= 2
        && name
            .chars()
            .nth(1)
            .map(|c| c.is_ascii_lowercase() || c.is_ascii_uppercase())
            .unwrap_or(false)
}

/// JSX runtime functions generated by the transpiler (not user-authored calls).
const JSX_RUNTIME_NAMES: &[&str] = &[
    "jsx",
    "jsxs",
    "jsxDEV",
    "jsxAttr",
    "jsxEscape",
    "Fragment",
    "createElement",
    "createRef",
    "h", // Preact/hyperscript
];

fn is_jsx_runtime(name: &str) -> bool {
    JSX_RUNTIME_NAMES.contains(&name)
}

/// Common class getter/property accessor patterns.
/// These are accessed via `obj.property` syntax, not called as functions.
const PROPERTY_ACCESSOR_NAMES: &[&str] = &[
    "url",
    "method",
    "protocol",
    "readyState",
    "hostname",
    "port",
    "pathname",
    "search",
    "hash",
    "href",
    "origin",
    "host",
    "body",
    "headers",
    "status",
    "statusText",
    "type",
    "length",
    "size",
    "name",
    "value",
    "checked",
    "disabled",
    "selected",
];

fn is_property_accessor(name: &str, fqn: &str) -> bool {
    if PROPERTY_ACCESSOR_NAMES.contains(&name) {
        return fqn.contains("::");
    }
    // Class getter pattern: short camelCase name that looks like a property
    // e.g., matchedRoutes, routePath, activeRouter
    if fqn.contains("::") && name.len() <= 20 {
        let lower = name.to_lowercase();
        if lower.ends_with("path") || lower.ends_with("routes") || lower.ends_with("router") {
            return true;
        }
    }
    false
}

/// JSX intrinsic element handlers: functions under `intrinsic_element::components::*`
/// or `jsx::dom::*::components::*`. These are invoked via `<element />` syntax.
fn is_jsx_intrinsic_element(fqn: &str) -> bool {
    let fqn_lower = fqn.to_lowercase();
    fqn_lower.contains("intrinsic_element::components::")
        || fqn_lower.contains("intrinsic-element::components::")
        || (fqn_lower.contains("jsx::dom::") && fqn_lower.contains("::components::"))
}

/// Mock/stub functions used by test infrastructure.
fn is_mock_function(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("mock") || lower.starts_with("stub") || lower.starts_with("fake")
}
