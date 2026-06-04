//! Pipeline modules for the parsing workflow
//!
//! Each module represents a step in the parsing pipeline, allowing
//! for independent testing and clear separation of concerns.

pub mod config;
pub mod file_discovery;
pub mod file_hashing;
pub mod graph_building;
pub mod incremental;
pub mod orchestrator;
pub mod per_file_slicer;
pub mod persistence;
pub mod semantic_resolution;
pub mod symbol_table;
pub mod syntax_extraction;
