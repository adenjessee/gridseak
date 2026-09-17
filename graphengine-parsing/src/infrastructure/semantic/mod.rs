//! Layer-2 semantic resolvers (per T6 — universal-fidelity sprint).
//!
//! This module sits at the same level as `infrastructure::lsp` but
//! holds Layer-2 semantic resolvers that run **without** an LSP-wire
//! transport — they link an IDE-grade name resolver directly as a
//! library. Today the only resident is the Rust adapter
//! (`RustLayer2SemanticResolver`, feature-gated behind
//! `rust-layer2`); other languages will land their own adapters in
//! this module as they graduate from Layer 1 (heuristic) to Layer 2
//! (semantic) per the layered-fidelity architecture.
//!
//! See [`docs/workstreams/universal-fidelity/tasks/T6-rust-layer2.md`]
//! for the design contract and the "two-jobs rule" split between
//! *transport* (LSP wire) and *authority* (semantic-grade confidence).

#[cfg(feature = "rust-layer2")]
pub mod rust_layer2;

pub mod caret;
pub mod disclosed_resolver;
pub mod symbol_mapping;

#[cfg(feature = "scip")]
pub mod index_backed;
#[cfg(feature = "scip")]
pub mod scip_semantic_index;

#[cfg(feature = "rust-layer2")]
pub use rust_layer2::RustLayer2SemanticResolver;

pub use disclosed_resolver::DisclosedSemanticResolver;
#[cfg(feature = "scip")]
pub use index_backed::IndexBackedSemanticResolver;
#[cfg(feature = "scip")]
pub use scip_semantic_index::ScipSemanticIndex;
