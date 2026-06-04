//! Infrastructure layer benchmarks
//!
//! Benchmarks for configuration loading, syntax extraction, and parsing pipeline
//! performance. Measures both speed and memory usage for infrastructure components.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::config::{get_available_languages, load_config};
use graphengine_parsing::*;
// R2 (v0.1.0-rc1 follow-up) — see comment in
// `tests/infrastructure_tests.rs` for context. Mocks live in the
// dev-only test-support crate so the GridSeak scan no longer surfaces
// `MockSyntaxExtractor::parse_file` / `MockSessionSupervisor::clone` /
// `MockGraphRepository::get` as production-source fan-in attractors.
use graphengine_parsing_test_support::MockSyntaxExtractorWithConfig as MockSyntaxExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

/// Benchmark configuration loading performance
fn bench_config_loading(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_loading");

    group.bench_function("load_rust_config", |b| {
        b.iter(|| {
            let config = black_box(load_config("rust").unwrap());
            assert_eq!(config.language, "rust");
        });
    });

    group.bench_function("validate_config", |b| {
        let config = load_config("rust").unwrap();
        b.iter(|| {
            config.validate().unwrap();
            black_box(());
        });
    });

    group.bench_function("get_available_languages", |b| {
        b.iter(|| {
            let languages = black_box(get_available_languages().unwrap());
            assert!(!languages.is_empty());
        });
    });

    group.finish();
}

/// Benchmark syntax extraction performance
fn bench_syntax_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("syntax_extraction");

    // Create test data
    let temp_dir = TempDir::new().unwrap();
    let test_files = create_test_files(&temp_dir, 10);

    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    group.throughput(Throughput::Elements(10));
    group.bench_function("extract_10_files", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            let results = rt.block_on(extractor.extract(&test_files)).unwrap();
            black_box(results);
        });
    });

    // Test with different file sizes
    for &file_count in &[1, 5, 10, 20] {
        let test_files = create_test_files(&temp_dir, file_count);
        group.throughput(Throughput::Elements(file_count as u64));
        group.bench_with_input(
            format!("extract_{}_files", file_count),
            &file_count,
            |b, &_file_count| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                b.iter(|| {
                    let results = rt.block_on(extractor.extract(&test_files)).unwrap();
                    black_box(results);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark parsing pipeline performance
fn bench_parsing_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("parsing_pipeline");

    // Create test data on disk; the binding is intentionally unused because
    // the pipeline below scans `temp_dir` directly.
    let temp_dir = TempDir::new().unwrap();
    let _test_files = create_test_files(&temp_dir, 5);

    group.throughput(Throughput::Elements(5));
    group.bench_function("complete_pipeline", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            // Inlined what used to be
            // `ParseRepositoryUseCase::with_infrastructure(...)`. The
            // factory method was deleted in R2; mocks come from the
            // dev-only `graphengine-parsing-test-support` crate.
            let config = load_config("rust").unwrap();
            let syntax_extractor = Box::new(
                graphengine_parsing_test_support::MockSyntaxExtractorWithConfig::new(
                    config.clone(),
                ),
            );
            let semantic_resolver = Box::new(
                graphengine_parsing_test_support::MockLspResolver::new(config, None),
            );
            let graph_repo = Box::new(graphengine_parsing_test_support::MockGraphRepository::new());

            let use_case = ParseRepositoryUseCase::new(
                syntax_extractor,
                semantic_resolver,
                graph_repo,
                Confidence::Medium,
            );

            let result = rt
                .block_on(use_case.parse(temp_dir.path().to_path_buf(), "rust".to_string()))
                .unwrap();
            black_box(result);
        });
    });

    group.finish();
}

/// Benchmark memory usage for large codebases
fn bench_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_usage");

    // Test with different codebase sizes
    for &file_count in &[10, 50, 100, 200] {
        let temp_dir = TempDir::new().unwrap();
        let test_files = create_test_files(&temp_dir, file_count);

        let config = load_config("rust").unwrap();
        let extractor = MockSyntaxExtractor::new(config);

        group.throughput(Throughput::Elements(file_count as u64));
        group.bench_with_input(
            format!("memory_{}_files", file_count),
            &file_count,
            |b, &_file_count| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                b.iter(|| {
                    let results = rt.block_on(extractor.extract(&test_files)).unwrap();

                    // Measure approximate memory usage
                    let node_memory = results.symbols.len() * std::mem::size_of::<Node>();
                    let call_site_memory = results.references.len() * std::mem::size_of::<Range>();
                    let import_memory = results.imports.len() * std::mem::size_of::<Range>();
                    let type_ref_memory = results.type_refs.len() * std::mem::size_of::<Range>();

                    let total_memory =
                        node_memory + call_site_memory + import_memory + type_ref_memory;

                    // Ensure we're using reasonable memory
                    assert!(total_memory < 100 * 1024 * 1024); // Less than 100MB
                    black_box(total_memory);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark parallel processing efficiency
fn bench_parallel_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_efficiency");

    let temp_dir = TempDir::new().unwrap();
    let test_files = create_test_files(&temp_dir, 100);

    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    group.throughput(Throughput::Elements(100));
    group.bench_function("parallel_extraction", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            let start = std::time::Instant::now();
            let results = rt.block_on(extractor.extract(&test_files)).unwrap();
            let duration = start.elapsed();

            // Should complete in reasonable time (less than 1 second for 100 files)
            assert!(duration.as_millis() < 1000);
            black_box(results);
        });
    });

    group.finish();
}

/// Benchmark error handling performance
fn bench_error_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_handling");

    let config = load_config("rust").unwrap();
    let extractor = MockSyntaxExtractor::new(config);

    // Test with non-existent files
    let non_existent_files = vec![
        PathBuf::from("/nonexistent/file1.rs"),
        PathBuf::from("/nonexistent/file2.rs"),
        PathBuf::from("/nonexistent/file3.rs"),
    ];

    group.bench_function("handle_missing_files", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            let results = rt.block_on(extractor.extract(&non_existent_files)).unwrap();
            black_box(results);
        });
    });

    // Test with unsupported file types
    let unsupported_files = vec![
        PathBuf::from("test.js"),
        PathBuf::from("test.py"),
        PathBuf::from("test.java"),
    ];

    group.bench_function("handle_unsupported_files", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            let results = rt.block_on(extractor.extract(&unsupported_files)).unwrap();
            black_box(results);
        });
    });

    group.finish();
}

/// Helper function to create test files
fn create_test_files(temp_dir: &TempDir, count: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();

    for i in 0..count {
        let file_path = temp_dir.path().join(format!("test_{}.rs", i));
        let content = format!(
            r#"
fn function_{}() -> i32 {{
    let result = helper_{}();
    result * 2
}}

fn helper_{}() -> i32 {{
    42
}}

struct Struct{} {{
    field: i32,
}}

mod module_{} {{
    pub fn inner_function() {{}}
}}

use std::collections::HashMap;
"#,
            i, i, i, i, i
        );

        std::fs::write(&file_path, content).unwrap();
        files.push(file_path);
    }

    files
}

criterion_group!(
    benches,
    bench_config_loading,
    bench_syntax_extraction,
    bench_parsing_pipeline,
    bench_memory_usage,
    bench_parallel_efficiency,
    bench_error_handling
);

criterion_main!(benches);
