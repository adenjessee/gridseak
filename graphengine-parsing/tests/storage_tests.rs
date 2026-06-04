//! Integration tests for the SQLite storage infrastructure

use graphengine_parsing::application::ports::GraphRepository;
use graphengine_parsing::domain::{
    Confidence, Edge, Graph, Node, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::SqliteRepository;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_sqlite_repository_creation() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path().to_str().unwrap();
    let _repo = SqliteRepository::new(db_path).unwrap();

    // Test that we can create a repository
    // The repository creation itself is the test
}

#[tokio::test]
async fn test_sqlite_repository_in_memory() {
    let _repo = SqliteRepository::new_in_memory().unwrap();

    // Test that we can create an in-memory repository
    // The repository creation itself is the test
}

#[tokio::test]
async fn test_upsert_and_retrieve_graph() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    // Create test nodes
    let node1 = Node::function("test::module::func1".to_string(), Range::test(1, 0, 1, 20));
    let node2 = Node::struct_(
        "test::module::Struct1".to_string(),
        Range::test(3, 0, 3, 15),
    );

    // Create test edge
    let edge = Edge::call(node1.id.clone(), node2.id.clone(), Provenance::lsp());

    // Create graph
    let mut graph = Graph::new();
    graph.add_node(node1.clone());
    graph.add_node(node2.clone());
    graph.add_edge(edge.clone());

    // Upsert graph
    repo.upsert(&graph).await.unwrap();

    // Retrieve and verify
    let retrieved_graph1 = repo.get(&node1.id).await.unwrap().unwrap();
    assert_eq!(retrieved_graph1.node_count(), 2);
    assert_eq!(retrieved_graph1.edge_count(), 1);

    let retrieved_graph2 = repo.get(&node2.id).await.unwrap().unwrap();
    assert_eq!(retrieved_graph2.node_count(), 2);
    assert_eq!(retrieved_graph2.edge_count(), 1);
}

#[tokio::test]
async fn test_upsert_duplicate_nodes() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    let node = Node::function("test::func".to_string(), Range::test(1, 0, 1, 10));
    let mut graph = Graph::new();
    graph.add_node(node.clone());

    // First upsert
    repo.upsert(&graph).await.unwrap();

    // Modify node and upsert again
    let mut modified_node = node.clone();
    modified_node.provenance = Provenance::new(ProvenanceSource::Heuristic, Confidence::Low);
    let mut modified_graph = Graph::new();
    modified_graph.add_node(modified_node.clone());

    repo.upsert(&modified_graph).await.unwrap();

    // Verify the node was updated
    let retrieved_graph = repo.get(&node.id).await.unwrap().unwrap();
    assert_eq!(retrieved_graph.node_count(), 1);
    assert_eq!(
        retrieved_graph.get_node(&node.id).unwrap().provenance,
        modified_node.provenance
    );
}

#[tokio::test]
async fn test_delete_node() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    let node1 = Node::function("test::func1".to_string(), Range::test(1, 0, 1, 10));
    let node2 = Node::function("test::func2".to_string(), Range::test(2, 0, 2, 10));
    let edge = Edge::call(node1.id.clone(), node2.id.clone(), Provenance::lsp());

    let mut graph = Graph::new();
    graph.add_node(node1.clone());
    graph.add_node(node2.clone());
    graph.add_edge(edge.clone());

    repo.upsert(&graph).await.unwrap();
    assert!(repo.get(&node1.id).await.unwrap().is_some());

    // Delete node1
    repo.delete(&node1.id).await.unwrap();
    assert!(repo.get(&node1.id).await.unwrap().is_none());

    // node2 should still exist
    assert!(repo.get(&node2.id).await.unwrap().is_some());
}

#[tokio::test]
async fn test_list_nodes() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    let node1 = Node::function("test::func1".to_string(), Range::test(1, 0, 1, 10));
    let node2 = Node::function("test::func2".to_string(), Range::test(2, 0, 2, 10));

    let mut graph1 = Graph::new();
    graph1.add_node(node1.clone());
    repo.upsert(&graph1).await.unwrap();

    let mut graph2 = Graph::new();
    graph2.add_node(node2.clone());
    repo.upsert(&graph2).await.unwrap();

    let listed_ids = repo.list().await.unwrap();
    assert_eq!(listed_ids.len(), 2);
    assert!(listed_ids.contains(&node1.id));
    assert!(listed_ids.contains(&node2.id));
}

#[tokio::test]
async fn test_edge_cascade_delete() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    let node1 = Node::function("test::func1".to_string(), Range::test(1, 0, 1, 10));
    let node2 = Node::function("test::func2".to_string(), Range::test(2, 0, 2, 10));
    let edge = Edge::call(node1.id.clone(), node2.id.clone(), Provenance::lsp());

    let mut graph = Graph::new();
    graph.add_node(node1.clone());
    graph.add_node(node2.clone());
    graph.add_edge(edge.clone());

    repo.upsert(&graph).await.unwrap();

    // Delete node1 - should cascade delete the edge
    repo.delete(&node1.id).await.unwrap();

    // node2 should still exist but with no edges
    let retrieved_graph = repo.get(&node2.id).await.unwrap().unwrap();
    assert_eq!(retrieved_graph.node_count(), 1);
    assert_eq!(retrieved_graph.edge_count(), 0);
}

#[tokio::test]
async fn test_empty_graph_operations() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    // Test operations on empty repository
    assert!(repo.get("non_existent_id").await.unwrap().is_none());
    assert!(repo.list().await.unwrap().is_empty());

    // Delete non-existent node should not error
    repo.delete("non_existent_id").await.unwrap();
}

#[tokio::test]
async fn test_large_graph_performance() {
    let repo = SqliteRepository::new_in_memory().unwrap();

    // Create a larger graph with multiple nodes and edges
    let mut graph = Graph::new();

    // Add 100 nodes
    for i in 0..100 {
        let node = Node::function(
            format!("test::module::func{}", i),
            Range::test(i as u32 + 1, 0, i as u32 + 1, 20),
        );
        graph.add_node(node);
    }

    // Add some edges between nodes
    let node_ids: Vec<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    for i in 0..50 {
        if i + 1 < node_ids.len() {
            let edge = Edge::call(
                node_ids[i].clone(),
                node_ids[i + 1].clone(),
                Provenance::lsp(),
            );
            graph.add_edge(edge);
        }
    }

    // Upsert the large graph
    repo.upsert(&graph).await.unwrap();

    // Verify we can retrieve nodes
    let listed_ids = repo.list().await.unwrap();
    assert_eq!(listed_ids.len(), 100);

    // Verify we can retrieve a specific node with its edges
    let retrieved_graph = repo.get(&node_ids[0]).await.unwrap().unwrap();
    assert!(retrieved_graph.node_count() > 0);
}
