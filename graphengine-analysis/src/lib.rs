//! GraphEngine Analysis System
//!
//! Structural health analysis for parsed code graphs.

pub mod health;
pub mod validation;

/// Crate version string. Exposed so shells (desktop, CI) can stamp telemetry
/// events with the analysis engine version independently of their own shell
/// version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
