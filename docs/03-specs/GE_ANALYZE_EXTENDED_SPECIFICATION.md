# ge-analyze: Extended Specification — New Metrics and Detection Systems

**Date:** 2026-02-27
**Status:** Specification for features beyond `GE_ANALYZE_FULL_SPECIFICATION.md`.
**Prerequisite:** Read `GE_ANALYZE_FULL_SPECIFICATION.md` first. This document extends it.

---

## 1. Purpose

This document specifies metrics and detection systems not covered in the original ge-analyze spec. It adds:

1. **Structural test/generated code detection** — graph-based system using import edges to test framework modules, not path/name matching
2. **Cyclomatic and cognitive complexity** — per-function complexity metrics (pulled forward from Phase 6)
3. **Temporal coupling** — co-change detection from git history (pulled forward from Phase 6)
4. **Module cohesion (LCOM)** — measures whether a module's internals are related
5. **Abstractness (Martin)** — ratio of abstract types to total types per module
6. **Distance from Main Sequence** — Martin's metric combining Abstractness + Instability
7. **God function detection** — composite finding for functions that do too much
8. **Automatic layer detection** — infer architectural layers from graph structure
9. **Function/file LOC** — lines of code from existing location data

These extend the original spec's Sections 4 (Algorithms), 5 (Health Score), and 6 (Output Format).

---

## 2. Structural Test and Generated Code Detection

### 2.1 The Problem

Test code included in production analysis distorts every metric:
- Test utility functions appear as "dead code" (nothing in production calls them)
- Heavily-tested functions get inflated fan-in (tests are callers)
- Test-to-production imports create false coupling
- Module coupling scores are distorted (test modules import production but not vice versa)
- Health score punishes well-tested codebases for having more "complexity"

The original spec (Section 3.1) defines `is_test`, `is_vendor`, `is_generated` properties on File nodes and `exclude_test_modules_from_coupling_avg` in ecosystem profiles. This section specifies a structurally-sound detection system and how exclusion works across ALL metrics.

### 2.2 Why Path/Name Matching Alone Is Wrong

A file named `test_runner.py` in a `tests/` directory is NOT necessarily test code:

- **A testing framework repo** (Jest, pytest, Vitest) has "test" everywhere — it's all production code for that project.
- **A test automation company** — their `TestRunner.java` IS the product.
- **A `test-utils` npm package** — it's a library consumed by other projects' tests, but it is production code for this repo.
- **A `TestableWidget` class** — part of a public API, not a test.
- **A `tests/` directory in a test framework repo** — exercises the framework (production code for that repo), would be incorrectly flagged.
- **An education repo called `testing-patterns`** — every file is "test"-related by name, none are actual test code.

Name and path patterns produce false positives in all of these scenarios. The correct primary signal is **structural**: what a file imports and who imports it.

### 2.3 Structural Detection System (Primary — Graph-Based)

**Core principle:** A file is test code if it **consumes** test framework APIs. A file that **defines** test APIs is production code. The import graph already captures this distinction.

#### Step 1 — Build the Test Framework Import Registry

Maintain a per-language list of known test framework module names. This list is small (~10-20 entries per language) and stable (new testing frameworks appear rarely — roughly 1 per year per ecosystem).

| Language | Known Test Framework Modules |
|---|---|
| TypeScript/JS | `jest`, `@jest/globals`, `vitest`, `mocha`, `chai`, `sinon`, `@testing-library/*`, `cypress`, `@playwright/test`, `supertest`, `nock`, `msw`, `jest-mock-extended`, `expect` (standalone), `node:test` |
| Python | `pytest`, `unittest`, `unittest.mock`, `mock`, `nose`, `nose2`, `hypothesis`, `responses`, `httpretty`, `factory_boy`, `faker`, `freezegun`, `pytest_mock` |
| Java/Kotlin | `org.junit.*`, `junit.*`, `org.testng.*`, `org.mockito.*`, `org.assertj.*`, `org.hamcrest.*`, `io.mockk.*`, `org.springframework.boot.test.*`, `org.springframework.test.*` |
| Go | `testing` (standard library) |
| Rust | (detected via attributes — see Step 4. No import-based detection.) |
| C# | `Xunit`, `Xunit.*`, `NUnit.*`, `Microsoft.VisualStudio.TestTools.UnitTesting`, `Moq`, `FluentAssertions`, `NSubstitute`, `Shouldly` |
| Swift | `XCTest` |
| Ruby | `rspec`, `rspec/*`, `minitest`, `minitest/*`, `test-unit` |
| PHP | `PHPUnit\Framework\*` |

