//! Storage Backend Abstraction for graphengine-parsing
//!
//! This module mirrors the storage_backend pattern from graphengine-core,
//! providing database-agnostic interfaces for the parsing crate's storage needs.
//!
//! Note: This is intentionally separate from graphengine-core to maintain
//! the parsing crate's independence. If the crates are unified in the future,
//! this can be consolidated.

use anyhow::Result;

/// Storage backend lifecycle management for the parsing system.
///
/// This trait handles database connection, initialization, and maintenance
/// independent of the specific database technology being used.
pub trait ParsingStorageBackend: Send + Sync {
    /// Initialize the storage backend (create tables, run migrations, etc.)
    fn initialize(&self) -> Result<()>;

    /// Run any pending migrations
    fn migrate(&self) -> Result<()>;

    /// Optimize the storage (VACUUM, ANALYZE, index rebuilding, etc.)
    fn optimize(&self) -> Result<()>;

    /// Clear all data from storage
    fn clear(&self) -> Result<()>;

    /// Check if the backend is healthy and connected
    fn health_check(&self) -> Result<bool>;

    /// Get storage statistics
    fn stats(&self) -> Result<ParsingStorageStats>;
}

/// Statistics about the parsing storage backend
#[derive(Debug, Clone, Default)]
pub struct ParsingStorageStats {
    /// Total number of nodes stored
    pub node_count: u64,
    /// Total number of edges stored
    pub edge_count: u64,
    /// Storage size in bytes (if available)
    pub size_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing_storage_stats_default() {
        let stats = ParsingStorageStats::default();
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }
}
