//! Visualforce-page extraction **stage wrapper** (TR-A.5).
//!
//! Post-T5: invoked from
//! [`crate::syntax::language::apex::extractor::ApexExtractor::post_syntax_hooks`]
//! via the trait-method dispatch the orchestrator uses. Previously lived at
//! `application::use_cases::parse_repo::pipeline::vf_extraction`.
//!
//! Sits between syntax extraction and symbol-table building. Reads
//! every `.page` file the SFDX layout detector classified, synthesises
//! the VF container Struct + `__vf_page__` Function + Contains edge,
//! and emits one `UnresolvedReference::FrameworkBinding` per resolved
//! `{!method}` binding so the existing semantic resolver binds each
//! call to its real Apex Function node via FQN-suffix matching and
//! emits `EdgeKind::Framework(VisualforcePage)` by construction (post-P1.d).
//!
//! # Why here, not inside `syntax_extraction`
//!
//! `.page` files are not a tree-sitter language; `syntax_extraction`
//! operates via `SyntaxExtractor` ports that assume tree-sitter. Also,
//! VF resolution needs `class_symbols` populated — which the Apex
//! tree-sitter pass fills during `syntax_extraction`. Running VF
//! extraction here keeps both concerns clean: tree-sitter machinery
//! owns `.cls` / `.trigger`, and the Apex-only VF machinery bolts on
//! afterwards without tainting the generic extraction contract.
//!
//! # No-op for non-Apex parses
//!
//! The stage short-circuits when `syntax_results.class_symbols` is
//! empty. Rust / Java / C# / TS / JS / Python / Go parses never touch
//! this path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::{debug, info, warn};

use crate::application::ports::{CallSite, SyntaxResults};
use crate::domain::apex::class_symbols::ApexClassSymbols;
use crate::domain::FrameworkKind;
use crate::syntax::language::apex::class_symbol_codec::deserialise_class_symbols;
use crate::syntax::language::apex::sfdx_layout;
use crate::syntax::language::apex::vf_page_reader::{self, VfPage};
use crate::syntax::language::apex::vf_page_resolver::{self, ResolvedBinding, VfPageResolution};

/// Public entry point for the stage. Returns aggregated counters the
/// orchestrator can log for telemetry; mutates `syntax_results` in place.
#[derive(Debug, Default, Clone, Copy)]
pub struct VfExtractionStats {
    /// Count of `.page` files discovered and successfully parsed.
    pub pages_parsed: usize,
    /// Count of `.page` files that failed to parse (file-system or
    /// XML errors). Failures are warned, not fatal — a single
    /// malformed page should not halt a whole NPSP scan.
    pub pages_failed: usize,
    /// Count of synthetic Struct + Function nodes emitted (always
    /// `pages_parsed * 2`; surfaced as a field for the metric-envelope
    /// check in `PHASE_A_EXECUTION_PLAN.md` §11).
    pub synthetic_nodes_emitted: usize,
    /// Count of `Call` synthesised `CallSite` rows pushed into
    /// `syntax_results.call_sites`. Bindings that failed resolution
    /// are NOT counted here — they never become CallSites.
    pub bindings_resolved: usize,
    /// Count of bindings that did not resolve to any method on any
    /// candidate class. Reported so §11 can gate "attribute-only
    /// scope coverage" when Phase C work later promotes some of these
    /// to resolved.
    pub bindings_unresolved: usize,
}

