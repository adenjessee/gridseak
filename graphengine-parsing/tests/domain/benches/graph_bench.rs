use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use graphengine_parsing::domain::*;
use std::mem;

fn bench_graph_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_validation");

    for size in [100, 500, 1000, 2000].iter() {
        group.bench_with_input(BenchmarkId::new("nodes_edges", size), size, |b, &size| {
            b.iter(|| {
                let mut graph = Graph::new();

                // Create nodes
                for i in 0..size {
                    let node = Node::function(
                        format!("test::func_{}", i),
                        Range::new(i as u32, 0, (i + 1) as u32, 10),
                    );
                    graph.add_node(node);
                }

                // Create edges (half the number of nodes)
                for i in 0..(size / 2) {
                    let from_id = format!("test::func_{}", i);
                    let to_id = format!("test::func_{}", i + size / 2);
                    let edge = Edge::call(from_id, to_id, Provenance::tree_sitter());
                    graph.add_edge(edge);
                }

                // Validate the graph
                black_box(graph.validate(Confidence::High))
            })
        });
    }
    group.finish();
}

fn bench_node_id_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_id_generation");

    for size in [100, 500, 1000, 2000].iter() {
        group.bench_with_input(BenchmarkId::new("nodes", size), size, |b, &size| {
            b.iter(|| {
                for i in 0..size {
                    let node = Node::function(
                        format!("test::func_{}", i),
                        Range::new(i as u32, 0, (i + 1) as u32, 10),
                    );
                    black_box(node.id);
                }
            })
        });
    }
    group.finish();
}

fn bench_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_usage");

    for size in [100, 500, 1000, 2000].iter() {
        group.bench_with_input(BenchmarkId::new("graph_size", size), size, |b, &size| {
            b.iter(|| {
                let mut graph = Graph::new();

                // Create nodes
                for i in 0..size {
                    let node = Node::function(
                        format!("test::func_{}", i),
                        Range::new(i as u32, 0, (i + 1) as u32, 10),
                    );
                    graph.add_node(node);
                }

                // Create edges
                for i in 0..(size / 2) {
                    let from_id = format!("test::func_{}", i);
                    let to_id = format!("test::func_{}", i + size / 2);
                    let edge = Edge::call(from_id, to_id, Provenance::tree_sitter());
                    graph.add_edge(edge);
                }

                // Measure memory usage
                let node_memory = graph.nodes.len() * mem::size_of::<Node>();
                let edge_memory = graph.edges.len() * mem::size_of::<Edge>();
                let total_memory = node_memory + edge_memory;

                black_box(total_memory)
            })
        });
    }
    group.finish();
}

fn bench_unicode_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("unicode_handling");

    let unicode_cases = [
        "测试::模块::函数",       // Chinese
        "αβγ::δεζ::ηθι",          // Greek
        "функция::модуль::класс", // Cyrillic
        "🚀::📦::⚡",             // Emoji
    ];

    for (i, fqn) in unicode_cases.iter().enumerate() {
        group.bench_with_input(BenchmarkId::new("unicode", i), fqn, |b, &fqn| {
            b.iter(|| {
                let node = Node::function(fqn.to_string(), Range::new(1, 0, 5, 10));
                black_box(node.id)
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_graph_validation,
    bench_node_id_generation,
    bench_memory_usage,
    bench_unicode_handling
);
criterion_main!(benches);
