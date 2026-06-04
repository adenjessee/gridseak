//! Validation errors for the domain layer
//!
//! Custom error types that represent validation failures in the graph domain.
//! Adapted from the old core system's error handling patterns.

use thiserror::Error;

/// Validation errors that can occur when working with graphs
#[derive(Error, Debug, PartialEq)]
pub enum ValidationError {
    /// An edge references nodes that don't exist in the graph
    #[error("Dangling edge: from {from_id} to {to_id}")]
    DanglingEdge { from_id: String, to_id: String },

    /// Too many elements have confidence below the required threshold
    #[error("Low confidence: {0} elements below threshold")]
    LowConfidence(usize),

    /// An invalid kind was specified for a node or edge
    #[error("Invalid kind: {0}")]
    InvalidKind(String),

    /// Invalid provenance configuration (e.g., LSP source with Low confidence)
    #[error("Invalid provenance: {0}")]
    InvalidProvenance(String),
}
