//! # graphengine-ra-ide-adapter
//!
//! Layer 2 semantic-resolution adapter for Rust source, driven by the
//! `ra_ap_ide` rust-analyzer IDE library. Introduced by the
//! universal-fidelity sprint task T6 (see
//! [`docs/workstreams/universal-fidelity/tasks/T6-rust-layer2.md`]).
//!
//! ## What problem this crate solves
//!
//! The engine's Rust pipeline today produces only heuristic call edges
//! (`Provenance { source: Heuristic, confidence: Low }`). There is no
//! code path that can attach `Provenance { source: Lsp, confidence: High }`
//! to a Rust call edge. This crate is that code path: it links the
//! rust-analyzer IDE library directly and runs `goto_definition`
//! queries against the compiler model, skipping the LSP wire protocol
//! entirely (no subprocess, no stdio-piped JSON-RPC, no timeout tuning).
//!
//! ## What this crate deliberately is not
//!
//! - **Not a cross-language adapter.** Java / Kotlin / Python / Go
//!   would each need their own Layer 2 adapter. T6 is the Rust
//!   pathfinder.
//! - **Not a proc-macro expander.** Calls inside proc-macro-expanded
//!   bodies resolve to heuristic edges by design. See
//!   [`docs/workstreams/universal-fidelity/FOLLOWUPS.md`] (`UF-FU-003`).
//! - **Not a replacement for the heuristic resolver.** The heuristic
//!   path stays; this adapter runs ahead of it and forwards anything
//!   it cannot resolve so the composition is additive, not
//!   substitutive.
//! - **Not an LSP transport.** `tsserver` subprocess code continues to
//!   live in `graphengine-parsing/src/infrastructure/lsp/`. This
//!   adapter is library linkage, not wire protocol.
//!
//! ## Architectural contract (the "two-jobs rule" at work)
//!
//! Today's `LspResolver` trait conflates transport (LSP wire) with
//! authority (semantic-grade confidence). `ra_ap_ide` delivers the
//! authority without the transport — proving the two are separable.
//! This crate's public surface takes a minimal file/position input
//! ([`SemanticQueryInput`]) and returns a minimal target output
//! ([`ResolvedTarget`]), both intentionally free of
//! `graphengine-parsing` types. The translation from
//! `UnresolvedReference` to `SemanticQueryInput` lives in the
//! wiring crate (`graphengine-parsing`) per T6 PR #2; this keeps the
//! adapter a pure, independently-testable unit.
//!
//! ## Dependency-pin discipline
//!
//! All `ra_ap_*` dependencies are pinned with `=0.0.307`. This is not
//! a conservative default; it is load-bearing. Every
//! `ra_ap_ide >= 0.0.308` release pulls in `ra-ap-rustc_index` versions
//! that use the unstable `new_zeroed_alloc` library feature, which
//! fails to compile on any released stable Rust toolchain. The
//! measurement matrix lives in the T6 design doc §9.B2.1; the
//! upgrade blocker is tracked as `UF-FU-008` in
//! `docs/workstreams/universal-fidelity/FOLLOWUPS.md`.

use std::path::{Path, PathBuf};

pub use ra_ap_ide::AnalysisHost;

mod query;
mod resolver;

pub use query::SemanticQueryInput;
pub use resolver::{
    Confidence, ResolvedTarget, RustAnalyzerSemanticResolver, SemanticResolverError,
};

/// Absolute-path convenience used by callers that want to avoid
/// constructing a [`SemanticQueryInput`] by hand. Line numbers are
/// 1-based (human-facing) and column numbers are 0-based (byte offset
/// into the line), matching the [`Range`] convention in
/// `graphengine-parsing`'s domain layer.
pub fn query_for_position(file: impl Into<PathBuf>, line: u32, column: u32) -> SemanticQueryInput {
    SemanticQueryInput {
        file: file.into(),
        line,
        column,
    }
}

/// Absolute-path helper. Wraps [`Path::canonicalize`] but preserves
/// the original path if canonicalisation fails (e.g. symlink loop,
/// permissions). Used at workspace-root boundary handling so a
/// relative `Cargo.toml` argument does not silently miss the sysroot
/// discovery path.
pub(crate) fn canonicalize_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod lib_smoke_tests {
    use super::*;

    #[test]
    fn query_for_position_builds_expected_shape() {
        let input = query_for_position("/abs/path/foo.rs", 12, 8);
        assert_eq!(input.file, PathBuf::from("/abs/path/foo.rs"));
        assert_eq!(input.line, 12);
        assert_eq!(input.column, 8);
    }

    #[test]
    fn canonicalize_or_original_preserves_nonexistent_path() {
        let p = Path::new("/definitely/does/not/exist/abcxyz");
        let result = canonicalize_or_original(p);
        assert_eq!(result, PathBuf::from("/definitely/does/not/exist/abcxyz"));
    }
}