/// Run the VF stage. `root` is the repository root (the same path the
/// orchestrator passed into file discovery) — used to locate `.page`
/// files via the shared SFDX layout detector. `workspace_root_str` is
/// the same path normalised for FQN composition (mirrors
/// `syntax_results.workspace_root`).
pub fn run(root: &Path, syntax_results: &mut SyntaxResults) -> Result<VfExtractionStats> {
    let mut stats = VfExtractionStats::default();

    // Fast-path: non-Apex parses never have class_symbols. Short-circuit
    // before touching the filesystem so the VF stage is truly invisible
    // in every non-Apex run.
    if syntax_results.class_symbols.is_empty() {
        debug!("VF extraction: no class_symbols present, skipping stage");
        return Ok(stats);
    }

    let pages = discover_vf_pages(root)?;
    if pages.is_empty() {
        debug!("VF extraction: no .page files under {}", root.display());
        return Ok(stats);
    }

    let class_symbols = deserialise_class_symbols(&syntax_results.class_symbols, "VF extraction");

    // Clone the workspace root up front so the per-page loop can hold
    // a mutable borrow on `syntax_results` without aliasing an
    // outstanding immutable borrow on `syntax_results.workspace_root`.
    let workspace_root: Option<String> = syntax_results.workspace_root.clone();

    for page_path in &pages {
        let parsed = match vf_page_reader::read_vf_page(page_path) {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    "VF extraction: failed to parse {} ({e}); skipping",
                    page_path.display()
                );
                stats.pages_failed += 1;
                continue;
            }
        };
        apply_resolution(
            parsed,
            &class_symbols,
            workspace_root.as_deref(),
            syntax_results,
            &mut stats,
        );
    }

    info!(
        "VF extraction: parsed {} pages, {} failed, {} synthetic nodes, {} bindings resolved, {} unresolved",
        stats.pages_parsed,
        stats.pages_failed,
        stats.synthetic_nodes_emitted,
        stats.bindings_resolved,
        stats.bindings_unresolved,
    );
    Ok(stats)
}

/// Discover every `.page` file under `root` via the SFDX layout
/// detector. Reuses the layout walker so VF discovery stays aligned
/// with Apex-class / trigger discovery — no second tree walk.
fn discover_vf_pages(root: &Path) -> Result<Vec<PathBuf>> {
    let layout = sfdx_layout::detect(root)?;
    Ok(layout.files.apex_vf_pages)
}

