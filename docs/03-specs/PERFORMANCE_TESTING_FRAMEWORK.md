# Performance Testing Framework

## Purpose

Establish **objective metrics** for measuring parsing and resolution performance. All optimization decisions should be backed by data from these tests.

---

## 1. Baseline Metrics to Capture

### 1.1 LSP Server Metrics

| Metric | Command | What It Measures |
|--------|---------|------------------|
| Cold Start Time | `time typescript-language-server --stdio < init.json` | Time to ready |
| Single Query Latency | Timed go-to-definition request | Per-request cost |
| Batch Throughput | 10 requests in parallel | Effective parallelism |
| Memory Usage | `ps -o rss` during operation | Resource cost |

### 1.2 Tree-sitter Metrics

| Metric | What It Measures |
|--------|------------------|
| Parse Time (per file) | Raw syntax extraction speed |
| AST Node Count | Complexity indicator |
| Query Match Count | Extraction efficiency |

### 1.3 End-to-End Metrics

| Metric | What It Measures |
|--------|------------------|
| Total Processing Time | User-visible duration |
| Peak Memory Usage | Resource requirements |
| Cache Hit Rate | Optimization effectiveness |
| Resolution Accuracy | Correctness |

---

## 2. Test Projects

### 2.1 Synthetic Test Projects

Create controlled test projects with known characteristics:

```bash
# Small: 100 files, ~5K LOC
test/perf/typescript_small/

# Medium: 1,000 files, ~50K LOC  
test/perf/typescript_medium/

# Large: 5,000 files, ~250K LOC
test/perf/typescript_large/
```

### 2.2 Generate Test Projects

```bash
#!/bin/bash
# scripts/generate_test_project.sh

generate_project() {
    local name=$1
    local file_count=$2
    local dir="test/perf/$name"
    
    mkdir -p "$dir/src"
    
    for i in $(seq 1 $file_count); do
        cat > "$dir/src/file_$i.ts" << EOF
export class Service$i {
    private value: number = $i;
    
    process(): number {
        return this.calculate(this.value);
    }
    
    private calculate(x: number): number {
        return x * 2;
    }
    
    callOther(): void {
        const s = new Service$((i % 10 + 1))();
        s.process();
    }
}
EOF
    done
    
    # Create index that imports all
    echo "// Auto-generated index" > "$dir/src/index.ts"
    for i in $(seq 1 $file_count); do
        echo "export * from './file_$i';" >> "$dir/src/index.ts"
    done
    
    # Create tsconfig
    cat > "$dir/tsconfig.json" << EOF
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "commonjs",
    "strict": true,
    "outDir": "./dist"
  },
  "include": ["src/**/*"]
}
EOF
}

generate_project "typescript_small" 100
generate_project "typescript_medium" 1000
generate_project "typescript_large" 5000
```

---

## 3. Timing Tests

### 3.1 LSP Startup Timing

```rust
// tests/perf/lsp_startup.rs

#[tokio::test]
async fn measure_lsp_startup_time() {
    let projects = vec![
        ("small", "test/perf/typescript_small"),
        ("medium", "test/perf/typescript_medium"),
        ("large", "test/perf/typescript_large"),
    ];
    
    for (name, path) in projects {
        let start = Instant::now();
        
        let lsp = TypeScriptLspClient::start(Path::new(path)).await.unwrap();
        lsp.wait_for_ready().await.unwrap();
        
        let duration = start.elapsed();
        
        println!("LSP startup ({}): {:?}", name, duration);
        
        // Record for tracking
        record_metric("lsp_startup", name, duration.as_millis() as u64);
        
        lsp.shutdown().await.unwrap();
    }
}
```

### 3.2 Single Query Timing

```rust
#[tokio::test]
async fn measure_single_query_latency() {
    let lsp = TypeScriptLspClient::start(Path::new("test/perf/typescript_medium")).await.unwrap();
    lsp.wait_for_ready().await.unwrap();
    
    let mut latencies = Vec::new();
    
    // Sample 100 random call sites
    for _ in 0..100 {
        let start = Instant::now();
        
        let _ = lsp.go_to_definition(
            Path::new("test/perf/typescript_medium/src/file_50.ts"),
            10, // line
            15, // column
        ).await;
        
        latencies.push(start.elapsed().as_millis() as u64);
    }
    
    // Calculate statistics
    latencies.sort();
    let p50 = latencies[50];
    let p95 = latencies[95];
    let p99 = latencies[99];
    let avg = latencies.iter().sum::<u64>() / 100;
    
    println!("Query latency: avg={}ms p50={}ms p95={}ms p99={}ms", avg, p50, p95, p99);
    
    // Assertions
    assert!(p99 < 500, "p99 latency should be under 500ms");
}
```

