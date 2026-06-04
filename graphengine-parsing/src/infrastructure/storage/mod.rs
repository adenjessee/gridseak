//! Storage infrastructure for persistent graph data
//!
//! Provides SQLite-based storage implementation of the GraphRepository port
//! with support for nodes, edges, and provenance tracking.
//!
//! # Architecture
//! - `ParsingStorageBackend` - Database-agnostic lifecycle management
//! - `GraphRepository` - High-level graph CRUD operations (from ports)
//! - `SqliteRepository` - Concrete SQLite implementation

pub mod file_cache_repository;
pub mod parse_meta_store;
pub mod schema;
pub mod sqlite_repository;
pub mod storage_backend;

// Re-export main types for convenience
pub use file_cache_repository::{FileCacheRepository, FileCacheRow};
pub use schema::*;
pub use sqlite_repository::*;
pub use storage_backend::*;
