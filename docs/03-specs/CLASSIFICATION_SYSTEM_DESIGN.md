# Classification System Design

> Engine team reference for the three-layer production vs. non-production
> classification system in `ge-analyze`.

Last updated: 2026-02-28

---

## 1. Problem Statement

No deterministic heuristic can classify all files as production vs.
non-production with 100% accuracy across all possible codebases. A testing-tools
company has `test-runner/` as production code. A medical system has `lab-test/`
as production. The engine must handle these edge cases gracefully while still
being correct for the vast majority of codebases without user intervention.

**Design goal:** ~98% correct on auto-classification (no user input),
100% correct after user verification.

---

## 2. Three-Layer Architecture

```
Layer 3: User Config (TOML overrides)     ← highest priority, definitive
Layer 2: Structural Detection              ← import graph, is_test flags
Layer 1: Path Heuristic (word-boundary)    ← lowest priority, broadest coverage
```

Each layer can override the layer below it. The final classification includes
the source and reason for auditability.

### Layer 1: Path Heuristic

**File:** `graphengine-analysis/src/health/path_classification.rs`

Uses **word-boundary matching** on path components. A word boundary is defined
as: start/end of string, or one of `-`, `_`, `.`.

#### Test Patterns

| Pattern Type | Examples Matched | Examples NOT Matched |
|-------------|-----------------|---------------------|
| Dunder directories | `__tests__/`, `__mocks__/` | — |
| File name infixes | `.test.ts`, `.spec.js`, `_test.go` | — |
| File name prefix | `test_auth.py` | `testable.py` (no boundary) |
| Directory word-boundary | `test/`, `tests/`, `runtime-tests/`, `e2e-test/`, `unit_tests/` | `contest/`, `testimony/`, `attest/` |

#### Auxiliary Patterns

| Category | Words Matched (at word boundary) |
|----------|--------------------------------|
| Examples | `example`, `examples` |
| Benchmarks | `bench`, `benchmark`, `benchmarks` |
| Fixtures | `fixture`, `fixtures`, `mock`, `mocks`, `stub`, `stubs` |
| Demos | `demo`, `demos`, `sample`, `samples` |
| Documentation | `docs`, `doc` |
| Test data | `testdata`, `e2e` |

#### Tooling Dotfiles

Directories starting with `.vitest`, `.jest`, `.storybook`, `.husky`,
`.cypress`, `.playwright`, `.nyc`, `.coverage` are classified as tooling/config.

#### Word Boundary Algorithm

```rust
fn contains_word_boundary(component: &str, word: &str) -> bool
```

For each position in `component` where `word` appears as a substring:
1. Check left: position is 0, or `component[pos-1]` is one of `-`, `_`, `.`
2. Check right: end position equals string length, or `component[end]` is
   one of `-`, `_`, `.`
3. Only match if BOTH boundaries are satisfied

This is O(n) per component and runs after lowercasing, so it's
case-insensitive.

### Layer 2: Structural Detection

**File:** `graphengine-analysis/src/health/structural_classification.rs`

Uses the import graph to detect test files. If a file imports a known test
framework module (e.g., `pytest`, `jest`, `testing` for Go), its nodes get
`is_test = true`. This propagates to File nodes during graph construction.

The centralized helper `AnalysisGraph::is_non_production_node()` checks:
1. Node's own `is_test`, `is_vendor`, `is_generated`, `is_build_output` flags
2. Parent File node's flags (via containment walk)
3. Path heuristic on `file_path` and `path_repo_rel`

### Layer 3: User Config (TOML Overrides)

**File:** `graphengine-analysis/src/health/config.rs`

Users provide a TOML file with:

```toml
[classification]
production_paths = ["src/test-runner/"]
non_production_paths = ["scripts/internal/"]
```

These are prefix-matched against module keys and override all heuristics.
Classifications from this layer get `confidence: "definitive"` and
`source: "user_config"`.

---

## 3. Centralized Helper

**File:** `graphengine-analysis/src/health/graph.rs`

```rust
pub fn is_non_production_node(&self, node_id: &str) -> bool
```

Single entry point that all finding generators use. Combines:
1. Structural flags from parent File node
2. Path heuristic on file's `path_repo_rel`
3. Path heuristic on file's `file_path`
4. Path heuristic on the node's own `file_path`
5. Path heuristic on the `node_id` itself (for module-key-style IDs)

Returns `true` if **any** signal says non-production. This is the conservative
choice: it's better to accidentally exclude a production module from a finding
(false negative) than to pollute findings with test code (false positive in
findings).

---

## 4. Classification Flow Through the Pipeline

```
1. Graph loaded from SQLite
   ├── Nodes have is_test/is_vendor/is_generated flags (from parser)
   └── Nodes have file_path, path_repo_rel (from parser)

2. Structural classification runs
   └── Propagates is_test to File nodes based on import graph

3. AnalysisConfig loaded
   └── classification_overrides populated from TOML (if provided)

4. Metrics computed
   ├── God function: skips nodes where graph.is_non_production_node() = true
   ├── Layer violation: skips violations where caller OR callee is non-production
   ├── API surface: skips non-production modules
   ├── Coupling findings: skips non-production modules
   ├── Coupling metrics: filters to production modules for avg/counts
   └── Cohesion: skips non-production modules

5. Classifications built (build_classifications)
   ├── For each module in folder_module_ids:
   │   ├── Check Layer 3 (user config) → if match, emit with definitive confidence
   │   ├── Check Layer 2 (structural) → if any descendant has is_test, emit
   │   └── Check Layer 1 (path) → classify_path() returns role + reason
   ├── Default: production with high confidence
   └── Each classification gets counts_toward_score = (role == "production")

6. Report emitted with classifications map
```

