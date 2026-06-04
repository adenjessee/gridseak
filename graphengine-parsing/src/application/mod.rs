//! Application layer for the parsing system
//!
//! This layer orchestrates the parsing pipeline without knowing concrete implementations.
//! It defines use cases (business logic) and ports (traits/interfaces) for dependency inversion.
//!
//! The application layer:
//! - Coordinates the parsing flow from workspace to validated graph
//! - Defines ports that infrastructure adapters must implement
//! - Enforces business rules and validation
//! - Provides async orchestration with proper error handling
//! - Enables testability through dependency injection

pub mod errors;
pub mod ports;
pub mod use_cases;

// Re-export main types for convenience.
// `pub mod mocks;` + `pub use mocks::*;` were removed in R2 (v0.1.0-rc1
// follow-up). The mocks moved to the dev-only
// `graphengine-parsing-test-support` crate so production builds no
// longer link them and the structural call graph reported by
// `gridseak scan` no longer includes substring-resolved edges into
// `MockGraphRepository::get` / `MockSessionSupervisor::clone` / etc.
pub use errors::*;
pub use ports::*;
pub use use_cases::parse_repo::*;
