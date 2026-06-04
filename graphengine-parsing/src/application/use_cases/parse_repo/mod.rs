//! Parse repository use case implementation
//!
//! Orchestrates the complete parsing pipeline from workspace discovery
//! to validated graph storage. This is the main entry point for parsing
//! a codebase into a semantic graph representation.

pub mod factory;
pub mod pipeline;
pub mod resolution;
pub mod use_case;

// Re-export main types
pub use use_case::{ParseRepositoryUseCase, ResolvedGraph};
