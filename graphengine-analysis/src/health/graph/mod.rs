//! In-memory graph representation loaded from SQLite.
//!
//! This module owns the data model that all analysis algorithms operate
//! on. It loads nodes and edges from the SQLite database produced by
//! `graphengine-parsing`, builds adjacency lists, and provides
//! traversal helpers (containment walk, module resolution).
//!
//! R1 (v0.1.0-rc1 follow-up) split the original 1,965 LOC `graph.rs`
//! into five files so the data model, the load path, the traversal
//! algorithms, the classification predicates, and the language
//! detection no longer share a wall of code. The public API is
//! preserved verbatim through the `pub use` re-exports below; every
//! caller that imports `crate::health::graph::Foo` keeps working
//! without change.
//!
//! Sub-module layout:
//! - [`types`] — pure data types ([`NodeKind`], [`EdgeKind`],
//!   [`FrameworkKind`], [`DeclarativeKind`], [`PersistedEdgeKind`],
//!   [`GraphNode`], [`Confidence`], [`GraphEdge`]). No `Connection`,
//!   no traversal — anyone can compile against this file.
//! - [`loader`] — `rusqlite::Connection` -> raw nodes + edges. Owns
//!   the only `prepare(...)` calls in the module.
//! - [`analysis_graph`] — the assembled [`AnalysisGraph`] struct, its
//!   adjacency / containment caches, and the core traversal methods.
//! - [`classification`] — the [`is_synthetic_node`] predicate plus
//!   the test/production classification methods on `AnalysisGraph`.
//! - [`language`] — File-level language / framework propagation and
//!   the public `detect_ecosystem` / `detect_primary_language` views
//!   the report uses.

mod analysis_graph;
mod classification;
mod language;
mod loader;
mod types;

// Re-exports that preserve the pre-R1 flat surface at
// `crate::health::graph::*`. External callers (every other module
// in this crate, plus `bin/ge_analyze.rs` and the integration
// tests) pull names from this path; the re-export list is
// load-bearing for backward compatibility.
pub use analysis_graph::AnalysisGraph;
pub use classification::is_synthetic_node;
pub use language::{detect_ecosystem, detect_primary_language};
pub use loader::{read_metadata, validate_schema};
pub use types::{
    Confidence, DeclarativeKind, EdgeKind, FrameworkKind, GraphEdge, GraphNode, NodeKind,
    PersistedEdgeKind,
};
