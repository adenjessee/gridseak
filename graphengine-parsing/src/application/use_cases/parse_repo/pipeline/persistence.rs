//! Graph persistence to repository

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::GraphRepository;
use crate::domain::Graph;

/// Graph persistence service
pub struct GraphPersistence;

impl GraphPersistence {
    /// Persist the graph to the repository
    ///
    /// # Arguments
    /// * `graph` - Graph to persist
    /// * `graph_repo` - Repository implementation
    ///
    /// # Returns
    /// * `()` - Success
    /// * `ParsingError` - If persistence fails
    pub async fn persist_to_repository(
        graph: &Graph,
        graph_repo: &dyn GraphRepository,
    ) -> Result<(), ParsingError> {
        graph_repo
            .upsert(graph)
            .await
            .map_err(|e| ParsingError::repository(format!("Graph persistence failed: {}", e)))
    }
}
