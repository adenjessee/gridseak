//! Statistics and aggregation modules for LSP resolution
//!
//! These modules handle tracking resolution statistics, edge deduplication,
//! and aggregating results from different resolution phases.

pub mod aggregator;
pub mod collector;

pub use aggregator::*;
pub use collector::*;
