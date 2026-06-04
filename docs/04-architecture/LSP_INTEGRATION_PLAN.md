# LSP Full Integration Architecture Plan

## Executive Summary

This document outlines the architecture for a **deterministic, accurate** code graph system that uses:
- **LSP** for semantic accuracy (type resolution, method binding)
- **Tree-sitter** for fast structural extraction (containment, symbol discovery)
- **Progressive loading** with real-time progress indicators for large codebases

**Goal**: Achieve maximum accuracy while providing responsive UX, even for codebases with 8,000+ files.

---

## 1. Performance Reality Check

### 1.1 Measured Baselines

| Operation | Time (approx) | Notes |
|-----------|---------------|-------|
| rust-analyzer startup | 30s - 2min | Depends on crate graph complexity |
| typescript-language-server startup | 5s - 30s | Much faster |
| Single "go to definition" | 50-200ms | After warmup |
| Single "hover" request | 50-200ms | Type information |
| Tree-sitter parse (1 file) | 1-10ms | Very fast |

### 1.2 Big O Analysis for 8,000 File Codebase

**Worst Case (Naive Sequential)**:
```
Files: 8,000
Call sites per file: ~50 (conservative)
Total call sites: 400,000
LSP calls per site: 1 (go-to-definition)
Time per LSP call: 150ms

Total time: 400,000 × 150ms = 60,000 seconds = 16.7 hours
```

**Optimized Case**:
```
Tree-sitter unique resolution: 60% (240,000 sites) → 0ms each
LSP batching: 10 requests/batch → 15ms per request effective
Parallelization: 4 workers → 4x speedup
Caching: 30% hit rate → 30% reduction

Remaining LSP calls: 160,000 × 0.7 = 112,000
Time: 112,000 × 15ms / 4 workers = 420 seconds = 7 minutes
```

**Strategy**: Get from 16 hours → 7 minutes through intelligent optimization.

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         USER INTERFACE                               │
│  ┌─────────────────────────────────────────────────────────────────┐│
│  │  Progress: [████████░░░░░░░░] 52%  ETA: 3:24                   ││
│  │  Phase: Semantic Resolution   Files: 4,160/8,000               ││
│  │  Current: src/services/auth.ts                                  ││
│  └─────────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      ORCHESTRATION LAYER                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌────────────┐ │
│  │   Phase 1   │→ │   Phase 2   │→ │   Phase 3   │→ │   Phase 4  │ │
│  │  Structure  │  │  Semantics  │  │  Enrichment │  │  Storage   │ │
│  │(Tree-sitter)│  │    (LSP)    │  │  (Derived)  │  │  (SQLite)  │ │
│  └─────────────┘  └─────────────┘  └─────────────┘  └────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     LSP CONNECTION POOL                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌────────────┐ │
│  │  Worker 1   │  │  Worker 2   │  │  Worker 3   │  │  Worker 4  │ │
│  │ (LSP Conn)  │  │ (LSP Conn)  │  │ (LSP Conn)  │  │ (LSP Conn) │ │
│  └─────────────┘  └─────────────┘  └─────────────┘  └────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 3. Phase-by-Phase Design

### Phase 1: Structural Extraction (Tree-sitter)

**Duration**: ~1-2 seconds per 1,000 files
**Parallelizable**: YES (embarrassingly parallel)

```rust
pub struct Phase1Result {
    pub files: Vec<ParsedFile>,
    pub symbols: Vec<Symbol>,           // Functions, classes, etc.
    pub call_sites: Vec<CallSite>,      // Unresolved calls
    pub containment: Vec<ContainmentEdge>,
}
```

**Progress Tracking**:
```json
{
  "phase": "structural",
  "files_total": 8000,
  "files_complete": 4200,
  "current_file": "src/services/auth.ts",
  "elapsed_ms": 2340,
  "estimated_remaining_ms": 2100
}
```

**What Tree-sitter CAN resolve (skip LSP for these)**:
- Direct function calls with unique names
- Constructor calls: `new ClassName()`
- Scoped calls: `Module::function()`
- Import-to-file relationships

### Phase 2: Semantic Resolution (LSP)

**Duration**: Variable - depends on ambiguity rate
**Parallelizable**: LIMITED (see section 4)

**Only use LSP for**:
1. Method calls where receiver type is unknown
2. Overloaded function names
3. Trait method resolution
4. Generic type instantiation