This registry is a static lookup table. It ships with ge-analyze and is updatable via `--config` TOML.

#### Step 2 — Classify Direct Test Consumers (Tier 1)

For each File node in the graph, check: does it have any Import edge whose target resolves to a module in the test framework registry?

If yes → classify as `role: test`, `role_confidence: definitive`, `role_reason: "Imports test framework: <module>"`.

This is deterministic, structural, and handles the edge cases correctly:
- pytest repo's own `test_config.py` that imports `pytest` → **Test** (correct — it IS a test of pytest itself)
- pytest repo's `_pytest/config.py` that doesn't import `pytest` as a test consumer → **Production** (correct)
- `TestRunner.java` that defines JUnit APIs but doesn't import JUnit → **Production** (correct)

#### Step 3 — Classify Test Support Files (Tier 2)

For each File node NOT yet classified:
1. Collect all incoming Import edges (files that import this file)
2. If ALL importers are classified as test (Tier 1 or previous Tier 2 iteration) AND this file has at least one importer → classify as `role: test_support`, `role_confidence: high`, `role_reason: "Only imported by test files"`

Iterate until no more files can be reclassified (transitive closure). This catches:
- `conftest.py` (imported only by test files, but doesn't import pytest itself)
- `__tests__/helpers/mockDb.ts` (imported only by test files)
- Test fixture factories, custom matchers, shared setup utilities

**Critical:** If a file has even ONE production importer, it stays production. This correctly handles:
- `test-utils` npm package imported by production code → has production importers → **Production**
- Shared types used by both test and production → has mixed importers → **Production**

#### Step 4 — AST-Level Detection (Tier 3 — Inline Tests)

Some languages embed test code inside production files. These require AST inspection during parsing:

| Language | AST Signal | Classification Scope |
|---|---|---|
| Rust | `#[cfg(test)]` attribute on a `mod` node | Only that module block and its children. The rest of the file remains production. |
| Rust | `#[test]` attribute on a function | That function only (if outside a `cfg(test)` block). |
| Java/Kotlin | `@Test`, `@ParameterizedTest`, `@BeforeEach`, `@AfterEach`, `@BeforeAll`, `@AfterAll` annotations | That method. If ALL non-lifecycle methods in a class have `@Test`, classify the entire file. |
| C# | `[Fact]`, `[Theory]`, `[Test]`, `[TestMethod]` attributes | Same rule as Java. |
| Go | `func Test*(t *testing.T)` or `func Benchmark*(b *testing.B)` | Already caught by Tier 1 (these files import `testing`). AST confirms. |

Tier 3 is definitive for the scope it covers (the annotated block), not the whole file unless the whole file is annotated.

#### Step 5 — Path/Name Patterns as Corroboration Only (Tier 4 — Fallback)

Path and name patterns activate ONLY when structural analysis is inconclusive — specifically:
- The file has NO import edges at all (isolated/orphan file)
- The parser couldn't resolve import targets (unresolvable dynamic imports, aliased paths)

In this fallback case, classification requires BOTH conditions:
1. File matches a known test naming pattern for its language (`*.test.ts`, `*_test.go`, `test_*.py`, etc.)
2. The file is in a directory that ALSO contains at least one Tier 1 confirmed test file

Both conditions must be true. Path/name patterns alone NEVER classify a file as test.

| Language | Tier 4 Name Patterns (corroboration only) |
|---|---|
| TypeScript/JS | `*.test.ts`, `*.spec.ts`, `*.test.tsx`, `*.spec.tsx`, `*.test.js`, `*.spec.js` |
| Go | `*_test.go` |
| Python | `test_*.py`, `*_test.py` |
| Rust | Files in `tests/` directory (integration tests) |
| Java/Kotlin | `*Test.java`, `*Tests.java`, `*IT.java`, `*Spec.java`, `*Test.kt` |
| C# | `*Test.cs`, `*Tests.cs`, `*Spec.cs` |
| Swift | `*Tests.swift`, `*Test.swift` |
| Ruby | `*_test.rb`, `*_spec.rb` |
| PHP | `*Test.php` |

### 2.4 Confidence Model

| Tier | Signal | Confidence | Can Override? |
|---|---|---|---|
| **Tier 1** | File imports a known test framework module | Definitive | Only by user config `force_production_paths` |
| **Tier 2** | File is only imported by Tier 1/2 files | High | Only by user config `force_production_paths` |
| **Tier 3** | AST-level test attributes/decorators | Definitive (for the annotated scope) | No |
| **Tier 4** | Path/name pattern + structural corroboration | Medium | Yes — user config can override |
| **None** | No signal detected | Production (default) | N/A |

**The default is always production.** A file is production code unless proven otherwise by structural evidence. This prevents the "test framework repo" problem entirely.

### 2.5 Edge Cases and How They Resolve

| Scenario | Tier 1 | Tier 2 | Tier 3 | Tier 4 | Classification |
|---|---|---|---|---|---|
| `auth.test.ts` imports Jest | Yes | — | — | — | **Test** (Tier 1) |
| `conftest.py` — no framework import, only imported by test files | No | Yes | — | — | **Test support** (Tier 2) |
| Rust `mod tests { #[test] fn ... }` inside `auth.rs` | No | No | Yes | — | **Test** (scoped to mod block) |
| `TestRunner.java` in a test framework repo — defines APIs, no JUnit import | No | No | No | Name matches | **Production** (no structural signal) |
| `test-utils` npm package imported by production code | No | No (production importers) | No | Name matches | **Production** (Tier 2 fails) |
| pytest repo's `test_config.py` that imports `pytest` | Yes | — | — | — | **Test** (correctly — it tests pytest) |
| `TestableComponent.tsx` — production widget | No | No | No | No | **Production** |
| Orphan `old_test.js` — no imports, no importers | No | No | No | Corroborate | **Production** (default) or **Test** (only if sibling files are Tier 1 confirmed AND name matches) |
| Education repo `testing-patterns/` — all files named `test_*` | No (files don't import test frameworks) | No | No | No (no Tier 1 siblings) | **Production** (correct) |

### 2.6 Framework Configuration Detection (Ambient Context)

Detecting test framework config files in the repo root tells the system which frameworks are present. This doesn't classify files — it enriches Tier 1 by confirming which framework module names to expect in imports.

| Config File(s) | Framework | Language |
|---|---|---|
| `jest.config.*`, `jest.setup.*` | Jest | TS/JS |
| `vitest.config.*` | Vitest | TS/JS |
| `cypress.config.*` | Cypress | TS/JS |
| `playwright.config.*` | Playwright | TS/JS |
| `.mocharc.*` | Mocha | JS |
| `pytest.ini`, `pyproject.toml` (pytest section) | pytest | Python |
| `phpunit.xml`, `phpunit.dist.xml` | PHPUnit | PHP |
| `.rspec` | RSpec | Ruby |
| `build.gradle` (test deps), `pom.xml` (`<scope>test</scope>`) | JUnit/TestNG | Java/Kotlin |

Config files themselves are classified as `role: tooling` (not test, not production).

### 2.7 Generated and Vendor Code Detection

**Generated code patterns:**

| Pattern | Language | What It Catches |
|---|---|---|
| `*.generated.*`, `*.gen.*` | All | Explicit generated markers |
| `*.d.ts` | TypeScript | Type declaration files (not authored code) |
| `*.pb.ts`, `*.pb.go`, `*.pb.rs` | All | Protobuf generated code |
| `*_generated.go` | Go | go generate output |
| Files containing `// Code generated .* DO NOT EDIT` header | Go | Go's standard generated file header |
| Files containing `@generated` or `@auto-generated` in first 5 lines | All | Common generated file markers |
| `*.graphql.ts`, `*.gql.ts` | TypeScript | GraphQL codegen output |
| `__generated__/**` | All | Generated directory convention |
| `*.swagger.*`, `*.openapi.*` | All | API spec generated clients |

**Vendor code patterns:**

| Pattern | Language |
|---|---|
| `vendor/**` | Go, PHP, Ruby |
| `third_party/**`, `third-party/**` | All |
| `external/**`, `extern/**` | C/C++, Rust |
| `node_modules/**` | TS/JS (already excluded by most parsers) |

### 2.8 Exclusion Across All Metrics

When a node is classified as test, vendor, or generated, it must be excluded from production scoring. The exclusion applies to:

| Metric | Exclusion Rule |
|---|---|
| **Fan-in/fan-out** | Test → production edges are excluded from production fan-in counts. Production → test edges (if any) are excluded from production fan-out. |
| **Hotspot detection** | Hotspot threshold computed on production functions only. |
| **Cycle detection** | Cycles involving only test nodes are excluded from findings. Mixed cycles (test + production) are reported but noted. |
| **Module coupling** | Test-only modules excluded from coupling average (already in ecosystem config). Mixed modules (test + production functions) have coupling computed on production edges only. |
| **Dead code** | Test functions are exempt (already in spec Section 4.4, Layer 1 heuristics). |
| **Blast radius** | Computed on production graph only. Test callers don't count as dependents. |
| **Depth** | Computed on production call graph only. |
| **Health score** | All component scores derived from production-only metrics. |
| **Summary stats** | Report production and test counts separately: `total_functions` (production), `total_test_functions` (test). |
| **IFC** | Computed on production fan-in/fan-out only. |
| **Hub score** | Computed on production modules only. |

### 2.9 Separate Test Health (Optional)

When `--include-test-health` flag is provided, produce an additional `test_health` block in the JSON output:

```json
{
  "test_health": {
    "total_test_files": 48,
    "total_test_functions": 312,
    "test_to_production_coverage": 0.73,
    "orphan_test_count": 5,
    "test_complexity_avg": 3.2,
    "test_duplication_risk": "low"
  }
}
```

- `test_to_production_coverage`: fraction of production functions that have at least one incoming edge from a test function (rough test coverage proxy from the graph, without lcov)
- `orphan_test_count`: test functions that don't call any production function (potentially dead tests)

This is a future feature. Design for it now, implement when test health becomes a selling point.

### 2.10 User-Configurable Overrides

Extend the `--config` TOML (spec Section 5.0.3):

```toml
[test_detection]
# Additional test framework module names to add to the registry (Tier 1)
extra_framework_modules = ["my-internal-test-lib", "company-test-utils"]

# Force specific paths to production even if structural analysis says test
# Use case: shared types that live in test directories but are part of the API
force_production_paths = ["test/helpers/shared-types.ts", "src/testing/**"]

# Force specific paths to test even when structural analysis can't determine
# Use case: test files with no imports that live outside conventional test dirs
force_test_paths = ["qa/manual-checks/"]

# Additional Tier 4 name patterns (corroboration only, never primary)
extra_test_file_patterns = ["*_check.ts", "*Validation.java"]
```

- `extra_framework_modules`: extends the Tier 1 registry with project-specific test library names
- `force_production_paths`: overrides ALL tiers — paths here are always production (highest precedence)
- `force_test_paths`: forces test classification for paths structural analysis can't reach
- `extra_test_file_patterns`: additional Tier 4 corroboration patterns (still require structural corroboration)

---

## 3. Cyclomatic and Cognitive Complexity

### 3.1 Cyclomatic Complexity (McCabe)

**Definition:** The number of linearly independent paths through a function's control flow graph. Equals: `1 + number of decision points`.

**Decision points counted per language:**

| Construct | Languages | Increment |
|---|---|---|
| `if` / `else if` / `elif` | All | +1 per branch |
| `else` | All | Not counted (implicit path) |
| `for` / `for...in` / `for...of` / `for_each` | All | +1 |
| `while` / `do...while` | All | +1 |
| `match` arm / `case` / `switch case` | All | +1 per case |
| `&&` / `\|\|` (short-circuit logical operators) | All | +1 each |
| `catch` / `except` / `rescue` | All | +1 |
| `?.` (optional chaining) | TS/JS, Kotlin, Swift, C# | +1 |
| `??` (nullish coalescing) | TS/JS, C# | +1 |
| `? :` (ternary) | All that have it | +1 |
| `guard` | Swift | +1 |
| `if let` / `while let` | Rust, Swift | +1 |

**Implementation:** Walk the tree-sitter AST for each function node. Count occurrences of the decision point node types. This runs during parsing, not during analysis — the complexity value is stored as a node property in the SQLite database.

**New `properties` key on Function nodes:**
```json
{
  "cyclomatic_complexity": 12,
  "cognitive_complexity": 18,
  "loc": 45
}
```

### 3.2 Cognitive Complexity (Sonar)

**Definition:** A measure of how difficult a function is to understand. Differs from cyclomatic by:
1. **Nesting penalty:** Each level of nesting adds +1 to the increment for contained decision points
2. **No penalty for simple sequences:** `else` without `if` (simple continuation) is not penalized
3. **Bonus for breaks in linear flow:** `break`, `continue`, `goto`, `throw`, `return` (when not the last statement)

**Algorithm:**

```
cognitive_complexity = 0
nesting_level = 0

for each node in AST:
    if node is structural (if/for/while/switch/try):
        cognitive_complexity += 1 + nesting_level
        nesting_level += 1
        process children
        nesting_level -= 1
    if node is hybrid (else if):
        cognitive_complexity += 1  (no nesting penalty — it's a continuation)
    if node is logical operator (&&, ||):
        cognitive_complexity += 1  (only if different from previous operator in chain)
    if node is flow-break (break/continue/throw/return in middle of function):
        cognitive_complexity += 1
```

**Why both metrics?** Cyclomatic measures testability (how many paths to test). Cognitive measures readability (how hard to understand). A function can have low cyclomatic but high cognitive (deeply nested single-path logic), or high cyclomatic but low cognitive (flat switch statement with 20 cases — many paths but easy to understand).

### 3.3 Finding Thresholds

| Condition | Severity | Rationale |
|---|---|---|
| Cyclomatic > 25 OR cognitive > 30 | `critical` | Function is dangerously complex |
| Cyclomatic > 15 OR cognitive > 20 | `high` | Function should be refactored |
| Cyclomatic > 10 OR cognitive > 12 | `warning` | Function is getting complex |

These are industry-standard thresholds (SonarQube uses cognitive > 15 as default threshold). Configurable via `--config` TOML:

```toml
[thresholds]
cyclomatic_critical = 25
cyclomatic_high = 15
cyclomatic_warning = 10
cognitive_critical = 30
cognitive_high = 20
cognitive_warning = 12
```

### 3.4 New Finding Type

```json
{
  "id": "complexity-1",
  "type": "excessive_complexity",
  "severity": "high",
  "description": "handlePayment: cyclomatic complexity 22, cognitive complexity 28 (45 lines)",
  "node_ids": ["handle-payment"],
  "cyclomatic_complexity": 22,
  "cognitive_complexity": 28,
  "loc": 45,
  "recommendation": "Extract conditional branches into helper functions. Consider a strategy pattern for the payment type switch."
}
```

---

## 4. Temporal Coupling

### 4.1 Definition

Two files are temporally coupled if they frequently change together in the same commits. This reveals coupling that the import graph doesn't show — files that are logically connected but not connected by import/call edges.

### 4.2 Algorithm

**Input:** `--git-dir <path-to-.git>` CLI argument (spec Section 2.1, already designed).

**Steps:**

1. Run `git log --name-only --pretty=format:"%H" --since="6 months"` to get commits and their changed files
2. For each commit, collect the set of changed files
3. For each pair of files (A, B) that changed in the same commit, increment their co-change count
4. Compute temporal coupling score: `co_changes(A, B) / max(total_changes(A), total_changes(B))`
5. Filter: only report pairs with co-change count >= 3 AND coupling score >= 0.5

**Output per pair:**

```json
{
  "file_a": "src/auth/login.ts",
  "file_b": "src/database/users.ts",
  "co_change_count": 12,
  "coupling_score": 0.75,
  "has_import_edge": false
}
```

The `has_import_edge` flag is critical — temporal coupling that exists WITHOUT an import edge reveals hidden dependencies (shared database state, shared configuration, implicit contracts).

### 4.3 Finding Type

```json
{
  "id": "temporal-1",
  "type": "temporal_coupling",
  "severity": "warning",
  "description": "src/auth/login.ts and src/database/users.ts change together in 75% of commits (12 co-changes) with no import relationship",
  "file_a": "src/auth/login.ts",
  "file_b": "src/database/users.ts",
  "co_change_count": 12,
  "coupling_score": 0.75,
  "has_import_edge": false,
  "recommendation": "These files change together but have no explicit dependency. Investigate shared state, configuration, or implicit contracts. Consider making the dependency explicit or extracting a shared module."
}
```

**Severity:**

| Condition | Severity |
|---|---|
| coupling_score > 0.8 AND no import edge AND co_changes >= 5 | `high` |
| coupling_score > 0.5 AND no import edge AND co_changes >= 3 | `warning` |
| coupling_score > 0.5 AND has import edge AND co_changes >= 5 | `info` (expected — they're connected) |

### 4.4 Module-Level Temporal Coupling

Aggregate file-level temporal coupling to the module level (using the same path-prefix analysis modules from spec Section 3.3). If modules A and B have high temporal coupling but low import coupling, the module boundary may be wrong.

---

## 5. Module Cohesion (LCOM)

### 5.1 Definition

Lack of Cohesion of Methods — measures whether a module's functions are related to each other. A cohesive module has functions that call each other and share dependencies. An incohesive module is a grab-bag of unrelated functions that happen to share a directory.

### 5.2 Algorithm (LCOM4 Variant)

For each analysis module:

1. Build a subgraph of only the functions within the module
2. Add an undirected edge between two functions if:
   - One calls the other (Call edge in either direction)
   - Both call the same external function (shared dependency)
   - Both are called by the same external function (shared consumer)
3. Count the number of connected components in this undirected subgraph
4. `cohesion_score = 1.0 / connected_components` (1.0 = perfectly cohesive, all functions connected; 0.1 = 10 disconnected groups)

### 5.3 Interpretation

| Connected Components | Cohesion Score | Meaning |
|---|---|---|
| 1 | 1.0 | Fully cohesive — all functions are related |
| 2 | 0.5 | Module could be split into 2 |
| 3-4 | 0.25-0.33 | Module is a grab-bag, should be refactored |
| 5+ | < 0.2 | Module has no coherent purpose |

### 5.4 Finding Type

```json
{
  "id": "cohesion-1",
  "type": "low_cohesion",
  "severity": "warning",
  "description": "src/utils: 5 disconnected function groups (cohesion 0.20). Functions within this module don't reference each other.",
  "node_ids": ["utils-module"],
  "cohesion_score": 0.20,
  "connected_components": 5,
  "recommendation": "This module contains unrelated functions. Consider splitting into focused modules based on the 5 function groups."
}
```

### 5.5 Module Annotation Extension

Add to `module_annotations` (spec Section 6.4):

```json
{
  "cohesion_score": 0.33,
  "connected_components": 3
}
```

---

## 6. Abstractness (Martin)

### 6.1 Definition

The ratio of abstract types (interfaces, traits, abstract classes) to total types in a module. Measures how much a module defines contracts vs implementations.

### 6.2 Formula

```
Abstractness = abstract_types / total_types
```

Where:
- `abstract_types` = count of nodes with kind `Interface` + nodes with kind `Type` that represent abstract classes (where detectable)
- `total_types` = count of all nodes with kind `Interface`, `Struct`, `Enum`, `Type`, `Function` within the module

**Scale:** 0.0 = fully concrete (all implementations). 1.0 = fully abstract (all interfaces/traits).

### 6.3 Language-Specific Detection

| Language | Abstract Types | Concrete Types |
|---|---|---|
| TypeScript | `interface`, `type` (when used as contract), `abstract class` | `class`, `function`, `const` |
| Rust | `trait` | `struct`, `enum`, `fn` |
| Java/Kotlin | `interface`, `abstract class` | `class`, `enum`, `record` |
| Go | `interface` | `struct`, `func` |
| Python | `ABC` subclass, `Protocol` | `class`, `def` |
| C# | `interface`, `abstract class` | `class`, `struct`, `record` |

### 6.4 Module Annotation Extension

Add to `module_annotations`:

```json
{
  "abstractness": 0.35,
  "abstract_types": 7,
  "total_types": 20
}
```

---

## 7. Distance from Main Sequence (Martin)

### 7.1 Definition

Measures how far a module is from the ideal balance of abstractness and stability.

### 7.2 Formula

```
D = |Abstractness + Instability - 1|
```

Where `Instability` is already computed (spec Section 4.7.1) and `Abstractness` is from Section 6 above.

**Scale:** 0.0 = on the Main Sequence (ideal). 1.0 = maximally far from ideal.

### 7.3 Zones

| Zone | Condition | Meaning |
|---|---|---|
| **Zone of Pain** | Low Abstractness + Low Instability | Concrete AND stable. Hard to change because everything depends on it, but it has no abstractions to extend. Rigid. |
| **Zone of Uselessness** | High Abstractness + High Instability | Abstract AND unstable. Nobody depends on these abstractions and they depend on everything. Dead weight. |
| **Main Sequence** | D ≈ 0 | Balanced. Abstract modules are stable (good). Concrete modules are unstable (also good — they're implementation details that can change). |

### 7.4 Finding Type

```json
{
  "id": "distance-1",
  "type": "zone_of_pain",
  "severity": "warning",
  "description": "src/database: Distance from Main Sequence 0.72 (Abstractness 0.05, Instability 0.23). Concrete and stable — hard to extend without breaking dependents.",
  "node_ids": ["database-module"],
  "distance": 0.72,
  "abstractness": 0.05,
  "instability": 0.23,
  "recommendation": "This module is concrete and heavily depended upon. Extract interfaces for key functions so consumers depend on abstractions, not implementations."
}
```

| Condition | Severity |
|---|---|
| D > 0.7 AND in Zone of Pain | `high` |
| D > 0.5 | `warning` |
| D > 0.3 | `info` |

---

## 8. God Function Detection

### 8.1 Definition

A composite finding that flags functions exhibiting multiple complexity signals simultaneously. A function is a "god function" when it has high complexity AND high connectivity AND high size.

### 8.2 Detection Criteria

A function is flagged when it meets ALL THREE:

1. **High complexity:** cyclomatic >= 10 OR cognitive >= 15
2. **High connectivity:** fan_out >= 8 (calls many other functions)
3. **High size:** LOC >= 40

Meeting only 1-2 criteria doesn't qualify — it's the combination that indicates a function doing too much.

### 8.3 Finding Type

```json
{
  "id": "god-1",
  "type": "god_function",
  "severity": "high",
  "description": "processOrder: 67 lines, cyclomatic complexity 18, calls 12 functions across 5 modules. This function handles validation, pricing, inventory, payment, and notification.",
  "node_ids": ["process-order"],
  "loc": 67,
  "cyclomatic_complexity": 18,
  "cognitive_complexity": 24,
  "fan_out": 12,
  "distinct_module_calls": 5,
  "recommendation": "Break into single-responsibility functions: validateOrder(), calculatePrice(), reserveInventory(), processPayment(), sendNotification(). Each should handle one concern."
}
```

### 8.4 Dependencies

Requires Sections 3 (complexity) and function LOC. Cannot be computed without complexity metrics.

---

## 9. Automatic Layer Detection

### 9.1 Definition

Infer architectural layers from the directed graph structure without user-defined layer configuration. Functions near entry points are "higher level" (orchestration); functions only reachable through many hops are "lower level" (utilities, data access).

### 9.2 Algorithm

1. **Identify entry points:** Functions with fan_in == 0 for Call edges (same as depth computation in spec Section 4.6)
2. **Assign layer depth:** BFS from entry points. Each function's layer = minimum BFS distance from any entry point
3. **Group into layers:** Layer 0 = entry points (API handlers, CLI commands). Layer 1 = direct orchestrators. Layer 2+ = progressively lower-level implementation.
4. **Detect layer violations:** A layer violation occurs when a function at layer N directly calls a function at layer N+K where K > 2 (skipping intermediate layers). Example: API handler (layer 0) directly calling a database query helper (layer 4) — skipping the service and repository layers.

### 9.3 Finding Type

```json
{
  "id": "layer-1",
  "type": "layer_violation",
  "severity": "warning",
  "description": "handleRequest (layer 0, API) directly calls rawSqlQuery (layer 4, data). Skips service and repository layers.",
  "node_ids": ["handle-request", "raw-sql-query"],
  "caller_layer": 0,
  "callee_layer": 4,
  "layers_skipped": 3,
  "recommendation": "Route through intermediate layers: handleRequest → orderService → orderRepository → rawSqlQuery. This preserves separation of concerns and makes each layer independently testable."
}
```

### 9.4 Limitations

- Requires the graph to have clear entry points. Libraries (where every public function is an entry point) produce flat layer structures with few violations.
- Layer inference is approximate — it reflects call depth, not necessarily architectural intent. The `--config` TOML can override with explicit layer definitions:

```toml
[layers]
# Optional explicit layer definitions. When provided, overrides automatic detection.
layer_0 = ["src/api/**", "src/routes/**"]
layer_1 = ["src/services/**"]
layer_2 = ["src/repositories/**"]
layer_3 = ["src/database/**", "src/external/**"]
```

### 9.5 Module Annotation Extension

Add to `node_annotations` (spec Section 6.3):

```json
{
  "inferred_layer": 2,
  "layer_label": "service"
}
```

Add to `module_annotations` (spec Section 6.4):

```json
{
  "avg_layer_depth": 1.8,
  "layer_violation_count": 3
}
```

---

## 10. Function/File LOC

### 10.1 Definition

Lines of code per function and per file, derived from the existing `location` JSON on each node (spec Section 3.1).

### 10.2 Formula

```
loc = end_line - start_line + 1
```

The `location` JSON already contains `start_line` and `end_line` for every node. This is trivial to compute.

### 10.3 Usage

- **Per-function LOC** feeds into: god function detection (Section 8), complexity density (complexity/LOC), report enrichment
- **Per-module LOC** (sum of function LOCs) feeds into: module size comparison, "lines per module" in module annotations
- **Project total LOC** feeds into: summary stats, population bucketing (the percentile system uses LOC brackets)

### 10.4 Node Annotation Extension

Add to `node_annotations`:

```json
{
  "loc": 45
}
```

Add to `module_annotations`:

```json
{
  "total_loc": 1240
}
```

---

## 11. Health Score Reweighting

### 11.1 Extended Component Weights

With the new metrics, the health score formula gains additional components. The original 5 components remain, and new ones are added with weight redistributed:

| Component | Original Weight | New Weight | Metric Used |
|---|---|---|---|
| Cycle severity | 25% | 20% | `cycle_ratio` |
| Coupling health | 25% | 18% | `avg_coupling` |
| Hotspot concentration | 20% | 15% | `hotspot_concentration` |
| Dead code ratio | 15% | 10% | `dead_ratio` |
| Depth complexity | 15% | 10% | `max_depth` |
| **Complexity** (new) | — | 10% | `avg_cyclomatic` or `avg_cognitive` |
| **Cohesion** (new) | — | 7% | `avg_cohesion` |
| **Distance from Main Sequence** (new) | — | 5% | `avg_distance` |
| **Temporal coupling** (new) | — | 5% | `temporal_coupling_score` |

Weights must sum to 1.0. Configurable via `score_weights` in the `--config` TOML.

When temporal coupling is unavailable (no `--git-dir`), its weight is redistributed proportionally to the other components.

### 11.2 New Metrics Block Entries

Add to the `metrics` block (spec Section 6.1):

```json
{
  "complexity": {
    "avg_cyclomatic": 6.3,
    "avg_cognitive": 8.1,
    "max_cyclomatic": 34,
    "max_cognitive": 52,
    "functions_above_threshold": 12,
    "description": "Average cyclomatic complexity 6.3 (12 functions above warning threshold)"
  },
  "cohesion": {
    "avg_cohesion": 0.62,
    "low_cohesion_modules": 3,
    "description": "3 of 23 modules have cohesion below 0.33 (multiple disconnected function groups)"
  },
  "distance_from_main_sequence": {
    "avg_distance": 0.38,
    "zone_of_pain_modules": 2,
    "zone_of_uselessness_modules": 1,
    "description": "2 modules in zone of pain (concrete + stable), 1 in zone of uselessness (abstract + unstable)"
  },
  "temporal_coupling": {
    "high_coupling_pairs": 5,
    "hidden_coupling_pairs": 3,
    "description": "5 file pairs with temporal coupling > 0.5 (3 with no import relationship)"
  }
}
```

---

## 12. Implementation Priority

| Priority | Task | Effort | Dependencies |
|---|---|---|---|
| 1 | Function/file LOC (#10) | 1 hour | None — uses existing data |
| 2 | Cyclomatic + cognitive complexity (#3) | 3-4 hours | None — tree-sitter AST traversal in parser |
| 3 | Structural test detection system (#2.3) — framework import registry, Tier 1-2 graph traversal, Tier 3 AST, Tier 4 fallback | 2 days | Requires import edges in the graph |
| 4 | Test/generated exclusion across all metrics (#2.8) | Half day | Depends on #3 |
| 5 | God function detection (#8) | 2-3 hours | Depends on #2 (complexity) and #1 (LOC) |
| 6 | Module cohesion LCOM (#5) | 1 day | None |
| 7 | Abstractness (#6) | Half day | None |
| 8 | Distance from Main Sequence (#7) | 1 hour | Depends on #7 |
| 9 | Temporal coupling (#4) | 1 day | None (git access) |
| 10 | Automatic layer detection (#9) | 1 day | None |
| 11 | Wire --config TOML (including test_detection overrides) | 2-3 hours | None |
| 12 | Health score reweighting (#11) | 1-2 hours | Depends on all above |
| 13 | --exclude-tests / --exclude-generated flags | 2-3 hours | Depends on #4 |

**Total: ~7-8 days.**

---

## 13. New Finding Types Summary

Add to spec Section 6.2 finding type enum:

| Type | Description | Generated By |
|---|---|---|
| `excessive_complexity` | Function with high cyclomatic or cognitive complexity | Complexity (Section 3) |
| `temporal_coupling` | Files that change together without import relationship | Temporal coupling (Section 4) |
| `low_cohesion` | Module with multiple disconnected function groups | Cohesion (Section 5) |
| `zone_of_pain` | Module that is concrete AND stable (rigid) | Distance from Main Sequence (Section 7) |
| `zone_of_uselessness` | Module that is abstract AND unstable (wasted) | Distance from Main Sequence (Section 7) |
| `god_function` | Function with high complexity + high connectivity + high size | God detection (Section 8) |
| `layer_violation` | Function calling across too many inferred layers | Layer detection (Section 9) |

---

## 14. Relationship to Other Documents

| Document | Relationship |
|---|---|
| `GE_ANALYZE_FULL_SPECIFICATION.md` | This document extends Sections 4, 5, and 6 of the original spec. All original algorithms remain unchanged. |
| `ROADMAP.md` | This document is the spec for ROADMAP tasks #1-12 (Tiers 1-3). |
| `~/Desktop/gridseak-web/PLAN.md` | These metrics feed into the web dashboard, score cards, and paid reports. |
| `~/Desktop/GridCash/strategy/MASTER_ROADMAP.md` | These features run before Stage 1 web build to maximize product differentiation. |
