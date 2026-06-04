//! Graph representation for parsed code
//!
//! Adapted from the old core system's UniversalGraph with validation and traversal.
//! Aggregates nodes and edges with integrity checking.

use super::edge::Edge;
use super::errors::ValidationError;
use super::node::Node;
use super::provenance::Confidence;
use std::collections::{HashMap, HashSet, VecDeque};

/// A graph representing parsed code with nodes and relationships
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Graph {
    /// All nodes in the graph
    pub nodes: Vec<Node>,
    /// All edges in the graph
    pub edges: Vec<Edge>,
    /// Pipeline metadata (e.g. lsp_available, language)
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl Graph {
    /// Create a new empty graph
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: Node) {
        self.nodes.push(node);
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    /// Validate the graph for integrity
    pub fn validate(&self, min_confidence: Confidence) -> Result<(), ValidationError> {
        // Check for dangling edges
        let node_ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();

        for edge in &self.edges {
            if !node_ids.contains(edge.from_id.as_str()) {
                return Err(ValidationError::DanglingEdge {
                    from_id: edge.from_id.clone(),
                    to_id: edge.to_id.clone(),
                });
            }
            if !node_ids.contains(edge.to_id.as_str()) {
                return Err(ValidationError::DanglingEdge {
                    from_id: edge.from_id.clone(),
                    to_id: edge.to_id.clone(),
                });
            }
        }

        // Check confidence levels
        let mut low_conf_count = 0;
        for edge in &self.edges {
            if edge.provenance.confidence < min_confidence {
                low_conf_count += 1;
            }
        }

        if low_conf_count > 0 {
            return Err(ValidationError::LowConfidence(low_conf_count));
        }

        Ok(())
    }

    /// Get a node by ID
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Get all edges from a node
    pub fn get_edges_from(&self, node_id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.from_id == node_id).collect()
    }

    /// Get all edges to a node
    pub fn get_edges_to(&self, node_id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.to_id == node_id).collect()
    }

    /// Breadth-first search from a starting node
    pub fn bfs(&self, start_id: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut result = Vec::new();

        if self.get_node(start_id).is_none() {
            return result; // Node doesn't exist
        }

        queue.push_back(start_id.to_string());
        visited.insert(start_id.to_string());

        while let Some(current_id) = queue.pop_front() {
            result.push(current_id.clone());

            // Add all unvisited neighbors
            for edge in self.get_edges_from(&current_id) {
                if !visited.contains(&edge.to_id) {
                    visited.insert(edge.to_id.clone());
                    queue.push_back(edge.to_id.clone());
                }
            }
        }

        result
    }

    /// Depth-first search from a starting node
    pub fn dfs(&self, start_id: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();

        if self.get_node(start_id).is_none() {
            return result; // Node doesn't exist
        }

        self.dfs_recursive(start_id, &mut visited, &mut result);
        result
    }

    /// Recursive helper for DFS
    fn dfs_recursive(
        &self,
        node_id: &str,
        visited: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) {
        if visited.contains(node_id) {
            return;
        }

        visited.insert(node_id.to_string());
        result.push(node_id.to_string());

        // Visit all neighbors
        for edge in self.get_edges_from(node_id) {
            self.dfs_recursive(&edge.to_id, visited, result);
        }
    }

    /// Get the number of nodes
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the number of edges
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Check if the graph is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}