```rust
pub struct SemanticQuery {
    pub call_site: CallSite,
    pub query_type: QueryType,
    pub priority: Priority,
}

pub enum QueryType {
    GoToDefinition,  // Primary - find where method is defined
    Hover,           // Fallback - get type information
    References,      // Validation - confirm bidirectional link
}

pub enum Priority {
    Critical,  // Method call with 5+ candidates
    High,      // Method call with 2-4 candidates  
    Low,       // Validation only
}
```

**Progress Tracking**:
```json
{
  "phase": "semantic",
  "total_queries": 45000,
  "resolved": 23400,
  "failed": 120,
  "cached_hits": 8900,
  "current_file": "src/services/auth.ts",
  "current_symbol": "authenticate",
  "lsp_queue_depth": 42,
  "workers_active": 4,
  "avg_response_ms": 87
}
```

### Phase 3: Enrichment (Derived Edges)

**Duration**: ~10-30 seconds total
**Parallelizable**: YES

Compute derived relationships:
- Transitive containment
- Call graph metrics (fan-in, fan-out)
- Coupling/cohesion scores
- Pattern detection

### Phase 4: Storage

**Duration**: ~5-15 seconds total
**Parallelizable**: BATCH WRITES

Persist to SQLite with transactions.

---

## 4. LSP Parallelization Strategy

### 4.1 The Problem

LSP servers are typically **single-threaded** internally. Sending 100 parallel requests to one server doesn't make it faster - it just queues them.

### 4.2 The Solution: Sharded Workers

```
┌─────────────────────────────────────────────────────────────┐
│                    FILE DISTRIBUTION                         │
│  Files 0-1999    Files 2000-3999   Files 4000-5999   ...    │
│       ↓                ↓                 ↓                   │
│   Worker 1          Worker 2          Worker 3              │
│  (LSP Instance)    (LSP Instance)   (LSP Instance)          │
│       ↓                ↓                 ↓                   │
│   Queue 1           Queue 2           Queue 3               │
└─────────────────────────────────────────────────────────────┘
```

**Key insight**: Each worker gets its own LSP server instance. Files are **sharded** so each worker handles a subset.

### 4.3 Request Batching

Instead of:
```
Request 1 → Wait 150ms → Response 1
Request 2 → Wait 150ms → Response 2
Request 3 → Wait 150ms → Response 3
Total: 450ms
```

Do:
```
Batch[Request 1, 2, 3] → Wait 180ms → [Response 1, 2, 3]
Total: 180ms
```

LSP supports concurrent requests. Send batches of 10-20 requests, await all.

### 4.4 Avoiding Race Conditions

```rust
pub struct LspWorker {
    id: usize,
    file_range: Range<usize>,  // Only process files in this range
    lsp_client: LspClient,
    request_counter: AtomicU64,
    pending_requests: DashMap<u64, oneshot::Sender<Response>>,
}

impl LspWorker {
    /// Guarantee: Each file is processed by exactly ONE worker
    /// No two workers will ever query the same file simultaneously
    pub fn owns_file(&self, file_idx: usize) -> bool {
        self.file_range.contains(&file_idx)
    }
    
    /// Send batch of requests, await all responses
    /// Correlation IDs prevent response mix-ups
    pub async fn batch_query(&self, queries: Vec<Query>) -> Vec<Result<Response>> {
        let mut futures = Vec::with_capacity(queries.len());
        
        for query in queries {
            let id = self.request_counter.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = oneshot::channel();
            self.pending_requests.insert(id, tx);
            self.lsp_client.send(query.with_id(id));
            futures.push(rx);
        }
        
        // Await all responses (order preserved by correlation IDs)
        join_all(futures).await
    }
}
```

---

## 5. Caching Strategy

### 5.1 Multi-Level Cache

```
┌─────────────────────────────────────────────────────────────┐
│  L1: In-Memory (per-session)                                │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Key: (file_path, line, column, query_type)            ││
│  │  Value: Resolution result                               ││
│  │  TTL: Session lifetime                                  ││
│  │  Size: ~100MB max                                       ││
│  └─────────────────────────────────────────────────────────┘│
│                           │ Miss                            │
│                           ▼                                 │
│  L2: SQLite (persistent)                                    │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Key: (file_hash, symbol_hash, query_type)             ││
│  │  Value: Resolution result                               ││
│  │  TTL: Until file content changes                        ││
│  └─────────────────────────────────────────────────────────┘│
│                           │ Miss                            │
│                           ▼                                 │
│  L3: LSP Query (expensive)                                  │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 Cache Invalidation

```rust
pub struct CacheKey {
    file_content_hash: u64,  // SHA256 of file content
    symbol_fqn: String,
    query_type: QueryType,
}