---

## 5. Confidence Tier Definitions

| Tier | Definition | When Assigned |
|------|-----------|--------------|
| `definitive` | Cannot be wrong given available information | User override (Layer 3), or structural Tier 1 (imports `pytest`) |
| `high` | Very likely correct, matches strong convention | Test directory at word boundary, `_test.go` file suffix |
| `medium` | Plausible but not certain | Example/benchmark directory, doc directory |
| `low` | Weak signal, most likely to need correction | Reserved for future ambiguous heuristics |

---

## 6. Known False-Positive Scenarios

| Scenario | Example | How Each Layer Handles It |
|----------|---------|--------------------------|
| Production code with "test" in name | `src/test-runner/main.py` | **Path:** false positive (classifies as test). **Structural:** may correct if no test imports. **User config:** definitive override. |
| Production code in `lab-test/` dir | `lab-test/analyzer.go` | **Path:** false positive. **Structural:** may correct. **User config:** definitive override. |
| Test code NOT in test directory | `src/utils/test_helpers.py` | **Path:** caught by `test_` prefix in filename. **Structural:** caught if imports test framework. |
| Vendor code | `vendor/lodash/index.js` | **Structural:** `is_vendor` flag from parser. **Path:** not caught (no test/aux signal). |
| Generated code | `generated/schema.rs` | **Structural:** `is_generated` flag from parser. |
| `contest` directory | `src/contest/entry.ts` | **Path:** NOT matched (word boundary prevents it — no boundary before 'test' in 'contest'). |
| `runtime-tests/` | `runtime-tests/node/index.ts` | **Path:** matched via word-boundary ('tests' after hyphen). |
| `.vitest.config` | `.vitest.config.ts` | **Path:** matched as tooling dotfile. |

---

## 7. Where Classification Is Consumed

| Consumer | File | What It Uses |
|----------|------|-------------|
| God function detection | `mod.rs` §13.5 | `graph.is_non_production_node(function_id)` |
| Layer violation findings | `mod.rs` §6.5 | `graph.is_non_production_node(caller_id/callee_id)` |
| API surface findings | `mod.rs` §11 | `graph.is_non_production_node(module_key)` |
| Coupling findings | `mod.rs` §5 | `graph.is_non_production_node(module_key)` |
| Coupling metrics summary | `mod.rs` summary | `graph.is_non_production_node(module_key)` |
| Cohesion analysis | `cohesion.rs` | `graph.is_non_production_node(module_key)` |
| Classification report | `mod.rs` `build_classifications()` | All three layers |

---

## 8. Test Plan

### Unit Tests (in `path_classification.rs`)

40 tests covering:
- Word boundary matching: exact match, hyphen/underscore/dot delimited
- Substring rejection: `contest`, `testimony`, `attest`, `protest`, `detest`, `testable`
- Test path detection: dunder dirs, file patterns, directory word-boundary
- Auxiliary path detection: examples, benchmarks, fixtures, demos, docs, dotfiles
- Classification function: returns correct role + reason
- Edge cases: empty paths, root files, backslash normalization

### Integration Tests (E2E suite — `test_e2e.py`)

| Repo | Language | What to Verify |
|------|----------|---------------|
| **chi** | Go | `_examples/*` classified as example, no god function/layer violation findings from `*_test.go` files |
| **express** | JavaScript | `examples/*` and `test/*` excluded, API surface findings show real module names |
| **requests** | Python | `tests/*` and `docs/*` excluded, no layer violations from test files |
| **hono** | TypeScript | `runtime-tests/*` caught by word-boundary, `benchmarks/*` caught, `.vitest.config` caught as config |
| **commander.js** | JavaScript | `tests/*` and `examples/*` excluded |

### Regression Checks

After any change to classification logic:
1. `cargo test -p graphengine-analysis --lib path_classification` — all 40 unit tests pass
2. `cargo test -p graphengine-analysis --lib` — all 133+ unit tests pass
3. `python3 test_e2e.py` — 0 inspection issues across all 5 repos
4. Spot-check `classifications` in JSON output for each repo

---

## 9. Future Considerations

- **Machine learning layer**: If heuristics prove insufficient for exotic codebases,
  a lightweight ML model trained on file content features could provide a
  `confidence: "medium"` signal between structural and path heuristic layers.
- **Language-specific structural rules**: Some languages have conventions that
  can be detected structurally (e.g., Go test files must be in `*_test.go`,
  Rust integration tests must be in `tests/`).
- **Classification diffing**: When a TOML override file exists and new modules
  appear in a re-scan, the UI should highlight them as "new, unverified."
  The engine already supports this by comparing the override's path lists
  against the current module set.