### 3.3 Batch Query Timing

```rust
#[tokio::test]
async fn measure_batch_throughput() {
    let lsp = TypeScriptLspClient::start(Path::new("test/perf/typescript_medium")).await.unwrap();
    lsp.wait_for_ready().await.unwrap();
    
    let batch_sizes = vec![1, 5, 10, 20, 50];
    
    for batch_size in batch_sizes {
        let queries: Vec<_> = (0..batch_size)
            .map(|i| (format!("test/perf/typescript_medium/src/file_{}.ts", i + 1), 10u32, 15u32))
            .collect();
        
        let start = Instant::now();
        
        // Send all queries in parallel
        let futures: Vec<_> = queries.iter()
            .map(|(file, line, col)| lsp.go_to_definition(Path::new(file), *line, *col))
            .collect();
        
        let _ = futures::future::join_all(futures).await;
        
        let duration = start.elapsed();
        let per_query = duration.as_millis() as f64 / batch_size as f64;
        
        println!("Batch {}: total={}ms per_query={:.1}ms", batch_size, duration.as_millis(), per_query);
    }
}
```

### 3.4 End-to-End Timing

```rust
#[tokio::test]
async fn measure_e2e_processing() {
    let projects = vec![
        ("small", "test/perf/typescript_small", 30),    // 30 second target
        ("medium", "test/perf/typescript_medium", 180), // 3 minute target
    ];
    
    for (name, path, target_seconds) in projects {
        let start = Instant::now();
        
        let result = analyze_typescript(Path::new(path)).await.unwrap();
        
        let duration = start.elapsed();
        
        println!(
            "E2E ({}): {:?} | {} nodes, {} edges",
            name,
            duration,
            result.nodes.len(),
            result.edges.len()
        );
        
        // Verify within target
        assert!(
            duration.as_secs() < target_seconds,
            "{} took {} seconds, target was {} seconds",
            name,
            duration.as_secs(),
            target_seconds
        );
    }
}
```

---

## 4. Memory Profiling

### 4.1 Memory Usage Script

```bash
#!/bin/bash
# scripts/measure_memory.sh

PROJECT=$1
OUTPUT_FILE="perf_results/memory_${PROJECT}.txt"

# Run with memory tracking
/usr/bin/time -v cargo run -p graphengine-parsing --release -- parse \
    --root "test/perf/${PROJECT}" \
    --lang typescript \
    --db /tmp/test.db \
    2>&1 | tee "$OUTPUT_FILE"

# Extract peak memory
grep "Maximum resident set size" "$OUTPUT_FILE"
```

### 4.2 Memory Targets

| Project Size | Target Peak Memory |
|--------------|-------------------|
| 100 files | < 500 MB |
| 1,000 files | < 1 GB |
| 5,000 files | < 2 GB |

---

## 5. Regression Testing

### 5.1 Performance Baseline

Store baselines in version control:

```json
// perf_baselines/typescript.json
{
  "version": "1.0.0",
  "date": "2025-02-05",
  "baselines": {
    "lsp_startup_small": {"ms": 3200, "tolerance": 0.2},
    "lsp_startup_medium": {"ms": 8500, "tolerance": 0.2},
    "query_latency_p99": {"ms": 250, "tolerance": 0.3},
    "e2e_small": {"seconds": 15, "tolerance": 0.2},
    "e2e_medium": {"seconds": 120, "tolerance": 0.2}
  }
}
```

### 5.2 CI Performance Check

```rust
#[test]
fn check_performance_regression() {
    let baselines = load_baselines("perf_baselines/typescript.json");
    let current = run_performance_tests();
    
    for (metric, baseline) in &baselines {
        let current_value = current.get(metric).unwrap();
        let tolerance = baseline.tolerance;
        let max_allowed = (baseline.value as f64 * (1.0 + tolerance)) as u64;
        
        assert!(
            *current_value <= max_allowed,
            "Performance regression in {}: current={} max_allowed={}",
            metric,
            current_value,
            max_allowed
        );
    }
}
```