// When file changes:
// 1. Compute new file_content_hash
// 2. All cache entries with old hash become invalid
// 3. No explicit deletion needed - just won't match
```

---

## 6. Progress Reporting Protocol

### 6.1 Event Stream Format

```json
// Emitted to stdout as JSON lines
{"type":"phase_start","phase":"structural","timestamp":1706900000000}
{"type":"file_start","file":"src/auth.ts","index":0,"total":8000}
{"type":"file_complete","file":"src/auth.ts","symbols":42,"calls":18,"ms":12}
{"type":"phase_complete","phase":"structural","files":8000,"symbols":145000,"ms":4200}

{"type":"phase_start","phase":"semantic","timestamp":1706900004200}
{"type":"batch_start","batch_id":1,"queries":20,"worker":0}
{"type":"query_complete","batch_id":1,"query_id":3,"target":"Auth::login","ms":87}
{"type":"batch_complete","batch_id":1,"resolved":18,"failed":2,"ms":340}
{"type":"progress","phase":"semantic","complete":23400,"total":45000,"eta_ms":180000}
```

### 6.2 Frontend Progress Display

```
┌────────────────────────────────────────────────────────────────┐
│  GraphEngine Analysis                                          │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  Phase 1: Structural Analysis          ✓ Complete (4.2s)      │
│  ────────────────────────────────────────────────────────────  │
│  [████████████████████████████████████] 100% (8,000 files)    │
│                                                                │
│  Phase 2: Semantic Resolution          ⟳ In Progress          │
│  ────────────────────────────────────────────────────────────  │
│  [████████████████░░░░░░░░░░░░░░░░░░░░] 52% (23,400/45,000)   │
│                                                                │
│  Current: src/services/payment/stripe.ts                       │
│  LSP Workers: 4 active | Queue: 42 pending                    │
│  Cache Hit Rate: 34%                                           │
│  ETA: 3:24 remaining                                           │
│                                                                │
│  Phase 3: Enrichment                   ○ Pending              │
│  Phase 4: Storage                      ○ Pending              │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

---

## 7. Guaranteed Correctness Before Speed

### 7.1 Sequential Baseline (Always Works)

```rust
/// Guaranteed correct - no race conditions possible
/// Use this first, optimize later
pub async fn analyze_sequential(project: &Path) -> Result<Graph> {
    let lsp = LspClient::start(project).await?;
    lsp.wait_for_ready().await?;  // Wait for full initialization
    
    let mut graph = Graph::new();
    
    // Phase 1: Tree-sitter (already fast)
    let syntax = extract_syntax_sequential(project)?;
    
    // Phase 2: LSP (slow but correct)
    for call_site in &syntax.call_sites {
        if needs_lsp_resolution(call_site, &syntax) {
            let result = lsp.go_to_definition(call_site).await?;
            graph.add_edge(call_site.caller, result.target);
        }
    }
    
    Ok(graph)
}
```

### 7.2 Parallel Optimization (Same Results, Faster)

Only after sequential works perfectly:

```rust
pub async fn analyze_parallel(project: &Path, workers: usize) -> Result<Graph> {
    // Same result as sequential, just faster
    let syntax = extract_syntax_parallel(project)?;  // Embarrassingly parallel
    
    // Shard files across workers
    let file_chunks = syntax.files.chunks(syntax.files.len() / workers);
    
    // Each worker gets its own LSP instance
    let handles: Vec<_> = file_chunks
        .enumerate()
        .map(|(i, chunk)| {
            tokio::spawn(async move {
                let lsp = LspClient::start(project).await?;
                resolve_chunk(lsp, chunk).await
            })
        })
        .collect();
    
    // Merge results (order-independent)
    let results = join_all(handles).await;
    merge_graphs(results)
}
```

---

## 8. Testing & Metrics Framework

### 8.1 Performance Benchmarks