/// Apply a per-page `VfPageResolution` to the pipeline's SyntaxResults
/// and bump stats. Kept in its own function so the `run` loop stays a
/// simple per-page iteration and this transformation is unit-testable
/// without spinning up filesystem fixtures.
fn apply_resolution(
    page: VfPage,
    class_symbols: &BTreeMap<String, ApexClassSymbols>,
    workspace_root: Option<&str>,
    results: &mut SyntaxResults,
    stats: &mut VfExtractionStats,
) {
    let resolution: VfPageResolution =
        vf_page_resolver::resolve_vf_page(&page, class_symbols, workspace_root);
    stats.pages_parsed += 1;
    stats.synthetic_nodes_emitted += resolution.synthetic_nodes.len();
    stats.bindings_resolved += resolution.resolved_bindings.len();
    stats.bindings_unresolved += resolution.unresolved_bindings.len();

    for node in resolution.synthetic_nodes {
        results.symbols.push(node);
    }
    for edge in resolution.synthetic_edges {
        results.add_synthesized_edge(edge);
    }

    for ResolvedBinding {
        target_class,
        target_method,
        call_site_location,
        ..
    } in resolution.resolved_bindings
    {
        // `ClassName::methodName` — the resolver's FqnSuffix strategy
        // in `lsp::call_resolver` binds this to the Apex Function
        // whose fully-qualified name ends with exactly that suffix.
        let function_name = format!("{}::{}", target_class, target_method);
        // Post-P1.d: framework context travels in a typed channel
        // (`UnresolvedReference::FrameworkBinding`) rather than an
        // `Option<EdgeKind>` hint on the call site. The resolver
        // dispatches on the enum variant, so the resulting edge is
        // `EdgeKind::Framework(VisualforcePage)` by construction, not
        // by a hint-check that a future resolver arm could forget.
        results.add_framework_binding(
            FrameworkKind::VisualforcePage,
            CallSite {
                location: call_site_location,
                function_name,
                receiver_range: None,
                receiver_text: None,
                arg_types: Vec::new(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{
        Access, ApexClassSymbols, ApexMethod, ApexParameter, ApexTypeRef,
    };

    fn apex_primitive(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive { name: name.into() }
    }

    fn symbols_with_one_method(method_name: &str) -> ApexClassSymbols {
        ApexClassSymbols {
            methods: vec![ApexMethod {
                name: method_name.into(),
                parameters: vec![ApexParameter {
                    name: "x".into(),
                    ty: apex_primitive("String"),
                }],
                return_type: None,
                access: Access::Public,
                is_static: false,
                is_virtual: false,
                is_abstract: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn apply_resolution_pushes_synthetic_nodes_edges_and_call_sites() {
        use crate::syntax::language::apex::vf_page_reader::VfBinding;
        use std::path::PathBuf;

        let page = VfPage {
            name: "P".into(),
            source_path: PathBuf::from("/tmp/ws/pages/P.page"),
            controller: Some("Ctrl".into()),
            extensions: Vec::new(),
            bindings: vec![VfBinding {
                identifier: "save".into(),
                is_invocation: true,
                offset: 0,
            }],
        };
        let mut class_symbols = BTreeMap::new();
        class_symbols.insert("Ctrl".into(), symbols_with_one_method("save"));

        let mut results = SyntaxResults::new();
        let mut stats = VfExtractionStats::default();

        apply_resolution(
            page,
            &class_symbols,
            Some("/tmp/ws"),
            &mut results,
            &mut stats,
        );

        assert_eq!(stats.pages_parsed, 1);
        assert_eq!(stats.synthetic_nodes_emitted, 2);
        assert_eq!(stats.bindings_resolved, 1);
        assert_eq!(stats.bindings_unresolved, 0);

        assert_eq!(results.symbols.len(), 2);
        assert_eq!(results.synthesized_edges.len(), 1);
        assert_eq!(results.references.len(), 1);
        // Post-P1.d: VF bindings emit as
        // `UnresolvedReference::FrameworkBinding` carrying
        // `FrameworkKind::VisualforcePage` by construction — not as
        // a `CallSite` with an `Option<EdgeKind>` hint. The typed
        // variant is checked by enum pattern match, not by field
        // inspection, so any future refactor that accidentally
        // routes VF through `add_call_site*` surfaces as a resolver
        // emitting `EdgeKind::Call` rather than
        // `EdgeKind::Framework(VisualforcePage)` and this test
        // fails.
        match &results.references[0] {
            crate::application::ports::UnresolvedReference::FrameworkBinding(fb) => {
                assert_eq!(fb.framework, FrameworkKind::VisualforcePage);
                assert_eq!(fb.call_site.function_name, "Ctrl::save");
            }
            other => panic!("expected FrameworkBinding for VF extraction, got {other:?}"),
        }
    }

    #[test]
    fn apply_resolution_skips_unresolved_bindings_from_call_sites() {
        use crate::syntax::language::apex::vf_page_reader::VfBinding;
        use std::path::PathBuf;

        let page = VfPage {
            name: "P".into(),
            source_path: PathBuf::from("/tmp/ws/pages/P.page"),
            controller: Some("Ctrl".into()),
            extensions: Vec::new(),
            bindings: vec![VfBinding {
                identifier: "noSuchMethod".into(),
                is_invocation: false,
                offset: 0,
            }],
        };
        let mut class_symbols = BTreeMap::new();
        class_symbols.insert("Ctrl".into(), symbols_with_one_method("save"));

        let mut results = SyntaxResults::new();
        let mut stats = VfExtractionStats::default();

        apply_resolution(
            page,
            &class_symbols,
            Some("/tmp/ws"),
            &mut results,
            &mut stats,
        );

        assert_eq!(stats.bindings_resolved, 0);
        assert_eq!(stats.bindings_unresolved, 1);
        assert_eq!(results.symbols.len(), 2);
        assert_eq!(results.synthesized_edges.len(), 1);
        assert!(results.references.is_empty());
    }
}