---

## 6. Optimization Experiments

### 6.1 Experiment Template

```rust
/// Compare sequential vs parallel resolution
#[tokio::test]
async fn experiment_parallel_speedup() {
    let project = "test/perf/typescript_medium";
    
    // Baseline: Sequential
    let start = Instant::now();
    let sequential_result = analyze_sequential(project).await.unwrap();
    let sequential_time = start.elapsed();
    
    // Experiment: Parallel (4 workers)
    let start = Instant::now();
    let parallel_result = analyze_parallel(project, 4).await.unwrap();
    let parallel_time = start.elapsed();
    
    // Verify correctness
    assert_eq!(
        sequential_result.edges.len(),
        parallel_result.edges.len(),
        "Parallel must produce same result as sequential"
    );
    
    // Measure speedup
    let speedup = sequential_time.as_secs_f64() / parallel_time.as_secs_f64();
    
    println!("Sequential: {:?}", sequential_time);
    println!("Parallel (4): {:?}", parallel_time);
    println!("Speedup: {:.2}x", speedup);
    
    // Expect at least 2x speedup with 4 workers
    assert!(speedup > 2.0, "Expected at least 2x speedup");
}
```

### 6.2 Cache Effectiveness

```rust
#[tokio::test]
async fn experiment_cache_hit_rate() {
    let project = "test/perf/typescript_medium";
    
    // First run: cold cache
    let (result1, stats1) = analyze_with_stats(project).await.unwrap();
    
    // Second run: warm cache
    let (result2, stats2) = analyze_with_stats(project).await.unwrap();
    
    println!("Cold cache: {} queries, {} hits ({:.1}%)",
        stats1.total_queries,
        stats1.cache_hits,
        stats1.cache_hits as f64 / stats1.total_queries as f64 * 100.0
    );
    
    println!("Warm cache: {} queries, {} hits ({:.1}%)",
        stats2.total_queries,
        stats2.cache_hits,
        stats2.cache_hits as f64 / stats2.total_queries as f64 * 100.0
    );
    
    // Expect high hit rate on second run
    let hit_rate = stats2.cache_hits as f64 / stats2.total_queries as f64;
    assert!(hit_rate > 0.9, "Expected >90% cache hit rate on warm run");
}
```

---

## 7. Continuous Monitoring

### 7.1 Metrics to Track Over Time

- E2E processing time (per project size)
- LSP startup time
- Query latency (p50, p95, p99)
- Memory usage
- Cache hit rate
- Resolution accuracy

### 7.2 Dashboard Data Format

```json
// Output from each CI run
{
  "timestamp": "2025-02-05T10:30:00Z",
  "commit": "abc123",
  "metrics": {
    "e2e_small_ms": 14500,
    "e2e_medium_ms": 118000,
    "lsp_startup_ms": 8200,
    "query_p99_ms": 180,
    "memory_mb": 890,
    "cache_hit_rate": 0.34,
    "accuracy": 1.0
  }
}
```

---

## 8. Commands Reference

```bash
# Generate test projects
./scripts/generate_test_project.sh

# Run all performance tests
cargo test -p graphengine-parsing --test perf -- --nocapture

# Run specific benchmark
cargo test -p graphengine-parsing measure_e2e_processing -- --nocapture

# Run with memory profiling
./scripts/measure_memory.sh typescript_medium

# Compare against baseline
cargo test -p graphengine-parsing check_performance_regression

# Run optimization experiment
cargo test -p graphengine-parsing experiment_parallel_speedup -- --nocapture
```

---

## 9. Expected Results Summary

| Metric | Small (100) | Medium (1K) | Large (5K) |
|--------|-------------|-------------|------------|
| LSP Startup | < 5s | < 15s | < 45s |
| E2E Time | < 30s | < 3min | < 15min |
| Memory | < 500MB | < 1GB | < 2GB |
| Query p99 | < 200ms | < 300ms | < 500ms |
| Accuracy | 100% | 100% | 100% |

These targets should be achievable with TypeScript. Rust will have different (slower) targets.