```rust
#[cfg(test)]
mod benchmarks {
    /// Baseline: How fast is tree-sitter alone?
    #[bench]
    fn bench_treesitter_parse_1000_files(b: &mut Bencher) {
        // Target: < 2 seconds
    }
    
    /// LSP cold start time
    #[bench]
    fn bench_lsp_startup_time(b: &mut Bencher) {
        // Target: < 30 seconds for TypeScript
    }
    
    /// Single go-to-definition latency
    #[bench]
    fn bench_lsp_single_query(b: &mut Bencher) {
        // Target: < 200ms p99
    }
    
    /// Batch query throughput
    #[bench]
    fn bench_lsp_batch_10_queries(b: &mut Bencher) {
        // Target: < 500ms for batch of 10
    }
    
    /// End-to-end for TypeScript project
    #[bench]
    fn bench_e2e_typescript_1000_files(b: &mut Bencher) {
        // Target: < 5 minutes total
    }
}
```

### 8.2 Correctness Tests

```rust
#[cfg(test)]
mod correctness {
    /// Sequential and parallel must produce identical results
    #[test]
    fn test_parallel_matches_sequential() {
        let sequential = analyze_sequential(TEST_PROJECT);
        let parallel = analyze_parallel(TEST_PROJECT, 4);
        assert_eq!(sequential.edges, parallel.edges);
    }
    
    /// Known test case: method call resolution
    #[test]
    fn test_method_call_resolves_correctly() {
        // auth.login() should resolve to Auth::login, not UserAuth::login
        let graph = analyze(TEST_PROJECT);
        let edge = graph.find_edge("main", "Auth::login");
        assert!(edge.is_some());
    }
}
```

---

## 9. Language Progression Plan

| Phase | Language | Complexity | Target Duration |
|-------|----------|------------|-----------------|
| 1 | TypeScript | Low | 2-3 weeks |
| 2 | Python | Medium | 2-3 weeks |
| 3 | Java/C# | Medium | 2-3 weeks |
| 4 | Rust | High | 4-6 weeks |

### 9.1 Why TypeScript First

1. **Fast LSP**: typescript-language-server is 3-5x faster than rust-analyzer
2. **Simpler types**: Class-based, explicit, less inference
3. **Better tooling**: Mature ecosystem, predictable behavior
4. **Large market**: More potential users to validate product

### 9.2 Graduation Criteria

Before moving to next language:
- [ ] 100% resolution accuracy on test corpus
- [ ] < 10 minute processing for 5,000 file project
- [ ] Parallel mode produces identical results to sequential
- [ ] Progress reporting works end-to-end
- [ ] Cache invalidation correct on file changes

---

## 10. Implementation Milestones

### Milestone 1: Sequential TypeScript (Week 1-2)
- [ ] Tree-sitter extraction for TypeScript
- [ ] LSP client with go-to-definition
- [ ] Sequential resolution loop
- [ ] Basic progress events

### Milestone 2: Parallel TypeScript (Week 3)
- [ ] Worker pool implementation
- [ ] File sharding
- [ ] Batch request handling
- [ ] Correctness tests (parallel == sequential)

### Milestone 3: Caching & Performance (Week 4)
- [ ] In-memory cache (L1)
- [ ] Persistent cache (L2)
- [ ] Performance benchmarks
- [ ] Cache invalidation on file change

### Milestone 4: Production Polish (Week 5-6)
- [ ] Detailed progress reporting
- [ ] Error recovery
- [ ] Large project testing (5,000+ files)
- [ ] Documentation

---

## 11. Parser Accuracy & Scope Detection (Future Improvements)

### 11.1 Current Tree-sitter Limitations (FIXED in PR #xxx)

**Issue**: Tree-sitter queries can capture unintended AST nodes:

1. **Anonymous callbacks captured as functions**: Arrow functions used as callbacks in `map()`, `filter()`, etc. were being extracted as top-level functions without names.

2. **Keyword names from file paths**: Files named after keywords (e.g., `if.ts`) would contribute the keyword to FQNs like `v3::types::if::someFunc`.

3. **No containment validation**: Nodes could exist without proper parent relationships, creating disconnected visualization.

**Solutions Implemented**:
- JavaScript function queries now require assignment to named variables
- Reserved keyword filter blocks invalid symbol names (`if`, `for`, `while`, etc.)
- Containment validation warns about orphaned nodes

### 11.2 Future LSP-Based Scope Detection

**Vision**: Use LSP for accurate scope and containment detection where Tree-sitter is insufficient.

#### Control Flow as Containable Scopes

