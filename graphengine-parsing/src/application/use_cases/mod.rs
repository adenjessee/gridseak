//! Use cases for the parsing system
//!
//! Contains the business logic and orchestration for parsing operations.
//! Each use case represents a complete workflow from input to output.

pub mod containment_builder;
pub mod parse_repo;

// Re-export main use cases
pub use parse_repo::*;
