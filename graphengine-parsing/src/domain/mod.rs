//! Domain layer for the parsing system
//!
//! Pure models with invariants and provenance tracking.
//! No external dependencies beyond stdlib and minimal error handling.

pub mod apex;
pub mod classification;
pub mod edge;
pub mod errors;
pub mod frameworks;
pub mod graph;
pub mod node;
pub mod node_id;
// `pub mod progress;` was removed in R3 (v0.1.0-rc1 follow-up).
// Progress event types and the emitter trait now live in the shared
// `graphengine-progress` crate so the parser, analyzer, runner,
// CLI, and MCP server reference one canonical type system. See
// `docs/02-strategy/V0_1_0_RC1_FOLLOWUP_ISSUES.md` (R3 plan-of-attack)
// for the migration record.
pub mod provenance;

// Re-export main types for convenience
pub use classification::*;
pub use edge::*;
pub use errors::*;
pub use graph::*;
pub use node::*;
pub use provenance::*;