Consider extracting control flow structures as scope containers (not as Functions):

```typescript
// Source file: auth.ts
function authenticate(user: User): boolean {
    if (user.isAdmin) {           // Branch node: auth::authenticate::if@line12
        return true;              // Logic contained within if scope
    }
    for (const role of roles) {   // Branch node: auth::authenticate::for@line15
        if (role.allows(action)) { // Nested branch: auth::authenticate::for::if@line16
            return true;
        }
    }
    return false;
}
```

**Potential NodeKind extensions**:
- `NodeKind::Branch` - for if/switch/match statements
- `NodeKind::Loop` - for for/while/loop statements
- `NodeKind::TryBlock` - for try/catch/finally

**Benefits**:
- Visualize code complexity (nested conditionals = high cyclomatic complexity)
- Track exception handling paths
- Show branch coverage in testing context

#### LSP Provides Accurate Type Context

Tree-sitter cannot determine:
- Which overload is being called
- Receiver types for method calls (`obj.method()` - what type is `obj`?)
- Generic instantiation

LSP `hover` and `typeDefinition` requests provide this:

```json
// Request: hover at line 42, character 10
{
  "method": "textDocument/hover",
  "params": {"textDocument": {"uri": "file:///auth.ts"}, "position": {"line": 42, "character": 10}}
}

// Response: Type information
{
  "result": {
    "contents": "(method) AuthService.login(user: User): Promise<Session>"
  }
}
```

### 11.3 Entry Point Detection

**Problem**: Functions like `HeroLogo` (React component) or `zod3` (benchmark) appear disconnected because they're called by external frameworks.

**Future Solution**: Mark entry points based on conventions:

| Pattern | Entry Point Type | Detection Method |
|---------|-----------------|------------------|
| `export default function` | Framework entry | Tree-sitter export query |
| `@Component` decorator | Framework entry | Decorator query |
| `describe()` / `it()` | Test entry | Call pattern matching |
| `bench()` / `benchmark()` | Benchmark entry | Call pattern matching |
| `main()` function | Program entry | Name matching |
| `__init__.py` symbols | Module entry | File name matching |

**Implementation Sketch**:
```rust
pub enum EntryPointKind {
    FrameworkExport,    // export default, @Component
    TestCase,           // describe/it/test
    BenchmarkCase,      // bench/benchmark
    ProgramMain,        // main()
    ModuleInit,         // __init__.py
}

pub struct EntryPoint {
    pub node_id: String,
    pub kind: EntryPointKind,
    pub confidence: Confidence,
}
```

### 11.4 Validation Improvements Roadmap

| Priority | Improvement | Effort | Impact |
|----------|-------------|--------|--------|
| P0 (Done) | Filter keyword names | 1 day | Prevents parser bugs |
| P0 (Done) | Validate containment | 1 day | Catches orphan nodes |
| P1 | Entry point detection | 3 days | Explains "orphan" functions |
| P2 | Branch/loop extraction | 1 week | Complexity visualization |
| P3 | Full LSP scope resolution | 2 weeks | Maximum accuracy |

---

## Appendix A: LSP Protocol Reference

### Go to Definition
```json
// Request
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "textDocument/definition",
  "params": {
    "textDocument": {"uri": "file:///path/to/file.ts"},
    "position": {"line": 42, "character": 15}
  }
}

// Response
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "uri": "file:///path/to/target.ts",
    "range": {"start": {"line": 10, "character": 0}, "end": {"line": 10, "character": 20}}
  }
}
```

### Hover (Type Information)
```json
// Request
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "textDocument/hover",
  "params": {
    "textDocument": {"uri": "file:///path/to/file.ts"},
    "position": {"line": 42, "character": 15}
  }
}

// Response
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "```typescript\n(method) Auth.login(username: string): Promise<User>\n```"
    }
  }
}
```

---

## Appendix B: Complexity Estimates

| Operation | Time Complexity | Space Complexity |
|-----------|-----------------|------------------|
| Tree-sitter parse | O(n) per file | O(n) AST nodes |
| Symbol index build | O(n log n) | O(n) symbols |
| LSP query (single) | O(1) amortized | O(1) |
| LSP query (batch k) | O(k) | O(k) |
| Cache lookup | O(1) hash | O(n) entries |
| Graph storage | O(E + V) | O(E + V) |

Where:
- n = lines of code
- E = number of edges
- V = number of vertices (symbols)
