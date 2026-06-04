//! Infrastructure layer for graphengine-parsing
//!
//! This module contains the concrete implementations of the ports defined
//! in the application layer. It includes:
//!
//! - Configuration system for language-specific parsing
//! - Tree-sitter based syntax extractor (when available)
//! - LSP-based semantic extractor, repository implementations
//!
//! Mock implementations of the same ports now live in the dev-only
//! `graphengine-parsing-test-support` crate (R2 follow-up, v0.1.0-rc1).
//! That keeps production source — and therefore the `gridseak scan`
//! structural graph — free of fake `MockSyntaxExtractor` /
//! `MockLspResolver` / `MockGraphRepository` symbols whose short
//! generic methods (`get`, `clone`, `new`, …) were soaking up
//! substring-resolved edges from every consumer in the workspace.

pub mod config;
pub mod lsp;
pub mod semantic;
pub mod storage;
pub mod utils;

// Re-export main types for convenience
pub use config::*;
pub use lsp::*;
#[cfg(feature = "rust-layer2")]
pub use semantic::RustLayer2SemanticResolver;
pub use storage::*;
pub use utils::*;
