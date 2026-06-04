# Structural Health: Complete Measurement Vision

**Date:** 2026-02-25  
**Status:** Vision specification — extends `GE_ANALYZE_FULL_SPECIFICATION.md`  
**Relationship:** The existing spec (Sections 4.1–4.7) defines the *foundation* metrics. This document defines everything else: the metrics that transform `ge-analyze` from a good static analyzer into a system that sees what architects see.

---

## 1. What We Have vs What's Missing

### 1.1 Already Specified (GE_ANALYZE_FULL_SPECIFICATION.md)

| Metric | Section | What It Captures |
|--------|---------|-----------------|
| Cycle detection (Tarjan SCC) | 4.1 | Circular dependencies |
| Fan-in / Fan-out | 4.2 | Node-level dependency concentration |
| Module coupling ratio | 4.3 | Boundary leakage |
| Dead code | 4.4 | Unreachable functions |
| Blast radius | 4.5 | Downstream impact per node |
| Depth complexity | 4.6 | Call chain length |
| Instability (Martin) | 4.7.1 | Module volatility tendency |
| Tangle index | 4.7.2 | Fraction of graph in cycles |
| IFC (Henry-Kafura) | 4.7.3 | Information flow bottlenecks |
| API surface ratio | 4.7.4 | Module encapsulation |
| Hub score | 4.7.5 | Cross-module bridge nodes |

These are strong Layer 1 metrics. They measure the graph correctly. But they answer *one class* of question: "how connected is everything?" They do not answer:

- **How far does a random change propagate?** (system-level resilience)
- **Are modules playing the right structural role?** (role correctness)
- **Does the dependency flow obey its own rules?** (constraint adherence)
- **What kind of coupling is this?** (coupling quality, not just quantity)
- **Is the system getting worse over time?** (trajectory)
- **Is the design internally consistent?** (conceptual integrity)

### 1.2 The Gaps — Organized by What They Reveal

| Gap | What It Reveals | Tier |
|-----|----------------|------|
| Propagation cost | System-level change resilience | 1 (High-ROI) |
| Abstractness + Distance from Main Sequence | Module role correctness | 1 (High-ROI) |
| Boundary fitness functions | Declared intent vs reality | 1 (High-ROI) |
| Stability gradient violations | Dependency flow direction correctness | 1 (High-ROI) |
| Connascence-weighted coupling | Coupling danger level, not just amount | 1 (High-ROI) |
| Function complexity profile | Code-level maintainability | 2 (Decomposed) |
| Information hiding ratio | Encapsulation depth (beyond API surface) | 2 (Decomposed) |
| Consumer completeness | Interface segregation violations | 2 (Decomposed) |
| Feature envy | Misplaced responsibilities | 2 (Decomposed) |
| Law of Demeter violations | Coupling chain depth | 2 (Decomposed) |
| LCOM4 (cohesion) | Class/struct split candidates | 2 (Decomposed) |
| Conceptual integrity | Design consistency across modules | 3 (System-Level) |
| Hub-authority (HITS) | Structural backbone identification | 3 (System-Level) |
| Layering violations | Upward dependency direction errors | 3 (System-Level) |
| Architectural erosion rate | Structural health trajectory | 3 (System-Level) |
| Complexity distribution shape | Risk tail detection | 3 (System-Level) |

---

## 2. The Grand Measurement Hierarchy

Every metric in the system occupies a position in a hierarchy. The hierarchy has three properties:

1. **Higher tiers explain lower tiers.** Propagation cost (Tier 1) is the *consequence* of coupling scores (existing), stability gradient violations (Tier 1), and boundary failures (Tier 1). If propagation cost is high, drill into the Tier 1 components to find *why*.

2. **Each metric has an evidence chain.** No metric exists in isolation. Every metric points to specific nodes, edges, or modules in the graph. Every recommendation generated from a metric links to the concrete graph elements that produced it.

3. **Metrics compose into a dashboard, not a single number.** The composite health score (percentile) remains as a summary. But the dashboard presents metrics in *functional groups* that map to questions architects actually ask.

### Dashboard Functional Groups

| Group | Question It Answers | Metrics |
|-------|-------------------|---------|
| **System Resilience** | "How fragile is the whole system?" | Propagation cost, tangle index, max depth |
| **Module Health** | "Are modules well-designed?" | Coupling, instability, abstractness, distance, API surface, LCOM4 |
| **Dependency Discipline** | "Does the dependency graph follow its own rules?" | Stability gradient violations, boundary fitness, layering violations, cycle count |
| **Hotspot Risk** | "Where are the dangerous nodes?" | Blast radius, fan-in, IFC, hub score, connascence profile |
| **Code Vitality** | "Is there waste or decay?" | Dead code ratio, dead code trend, feature envy, complexity distribution |
| **Trajectory** | "Is the architecture getting better or worse?" | Erosion rate, propagation cost trend, boundary violation trend |

---

## 3. Tier 1: The 5 High-ROI Additions

These are the highest-leverage metrics not yet in the specification. Each is fully deterministic, computable from the existing graph data (or with minimal extensions), and has a strong evidence base in peer-reviewed research.

### 3.1 Propagation Cost

**Source:** MacCormack, Rusnak, Baldwin — "Exploring the Duality between Product and Organizational Architectures" (Harvard Business School, 2008-2012). Validated against hundreds of open-source and commercial systems.

**What it measures:** The fraction of the system that could be affected by a change to a random node. One scalar that captures the system's global resilience to change.

**Why it's the single most important addition:** Every other metric is local (per-node, per-module). Propagation cost is the only metric that answers the *system-level* question: "how much of this codebase do I need to understand to change any part of it safely?" It integrates all coupling, all dependency paths, all structural decisions into one number.

#### 3.1.1 Sub-Components

**The Dependency Structure Matrix (DSM):**

Given \( n \) nodes (functions, structs, modules — configurable granularity), construct the binary adjacency matrix \( A \) where \( A_{ij} = 1 \) if node \( i \) has a non-containment dependency on node \( j \).

**The Visibility Matrix:**

\[
V = I + A + A^2 + A^3 + \ldots + A^k
\]

where \( k \) = graph diameter (longest shortest path). Each entry \( V_{ij} \) represents whether node \( i \) can reach node \( j \) through any chain of dependencies. For practical computation, use the boolean transitive closure (Warshall's algorithm, \( O(n^3) \)) or iterative squaring.

For large graphs, use the **damped version** to weight indirect paths less:

\[
V = (I - \epsilon A)^{-1}
\]

where \( \epsilon \in (0, 1/\lambda_{\max}) \) and \( \lambda_{\max} \) is the largest eigenvalue of \( A \). This converges and avoids issues with cyclic graphs.

**Propagation Cost:**

\[
\text{PC} = \frac{|\{(i,j) : V_{ij} > 0, i \neq j\}|}{n(n-1)}
\]

The fraction of off-diagonal entries in \( V \) that are non-zero.

#### 3.1.2 Interpretation Scale

| PC Value | Meaning | Evidence |
|----------|---------|----------|
| 0.00–0.10 | Highly modular. Changes are well-contained. | Typical of well-designed microservice architectures, Rust crates with clean boundaries. |
| 0.10–0.25 | Moderate propagation. Most changes are local. | Typical of mature open-source libraries (lodash, express). |
| 0.25–0.40 | Significant propagation risk. Refactors frequently touch distant code. | Common in growing monoliths, enterprise codebases. |
| 0.40–0.60 | High propagation. Architecture is a bottleneck. | Correlates with high defect density (MacCormack et al. r > 0.6). |
| 0.60–1.00 | Pathological. Effectively a single entangled unit. | System has no meaningful modularity despite file/folder separation. |

#### 3.1.3 Computation Strategy for ge-analyze

At function-level granularity, the \( O(n^3) \) cost of transitive closure is prohibitive for 10,000-node graphs. Two options:

**Option A — Module-level propagation cost:** Compute on analysis modules (typically 20-50 nodes). \( O(50^3) \) is trivial. This gives the system-level answer. Per-node detail comes from the existing blast radius metric.

**Option B — Sampled propagation cost:** Randomly sample 500 nodes, compute reachability from each via BFS (\( O(500 \times (V + E)) \)), estimate the fraction of non-zero entries. Converges to true PC within ±0.02 at 500 samples for graphs up to 50,000 nodes.

**Recommendation:** Option A for the headline metric. Option B as validation. Both are well within the 60-second budget for 10,000-node graphs.

#### 3.1.4 Output

```json
"propagation_cost": {
  "value": 0.23,
  "granularity": "module",
  "modules_analyzed": 29,
  "description": "23% of module pairs have a dependency path between them — a change to any module could propagate to roughly 1 in 4 others"
}
```

Added to the `metrics` block. Added to the `percentiles` block when norms available.

#### 3.1.5 Relationship to Existing Metrics

- **Propagation cost is the *consequence*** of coupling, cycles, and hub concentration.
- If PC is high, drill into: module coupling scores (which modules leak the most?), cycle membership (which cycles create unbounded propagation?), hub nodes (which nodes act as propagation bridges?).
- PC provides the *motivation* for fixing what the other metrics detect.

---

### 3.2 Abstractness and Distance from the Main Sequence

**Source:** Robert C. Martin — "Agile Software Development: Principles, Patterns, and Practices" (2002). The Stable Abstractions Principle (SAP).

**What it measures:** Whether each module plays the right structural role. Stable modules should be abstract (so they can be extended without modification). Unstable modules should be concrete (so they can be replaced easily). Modules that violate this relationship are in one of two danger zones.

#### 3.2.1 Sub-Components

**Abstractness per module:**

\[
A = \frac{N_a}{N_c}
\]

where:
- \( N_a \) = count of abstract types in the module (nodes with `kind` ∈ {`Interface`, `Type` with no body, abstract `Struct`})
- \( N_c \) = total type-level nodes in the module (nodes with `kind` ∈ {`Interface`, `Struct`, `Enum`, `Type`})

If the module has zero type-level nodes, abstractness is undefined (skip the module for distance computation, report abstractness as `null`).

**Detection of abstract types from the existing schema:**
- `Interface` kind → always abstract (interfaces/traits define contracts)
- `Type` kind → abstract if it has no `Contains` edges to `Function` nodes (a type alias or opaque type)
- `Struct` kind → concrete (even if it has trait implementations, the struct itself is concrete)
- `Enum` kind → concrete

This is deterministic from the graph. No runtime information needed.

**Instability** is already in the spec (Section 4.7.1):

\[
I = \frac{C_e}{C_a + C_e}
\]

**Distance from the Main Sequence:**

\[
D = |A + I - 1|
\]

The "Main Sequence" is the line \( A + I = 1 \). Modules on this line have the ideal balance: highly stable modules are highly abstract, highly unstable modules are highly concrete.

#### 3.2.2 The Two Danger Zones

**Zone of Pain** (\( I \approx 0 \), \( A \approx 0 \), \( D > 0.5 \)):
- The module is stable (everything depends on it) AND concrete (no abstractions).
- Consequence: it's nearly impossible to change without breaking dependents, and there are no abstractions to extend.
- Real examples: a shared `utils.ts` that 30 modules import, containing only concrete functions. An `auth` module that the entire system depends on, with zero interfaces.
- The recommendation is always: extract interfaces, invert dependencies, or split the module.

**Zone of Uselessness** (\( I \approx 1 \), \( A \approx 1 \), \( D > 0.5 \)):
- The module is unstable (depends on everything, nothing depends on it) AND abstract (mostly interfaces).
- Consequence: abstract types that nobody consumes. Over-engineering. Dead abstractions.
- Real examples: an `interfaces/` folder full of types that only one concrete class implements. A `contracts/` module that was designed speculatively and never used.
- The recommendation is: remove unused abstractions, or make the module useful by having concrete implementations depend on it.

#### 3.2.3 Output

Per-module addition to `module_annotations`:

```json
{
  "abstractness": 0.15,
  "abstract_types": 3,
  "total_types": 20,
  "distance_from_main_sequence": 0.50,
  "zone": "pain"
}
```

`zone` is one of: `"pain"` (\( A < 0.3 \) and \( I < 0.3 \) and \( D > 0.3 \)), `"uselessness"` (\( A > 0.7 \) and \( I > 0.7 \) and \( D > 0.3 \)), or `null` (not in a danger zone).

Finding output when \( D > 0.5 \):

```json
{
  "id": "distance-1",
  "type": "role_mismatch",
  "severity": "warning",
  "description": "auth/ module: stable (I=0.15) but concrete (A=0.10) — Zone of Pain. Distance 0.75. Dependents cannot extend without modifying this module.",
  "node_ids": ["module-auth"],
  "metric_name": "distance_from_main_sequence",
  "metric_value": 0.75,
  "recommendation": "Extract interfaces for the 3 most-depended-on types in this module. Current dependents can then depend on abstractions instead of implementations."
}
```

#### 3.2.4 Relationship to Existing Metrics

- **Instability** (already in spec) is one axis. Abstractness is the other. Neither is meaningful alone — it's the *combination* that reveals whether a module's role is correct.
- A module with high coupling (existing metric) AND in the Zone of Pain is the most dangerous structural pattern: hard to change, heavily depended upon, no abstractions to absorb change. The combination should elevate severity.
- Distance feeds into propagation cost explanation: modules in the Zone of Pain are propagation *amplifiers* because changes to them cannot be absorbed by abstractions.

---

### 3.3 Boundary Fitness Functions

**Source:** Neal Ford, Rebecca Parsons — "Building Evolutionary Architectures" (O'Reilly, 2017, 2nd ed 2023). The core idea: architectural properties must be *automated, measurable, continuously verified* — not documented and forgotten.

**What it measures:** The delta between the architecture the developer *intends* and the architecture that *exists*. This is the only metric in the system that requires user-declared input — but once declared, measurement is fully deterministic.

#### 3.3.1 Sub-Components

A boundary fitness function is a rule of the form:

```
<source_module_pattern> must not depend on <target_module_pattern>
```

or more generally:

```
<constraint_type> <subject> <predicate> <object>
```

**Constraint types (ordered by implementation priority):**

| Constraint | Syntax | Graph Operation |
|-----------|--------|----------------|
| **No-depend** | `src/auth must_not_depend_on src/ui` | Count edges from auth module to ui module. Violations = count. |
| **Only-through** | `src/handlers must_only_depend_on_through src/services` | For handler→any_other_module edges, verify an intermediate node in services module exists on the path. Violations = direct edges that bypass. |
| **Layer ordering** | `layers: [ui, handlers, services, data]` | For every edge, verify source layer index ≤ target layer index. Violations = upward edges. |
| **Acyclic modules** | `acyclic: [src/auth, src/payments, src/config]` | Run Tarjan's SCC on only the listed modules. Violations = cycles within the set. |
| **Max fan-in** | `src/utils max_fan_in 10` | Count incoming edges from other modules. Violation if exceeds threshold. |
| **Max coupling** | `src/core max_coupling 0.5` | Module coupling score. Violation if exceeds threshold. |

#### 3.3.2 Declaration Format

The rules live in the `--config` TOML (extending the schema in Section 5.0.3 of the main spec):

```toml
[[boundary_rules]]
type = "no_depend"
from = "src/ui"
to = "src/data"
description = "UI layer must not access data layer directly"

[[boundary_rules]]
type = "layer_order"
layers = ["ui", "handlers", "services", "data"]
module_prefix_map = {
  "ui" = "src/components",
  "handlers" = "src/routes",
  "services" = "src/services",
  "data" = "src/db"
}
description = "Layered architecture: UI → handlers → services → data"

[[boundary_rules]]
type = "max_coupling"
module = "src/core"
threshold = 0.5
description = "Core module must remain well-encapsulated"
```

#### 3.3.3 Output

Violations populate the existing `boundary_violations` array (already stubbed in Section 6.5 of the main spec):

```json
{
  "boundary_violations": [
    {
      "rule_id": "boundary-1",
      "rule_type": "no_depend",
      "rule_description": "UI layer must not access data layer directly",
      "violations": 3,
      "violating_edges": [
        {
          "from_fqn": "src/components/UserProfile::fetchUser",
          "to_fqn": "src/db/queries::getUserById",
          "edge_kind": "Call"
        }
      ],
      "severity": "high",
      "recommendation": "Route data access through the services layer. UserProfile should call a UserService method, which calls the database query."
    }
  ]
}
```

#### 3.3.4 Why This Isn't Subjective

The *declaration* of rules is a design decision (subjective in origin). But the *measurement* is fully deterministic: given the rules, the violation count is a mathematical fact about the graph. This is the same pattern as test assertions — the developer chooses what to test (subjective), the pass/fail is objective.

The power is that boundary rules turn *architectural intent into a CI-verifiable contract*. When the intent is declared, drift becomes detectable. Without the declaration, drift is invisible until something breaks in production.

#### 3.3.5 Auto-Inferred Boundaries (Future Enhancement)

For projects without declared rules, `ge-analyze` can *infer* likely boundaries:
1. Modules with zero cycles between them → candidate for `acyclic` constraint.
2. Module pairs where dependency is strictly one-directional → candidate for `no_depend` in the reverse direction.
3. Module clusters where coupling < 0.3 → candidate for `max_coupling` constraints at that threshold.

The inferred constraints are reported as *suggestions*, not findings. The developer promotes the ones that match intent.

#### 3.3.6 Relationship to Existing Metrics

- Boundary violations are a *focused* version of coupling analysis. Coupling score tells you "how much leakage." Boundary violations tell you "*which specific leakages matter* based on declared intent."
- They complement cycle detection. Cycles are universally bad. Boundary violations catch dependencies that aren't cycles but still violate the intended layering.
- In the dashboard, boundary violations appear in the "Dependency Discipline" group alongside cycles, stability gradient, and layering violations.

---

### 3.4 Stability Gradient Violations

**Source:** Robert C. Martin — Stable Dependencies Principle (SDP): "Depend in the direction of stability." A module should only depend on modules that are *more stable* than itself.

**What it measures:** Per-edge correctness of dependency direction. Every dependency in the graph either flows "downhill" (toward stability) or "uphill" (toward instability). Uphill dependencies are violations.

#### 3.4.1 Sub-Components

For every non-containment inter-module edge \( (M_a \rightarrow M_b) \):

\[
\text{violation} = \begin{cases} 1 & \text{if } I(M_a) < I(M_b) \\ 0 & \text{otherwise} \end{cases}
\]

where \( I(M) \) is the instability of module \( M \) (already computed per Section 4.7.1).

A stable module (\( I \approx 0 \)) depending on an unstable module (\( I \approx 1 \)) is a violation: the stable module has many dependents, and it now depends on something likely to change.

**Stability Gradient Score:**

\[
\text{SGS} = \frac{\text{violation edges}}{\text{total inter-module edges}}
\]

A system where 100% of dependencies flow downhill has SGS = 0. A system with random dependency direction has SGS ≈ 0.5.

#### 3.4.2 Severity

| SGS Value | Severity | Meaning |
|-----------|----------|---------|
| 0.00–0.10 | Healthy | Dependencies overwhelmingly flow toward stability. |
| 0.10–0.25 | Info | A few uphill dependencies. May be justified (e.g., adapter patterns). |
| 0.25–0.40 | Warning | Significant fraction of dependencies flow the wrong way. Module roles are unclear. |
| 0.40+ | High | The dependency graph has no consistent direction. Modules are not playing distinct stability roles. |

#### 3.4.3 Per-Edge Output

For each violating edge:

```json
{
  "id": "sdp-violation-1",
  "type": "stability_gradient_violation",
  "severity": "warning",
  "description": "src/core (I=0.12, stable) depends on src/handlers (I=0.85, unstable). A stable module should not depend on a volatile one.",
  "from_module": "src/core",
  "to_module": "src/handlers",
  "from_instability": 0.12,
  "to_instability": 0.85,
  "instability_delta": -0.73,
  "recommendation": "Invert the dependency: extract an interface in src/core that src/handlers implements, or move the shared functionality to a third module."
}
```

**Aggregate output added to `metrics` block:**

```json
"stability_gradient": {
  "violation_edges": 14,
  "total_inter_module_edges": 156,
  "ratio": 0.090,
  "description": "14 of 156 inter-module edges flow toward less stable modules (9.0%)"
}
```

#### 3.4.4 Computation

This is trivial once instability is computed (it already is). For each inter-module edge, compare \( I(\text{source module}) \) and \( I(\text{target module}) \). Cost: \( O(E) \) — a single pass over the edge list.

#### 3.4.5 Relationship to Existing Metrics

- **Instability** (existing) gives per-module values. Stability gradient uses those values to evaluate *edges*.
- **Propagation cost** (Tier 1 above) is high *because* of stability gradient violations. If stable modules depend on unstable ones, changes to unstable modules propagate into stable modules that have many dependents, creating cascading propagation.
- Stability gradient violations are one of the most actionable findings because the fix pattern is always the same: dependency inversion or interface extraction.

---

### 3.5 Connascence-Weighted Coupling

**Source:** Meilir Page-Jones — "What Every Programmer Should Know About Object-Oriented Design" (1995). Extended by Jim Weirich — "Grand Unified Theory of Software Design" (RubyConf 2009).

**What it measures:** Not how *much* coupling exists (the existing coupling score does that), but how *dangerous* the coupling is. Connascence is a taxonomy with a strict ordinal ranking — stronger forms of connascence are harder to detect, harder to fix, and more likely to cause defects.

#### 3.5.1 The Connascence Taxonomy

Listed from weakest (least dangerous) to strongest (most dangerous):

| Level | Name | Definition | Detectable from Graph? | Detection Method |
|-------|------|-----------|----------------------|-----------------|
| 1 | **Name** | Two components must agree on a symbol name | Yes | Default. Every `Call`, `Import`, `Uses` edge implies CoN. |
| 2 | **Type** | Two components must agree on a type | Yes | `Type` edges in the graph. Count of shared type dependencies between modules. |
| 3 | **Meaning** | Two components must agree on the meaning of a value (magic numbers, string constants, enum semantics) | Partially | Shared constant references across modules. Requires AST-level detection: look for literal values that appear in multiple modules. |
| 4 | **Position** | Two components must agree on the order of values (parameter ordering) | Yes | Functions called from multiple modules where parameter count > 3. Detectable from AST: callers must pass arguments in the correct positional order. |
| 5 | **Algorithm** | Two components must agree on an algorithm (serialization format, hashing, encoding) | Partially | Shared function pairs where both call the same algorithm primitives (e.g., both import SHA-256, both use JSON.parse). Pattern-match on common algorithm-indicator imports. |
| 6–9 | Execution, Timing, Value, Identity | Runtime connascence | No | Requires runtime analysis. Out of scope for static graph analysis. |

#### 3.5.2 Connascence Weight Formula

For each inter-module edge, assign a **connascence weight** based on the strongest detected connascence level:

| Connascence Level | Weight |
|-------------------|--------|
| Name | 1.0 |
| Type | 1.5 |
| Meaning | 2.5 |
| Position | 3.0 |
| Algorithm | 4.0 |

**Connascence-Weighted Coupling per module:**

\[
\text{CWC}(M) = \frac{\sum_{e \in \text{external\_edges}(M)} w(e)}{\sum_{e \in \text{all\_edges}(M)} w(e)}
\]

where \( w(e) \) is the connascence weight of edge \( e \).

This replaces or supplements the existing coupling score (which treats all edges equally) with a quality-weighted version. A module with 10 name-connascent edges is healthier than one with 3 algorithm-connascent edges, even though the first has higher raw coupling.

#### 3.5.3 Static Detection Methods

**Connascence of Type (level 2):**
Already captured. Count `Type` edges between modules. Enrichment: for each type-dependency edge, check if the shared type is an interface (weak CoT) or a concrete struct (strong CoT). Weight accordingly: interface types get 1.2, concrete types get 1.8.

**Connascence of Meaning (level 3):**
Requires an AST enrichment pass. For each module, extract literal constants (string literals, numeric literals, enum variants). Cross-reference: if the same literal appears in two modules that have an edge between them, flag as CoMeaning. Heuristic: only flag literals that appear 3+ times across 2+ modules (filters out common values like 0, 1, "", true).

**Connascence of Position (level 4):**
For each function node, count parameters (from AST or schema enrichment). If a function has 4+ parameters and is called from 2+ modules, the callers are position-connascent. More parameters = stronger positional coupling. Weight: `min(param_count / 3, 3.0)`.

**Connascence of Algorithm (level 5):**
Pattern-match on shared algorithmic imports. If two modules both import from the same hashing/encoding/serialization library AND have an edge between them, flag as CoAlgorithm. Detection list: `crypto`, `hash`, `serialize`, `encode`, `decode`, `compress`, `cipher`, `hmac`, `jwt`, `base64`.

#### 3.5.4 Phased Implementation

**Phase A (Tier 1 priority, no schema changes):**
- Weight existing edges by `kind`: `Call` = 1.0, `Import` = 1.0, `Uses` = 1.5, `Type` = 1.5.
- Compute CWC using these weights.
- This already produces a meaningfully different ranking than raw coupling. Modules with mostly `Type` dependencies are flagged as more dangerous than those with `Call` dependencies.

**Phase B (requires AST enrichment — separate ticket):**
- Add parameter count to function node properties during parsing.
- Detect positional connascence for functions with 4+ params called cross-module.
- Add constant literal extraction for meaning connascence.

**Phase C (future):**
- Import analysis for algorithm connascence.
- Connascence report as a distinct section in the health JSON.

#### 3.5.5 Output

```json
"connascence_profile": {
  "total_inter_module_edges": 156,
  "by_level": {
    "name": { "count": 98, "weight_sum": 98.0 },
    "type": { "count": 34, "weight_sum": 51.0 },
    "meaning": { "count": 12, "weight_sum": 30.0 },
    "position": { "count": 8, "weight_sum": 24.0 },
    "algorithm": { "count": 4, "weight_sum": 16.0 }
  },
  "weighted_coupling_avg": 0.48,
  "description": "24 of 156 inter-module edges have connascence above name-level — these carry higher refactoring risk"
}
```

#### 3.5.6 Relationship to Existing Metrics

- **Coupling score** (existing) measures *quantity* of boundary leakage. CWC measures *quality*. Two modules can have identical coupling scores but vastly different CWC if one's edges are all `Call` (weak) and the other's are `Type` + shared constants (strong).
- **IFC** (existing) measures information flow *volume*. Connascence measures information flow *kind*. They're complementary.
- In the dashboard, connascence profile appears in "Hotspot Risk" — it tells you not just *which* nodes are risky, but *why* the risk is hard to fix (stronger connascence = harder to decouple).

---

## 4. Tier 2: Decomposed Subjective Metrics Made Deterministic

These metrics address things developers call "code quality" or "maintainability" — concepts that *sound* subjective but decompose into measurable sub-components.

### 4.1 Function Complexity Profile

**What it decomposes:** "This code is hard to read/maintain."

"Hard to maintain" is not measurable. But the *causes* of maintenance difficulty are:

| Sub-Component | Formula | Source Data | Interpretation |
|--------------|---------|-------------|---------------|
| **Body size** | `end_line - start_line` per function | `location` JSON (already in schema) | Long functions are harder to reason about. Distribution shape matters more than mean. |
| **Parameter count** | Count of parameters per function | Requires AST enrichment (add `param_count` to function node properties during parsing) | High param count = positional coupling, high cognitive load, difficult testing. |
| **Nesting depth** | Max indentation level within function body | Requires AST enrichment (tree-sitter can compute max depth of the AST subtree) | Deep nesting = complex control flow, difficult to trace execution. |
| **Cyclomatic complexity** | Count of decision points (if/else/match/for/while) + 1 | Requires AST enrichment | High CC = many execution paths, exponential test space. |

**Why distribution shape matters more than mean:**

A codebase with mean function length 20 lines and max 25 is healthy. A codebase with mean function length 20 lines and a few functions at 500 lines is dangerous — the mean hides the fat tail. Report:

```json
"function_complexity_profile": {
  "body_size": {
    "p50": 12,
    "p75": 28,
    "p90": 67,
    "p95": 142,
    "p99": 380,
    "max": 847,
    "fat_tail_ratio": 0.08,
    "description": "8% of functions exceed 100 lines (fat tail detected)"
  },
  "param_count": {
    "p50": 2,
    "p75": 3,
    "p90": 5,
    "p95": 7,
    "max": 14,
    "functions_above_5": 23,
    "description": "23 functions have 6+ parameters"
  }
}
```

**`fat_tail_ratio`:** Fraction of functions exceeding 3× the median. If > 0.05 (more than 5% of functions are 3× longer than the median), this is a finding.

**Implementation dependency:** Body size is computable today (`location` JSON exists). Parameter count, nesting depth, and cyclomatic complexity require tree-sitter AST enrichment during parsing. This is a parser-level change, not an analysis-level change.

### 4.2 Information Hiding Ratio

**What it decomposes:** "This module has bad encapsulation."

The existing API surface ratio measures how many functions are externally called. Information hiding ratio goes deeper: of the module's *internal* implementation, how much is invisible to outsiders?

\[
\text{IHR}(M) = 1 - \frac{\text{externally reachable nodes}}{\text{total nodes in M}}
\]

Where "externally reachable" means: a node in \( M \) that is the target of any non-containment edge from outside \( M \), or is reachable from such a target via internal edges.

**Difference from API surface:** API surface counts only *direct* external targets. IHR counts *transitive* internal reachability from external entry points. A module can have a small API surface (2 exported functions) but low IHR if those 2 functions internally call everything else in the module — the module has a narrow interface but no real encapsulation.

IHR = 0.0 means every node is reachable from outside. IHR = 0.8 means 80% of the module's internals are truly hidden.

**Output per module (added to `module_annotations`):**

```json
{
  "information_hiding_ratio": 0.65,
  "externally_reachable_nodes": 7,
  "total_nodes": 20,
  "description": "65% of module internals are unreachable from external entry points"
}
```

**Computation:** For each module, find its external entry points (nodes targeted by cross-module edges). BFS from each entry point following *intra-module* edges. Count reachable nodes. IHR = 1 - (reachable / total). Cost: \( O(\sum_M (V_M + E_M)) \) — total cost is bounded by graph size.

### 4.3 Consumer Completeness (Interface Segregation)

**What it decomposes:** "This module's interface is too fat."

The Interface Segregation Principle (ISP) says: no consumer should be forced to depend on methods it doesn't use. Consumer completeness measures this directly.

For each module \( M \), let \( S \) = set of externally-called functions (the module's effective interface). For each consuming module \( C \) that depends on \( M \):

\[
\text{completeness}(C, M) = \frac{|S_C|}{|S|}
\]

where \( S_C \) = the subset of \( S \) that \( C \) actually uses.

**Module-level ISP violation score:**

\[
\text{ISP}(M) = 1 - \overline{\text{completeness}(C, M)}
\]

If every consumer uses every exported function, ISP = 0 (no violation). If each consumer uses only 1 of 20 exported functions on average, ISP = 0.95 (severe violation — the module should probably be split).

**Output per module:**

```json
{
  "isp_score": 0.72,
  "interface_size": 15,
  "consumers": 8,
  "avg_consumer_usage": 4.2,
  "description": "8 consumers use an average of 4.2 out of 15 exported functions (72% ISP violation). Consider splitting into smaller, focused modules."
}
```

**Computation:** For each module, find its exported functions (already computed for API surface). For each consumer module, count which of those functions it calls. Average the ratios. Cost: proportional to the number of cross-module edges.

### 4.4 Feature Envy

**What it decomposes:** "This function is in the wrong module."

A function exhibits feature envy when it references more symbols from another module than from its own. It's structurally misplaced — it should probably live in the module it envies.

For each function \( f \) in module \( M \):

\[
\text{envy}(f, M') = \frac{\text{edges from } f \text{ to } M'}{\text{edges from } f \text{ to any module}}
\]

If \( \text{envy}(f, M') > 0.5 \) for some \( M' \neq M \), function \( f \) envies module \( M' \). Stronger: if \( \text{envy}(f, M') > \text{edges to own module} \), the function accesses \( M' \) more than its own home.

**Finding output:**

```json
{
  "id": "envy-1",
  "type": "feature_envy",
  "severity": "info",
  "description": "formatPaymentReceipt in src/utils references 7 symbols in src/payments but only 2 in src/utils. Consider moving it to src/payments.",
  "node_ids": ["node-formatPaymentReceipt"],
  "envied_module": "src/payments",
  "own_module_references": 2,
  "envied_module_references": 7,
  "recommendation": "Move formatPaymentReceipt to src/payments where it has the strongest affinity."
}
```

**Computation:** For each function, partition its outgoing edges by target module. If max(external module) > own module count, flag. Cost: \( O(E) \).

### 4.5 Law of Demeter Violations

**What it decomposes:** "This code is too tightly coupled to the internal structure of other objects."

The Law of Demeter (LoD) states: a function should only call methods on (a) itself, (b) its parameters, (c) objects it creates, (d) its direct component objects. Violations manifest as call chains: `a.getB().getC().doSomething()`.

**Static detection:** For each function \( f \), look at its call targets. If \( f \) calls function \( g \), and \( g \) is NOT:
- In \( f \)'s own module, OR
- In a module that \( f \)'s module directly depends on

then \( f \) is reaching through a transitive dependency — an LoD violation.

More concretely: if a function calls into module \( M_3 \) but its own module has no direct dependency on \( M_3 \) (only through \( M_2 \)), the function is reaching through \( M_2 \) to get to \( M_3 \).

**LoD violation count per function:**

\[
\text{LoD}(f) = |\{g : f \rightarrow g \text{ and } M(g) \notin \text{direct\_deps}(M(f)) \cup \{M(f)\}\}|
\]

**Aggregate output:**

```json
"law_of_demeter": {
  "violation_functions": 34,
  "total_functions": 1314,
  "violation_ratio": 0.026,
  "description": "34 functions call into modules their own module has no direct dependency on — reaching through transitive dependencies"
}
```

### 4.6 LCOM4 (Lack of Cohesion in Methods)

**What it decomposes:** "This class does too many things."

LCOM4 (Hitz & Montazeri, 1995) is the number of connected components in the intra-class method graph, where methods are connected if they share a field access.

**Detection from graph:** For each `Struct` or `Interface` node \( S \):
1. Find all `Function` nodes contained by \( S \) (via `Contains` edges).
2. For each pair of functions, check if they share any `Uses` or `Call` edges to the same variable/field node within \( S \).
3. Build a graph of functions connected by shared access.
4. Count connected components.

LCOM4 = 1 means the class is cohesive (all methods are interconnected). LCOM4 > 1 means the class has \( n \) independent groups of methods that don't share state — it should be split into \( n \) classes.

**Limitation:** This requires `Variable` nodes and intra-struct `Uses` edges. The current parser may not emit these for all languages. If the data isn't available, LCOM4 is skipped (graceful degradation per Section 7.4 of the main spec).

**Output per struct/class:**

```json
{
  "lcom4": 3,
  "method_count": 12,
  "component_sizes": [5, 4, 3],
  "description": "UserService has 3 independent method groups — consider splitting into 3 focused classes"
}
```

### 4.7 Conceptual Integrity

**What it decomposes:** "This system feels like it was designed by a committee."

Fred Brooks argued conceptual integrity — the system reflecting a single coherent design vision — is the most important factor in system quality. It *sounds* unmeasurable. But it has a measurable proxy.

**Dependency Signature Consistency:**

For each module at depth \( d \) (peer modules), compute its "dependency signature": a vector of `[afferent_coupling, efferent_coupling, instability, coupling_ratio, api_surface, fan_in_distribution_shape]`.

Conceptual integrity = consistency of these signatures across peer modules.

\[
\text{CI} = 1 - \text{CoV}(\text{distance\_from\_centroid across peer modules})
\]

where CoV = coefficient of variation.

If all service modules have similar dependency patterns (similar coupling, similar instability, similar API surface), CI is high. If some service modules are tightly coupled, others isolated, some stable, others volatile — CI is low.

**This is a research-grade metric.** It provides genuine signal but requires careful interpretation. Report it as informational, not as a finding.

```json
"conceptual_integrity": {
  "score": 0.73,
  "peer_groups_analyzed": 3,
  "most_consistent_group": "src/services (6 modules, CoV=0.12)",
  "least_consistent_group": "src/utils (4 modules, CoV=0.67)",
  "description": "Service modules follow consistent dependency patterns. Utility modules show highly inconsistent structural roles."
}
```

---

## 5. Tier 3: System-Level Architectural Health

### 5.1 Hub-Authority Structure (HITS Algorithm)

**Source:** Jon Kleinberg — "Authoritative Sources in a Hyperlinked Environment" (1999). Applied to software dependency graphs by multiple researchers.

Beyond fan-in/fan-out and hub score (existing), HITS identifies the *structural backbone* of the system: which nodes are **authorities** (pointed to by many hubs) and which are **hubs** (point to many authorities).

**Why it matters beyond existing hub score:** The existing hub score (Section 4.7.5) measures how many modules a function bridges. HITS reveals the *eigenvector structure* — which nodes are important not just by direct connections but by the importance of their connections. It separates the structural backbone from the periphery.

**Algorithm:** Standard HITS — iterative until convergence:
1. Initialize all authority and hub scores to 1.
2. Authority update: \( a(v) = \sum_{u: u \rightarrow v} h(u) \)
3. Hub update: \( h(v) = \sum_{v \rightarrow w} a(w) \)
4. Normalize both vectors.
5. Repeat until convergence (typically < 20 iterations).

**Output per node (added to `node_annotations`):**

```json
{
  "hits_authority": 0.034,
  "hits_hub": 0.012
}
```

**Output aggregate:**

```json
"structural_backbone": {
  "top_authorities": ["node-id-1", "node-id-2", "node-id-3"],
  "top_hubs": ["node-id-4", "node-id-5", "node-id-6"],
  "backbone_size": 23,
  "backbone_ratio": 0.018,
  "description": "23 nodes (1.8%) form the structural backbone — changes to these nodes have outsized impact"
}
```

**Implementation:** HITS is \( O(k \times E) \) where \( k \) = iterations (typically 20). For 10,000 nodes: trivial.

### 5.2 Layering Violations

If a project declares layers (via boundary fitness functions, Section 3.3) or if layers can be inferred from the containment hierarchy, layering violations count edges that flow "upward" (from lower to higher layer).

This is a subset of boundary fitness functions. When layer ordering is declared:

```json
"layering": {
  "declared_layers": ["data", "services", "handlers", "ui"],
  "upward_edges": 7,
  "total_inter_layer_edges": 89,
  "violation_ratio": 0.079,
  "description": "7 of 89 inter-layer edges violate the declared layer ordering"
}
```

### 5.3 Architectural Erosion Rate

**What it measures:** How fast the structural health metrics are degrading over time.

This requires running `ge-analyze` on multiple snapshots (commits/tags) and computing deltas.

\[
\text{erosion\_rate}(m) = \frac{m_{\text{current}} - m_{\text{baseline}}}{t_{\text{current}} - t_{\text{baseline}}}
\]

for each metric \( m \). Positive erosion = getting worse.

**Implementation:** This is not computed within a single `ge-analyze` run. It's computed by a wrapper that:
1. Runs `ge-analyze` on the current commit.
2. Loads historical health reports (stored per-commit or per-tag).
3. Computes deltas.

**Output (separate from the health report, or as an optional `--compare` flag):**

```json
"erosion": {
  "baseline_commit": "abc123",
  "baseline_date": "2026-01-15",
  "current_date": "2026-02-25",
  "days_elapsed": 41,
  "deltas": {
    "propagation_cost": { "baseline": 0.18, "current": 0.23, "delta": +0.05, "direction": "worse" },
    "avg_coupling": { "baseline": 0.62, "current": 0.67, "delta": +0.05, "direction": "worse" },
    "cycle_ratio": { "baseline": 0.0018, "current": 0.0022, "delta": +0.0004, "direction": "worse" },
    "boundary_violations": { "baseline": 1, "current": 3, "delta": +2, "direction": "worse" }
  },
  "summary": "4 of 4 tracked metrics degraded over the past 41 days"
}
```

**Why this matters:** Individual metrics are snapshots. Erosion rate tells you the *trajectory*. A system with 0.23 propagation cost and rising is more urgent than one at 0.30 and falling.

### 5.4 Complexity Distribution Shape

**What it measures:** Not the mean or max of complexity metrics, but the *shape* of the distribution — specifically, whether there's a dangerous fat tail.

For each complexity metric (body size, fan-in, fan-out, IFC, blast radius), compute:

| Stat | Purpose |
|------|---------|
| Kurtosis | Measures tail heaviness. Normal distribution has kurtosis 3. High kurtosis = fat tail = a few extreme outliers. |
| Skewness | Measures asymmetry. High positive skew = long right tail = a few functions dominate the metric. |
| Tail ratio | Fraction of items > 3× median. If > 5%, the tail is significant. |

**Why the tail matters more than the mean:** A system with mean blast radius 5 and max 8 is fundamentally different from one with mean blast radius 5 and max 500. The mean is identical. The risk profile is not. The second system has a "structural landmine" — a single node whose failure cascades through 100× more of the system than average.

```json
"distribution_shapes": {
  "blast_radius": {
    "kurtosis": 12.4,
    "skewness": 3.8,
    "tail_ratio": 0.06,
    "assessment": "fat_tail",
    "description": "Blast radius has extreme concentration — 6% of nodes have 3× the median impact"
  },
  "fan_in": {
    "kurtosis": 4.2,
    "skewness": 1.9,
    "tail_ratio": 0.03,
    "assessment": "moderate",
    "description": "Fan-in distribution is moderately concentrated"
  }
}
```

---

## 6. The Provability Contract

Every recommendation or warning produced by the system must satisfy these criteria:

### 6.1 Evidence Chain

Every finding links to:
1. **The specific graph elements** (node IDs, edge IDs, module names) that produced it.
2. **The metric value** and **threshold** that triggered it.
3. **The consequence** — what bad thing happens if this is ignored, expressed in structural terms (not opinion).

Example of a *good* finding:

> `src/core` module (I=0.15, stable) depends on `src/handlers` (I=0.85, unstable). If `src/handlers` changes — which it does frequently given its instability — the change propagates into `src/core`, which has 7 incoming dependents. Net propagation: 1 change in handlers could affect 8+ modules.

The consequence is *provable from the graph*. The user can verify it by inspecting the edges.

Example of a *bad* finding:

> "This module has too much coupling. Consider refactoring."

This tells the developer nothing they can verify, nothing about consequences, and nothing about what specifically to change.

### 6.2 Universal Consequence Mapping

Every metric has a mapped consequence that is universally true — not "we think this is bad" but "this structural property mathematically implies this behavior."

| Metric Condition | Provable Consequence |
|-----------------|---------------------|
| Cycle between A, B, C | A, B, C cannot be tested, deployed, or reasoned about independently. Any change to one requires considering all. |
| Blast radius > 90th pctl | A bug in this function has a disproportionate chance of cascading. The number of affected paths is a fact. |
| Stability gradient violation (stable → unstable) | The stable module's dependents are exposed to volatility they didn't sign up for. Count of transitive dependents is a fact. |
| Distance > 0.5 in Zone of Pain | No abstraction exists to absorb change. Every modification to this module requires modifying all N dependents. N is a fact. |
| Propagation cost > 0.4 | More than 40% of module pairs have dependency paths. A change to any module has >40% chance of requiring consideration of another module. This is a mathematical property of the visibility matrix. |
| ISP score > 0.7 | Consumers depend on an average of <30% of the module's interface. They are forced to depend on functionality they don't use. Each unused dependency is a potential breaking change that wastes their time. |
| Feature envy ratio > 0.5 | This function accesses another module more than its own. If the envied module changes, this function in a distant module breaks. The location mismatch is verifiable from the edge list. |

### 6.3 The "Yeah But" Defense

Developers will sometimes say "yeah, I made it like that because..." Valid reasons exist. The system's response must:

1. **Acknowledge the pattern by name** when a known justified exception applies.
2. **Quantify the cost anyway** — even justified decisions have costs. "You chose to have a central dispatcher (hub node). That's a valid pattern. The cost is: blast radius 47, and 3 module boundaries that cannot be independently deployed. Here are the specific paths."
3. **Never suppress findings based on assumed intent.** The finding is a fact about the graph. The recommendation is where the system acknowledges that the developer may have a reason.

Finding format for known-pattern situations:

```json
{
  "recommendation": "This may be intentional (central dispatcher pattern). If so, the cost is: blast radius 47 across 3 module boundaries. Mitigate by ensuring the dispatcher interface is stable (currently I=0.35, which is moderate)."
}
```

---

## 7. The Dashboard Vision

### 7.1 Layout

The dashboard is organized into the functional groups from Section 2, not as a flat list of metrics. Each group answers a question.

```
┌──────────────────────────────────────────────────────────────┐
│  HEADLINE: Propagation Cost 0.23 | Composite Percentile 73  │
├──────────────────────┬───────────────────────────────────────┤
│  SYSTEM RESILIENCE   │  MODULE HEALTH                       │
│  ─────────────────   │  ────────────                        │
│  Propagation: 0.23   │  Modules: 29                         │
│  Tangle: 0.04%        │  Avg Coupling: 0.67                  │
│  Max Depth: 10         │  Avg Distance (D): 0.31              │
│  Cycle Groups: 5       │  Zone of Pain: 2 modules             │
│                        │  Zone of Uselessness: 0              │
│                        │  Avg ISP violation: 0.42             │
├──────────────────────┬───────────────────────────────────────┤
│  DEPENDENCY DISCIPLINE │  HOTSPOT RISK                        │
│  ────────────────────  │  ────────────                        │
│  SGS Violations: 9.0%  │  Hotspot functions: 65 (top 5%)      │
│  Boundary Violations: 3│  Hotspot fan-in share: 46.5%         │
│  Layering Violations: 7│  Top connascence: 24 edges > Name    │
│  Cycles: 5 groups      │  CWC avg: 0.48                       │
│  [▼ Show violated      │  Structural backbone: 23 nodes (1.8%)│
│     edges]             │  [▼ Show top authorities]             │
├──────────────────────┬───────────────────────────────────────┤
│  CODE VITALITY         │  TRAJECTORY                          │
│  ──────────────        │  ──────────                          │
│  Dead code: 10.9%      │  PC Δ: +0.05 (41d) ▲ worse          │
│  Feature envy: 12 fns  │  Coupling Δ: +0.05 ▲ worse          │
│  Fat tail (body): 8%   │  Boundary Δ: +2 violations ▲ worse  │
│  Fat tail (blast): 6%  │  Cycle Δ: +1 group ▲ worse          │
│  LCOM4 > 1: 8 structs  │  Overall: 4/4 metrics degrading     │
└──────────────────────┴───────────────────────────────────────┘
```

### 7.2 Design Principles

**No single number as the hero.** The composite percentile appears in the headline, but it never stands alone. It's always accompanied by propagation cost — the single metric that best captures system-level health.

**Drill-down, not drill-around.** Every number in the dashboard is clickable. Propagation cost → which module pairs have paths → which edges form the path → which nodes are the bottlenecks. The user never has to mentally connect two different screens.

**Color encodes severity, not value.** A coupling score of 0.7 isn't red because 0.7 is "bad" — it's red because it crossed the threshold AND its module is in the Zone of Pain AND it has stability gradient violations. The combination determines color.

**Trajectory is always visible.** The "Trajectory" panel is not buried. It's top-level. A system at percentile 50 and improving is healthier than one at percentile 75 and degrading. Direction matters more than position.

### 7.3 Dashboard Interaction Model

Each metric group supports three levels of interaction:

1. **Glance** — The summary numbers visible without clicking. "Propagation: 0.23, 9 SGS violations, 3 boundary violations." A senior engineer scans this in 5 seconds and knows where to look.

2. **Inspect** — Clicking any number expands to show the contributing elements. Clicking "SGS Violations: 9.0%" expands to show the 14 violating edges, sorted by instability delta. Each edge shows source module, target module, and the instability values.

3. **Investigate** — From any contributing element, jump to the spatial graph view with the relevant nodes/edges highlighted. The graph becomes the evidence. The dashboard is the map; the graph is the territory.

### 7.4 Findings as Actionable Cards

Each finding from ge-analyze is presented as a card with:

```
┌──────────────────────────────────────────────┐
│ 🔴 HIGH | Stability Gradient Violation       │
│                                              │
│ src/core (I=0.12) → src/handlers (I=0.85)   │
│                                              │
│ CONSEQUENCE:                                 │
│ src/core has 7 dependents. A change to       │
│ src/handlers propagates through src/core     │
│ into all 7 — affecting 27% of the system.    │
│                                              │
│ RECOMMENDATION:                              │
│ Extract an interface in src/core that        │
│ src/handlers implements. This inverts the    │
│ dependency so src/core remains stable.       │
│                                              │
│ [View in Graph] [Inspect Affected Paths]     │
└──────────────────────────────────────────────┘
```

Every card has:
- **Severity** — the signal strength.
- **The structural fact** — what the graph shows, in concrete terms.
- **Consequence** — the provable downstream impact, with numbers.
- **Recommendation** — the specific action, acknowledging when patterns may be intentional.
- **Evidence links** — jump to the graph or the path list.

### 7.5 Dashboard-to-Graph Bridge

The dashboard and the spatial graph view (GridSeak's core) are not separate tools — they're two views of the same data. Every metric in the dashboard maps to a visual pattern in the graph:

| Dashboard Metric | Graph Visualization |
|-----------------|-------------------|
| Propagation cost | Heatmap overlay: nodes colored by reachability count |
| Module coupling | Module boundary thickness or color |
| Stability gradient violations | Red edge coloring for violating edges |
| Zone of Pain / Uselessness | Module node coloring (red = pain, grey = uselessness) |
| Hotspot concentration | Node size scaled by fan-in or blast radius |
| Connascence profile | Edge thickness or dash pattern by connascence level |
| Boundary violations | Dotted red edges crossing declared boundaries |
| Cycle membership | Pulsing/glowing cycle edges |
| Feature envy | Cross-module arrows highlighted on envy functions |

The user sees the dashboard number, clicks "View in Graph," and the graph zooms to the relevant region with the appropriate overlay applied. The spatial view makes the abstract metric viscerally comprehensible.

---

## 8. Implementation Priority and Phasing

### 8.1 What Can Ship with Existing Graph Data (No Parser Changes)

| Metric | Data Source | Effort |
|--------|-----------|--------|
| Propagation cost (module-level) | Existing module resolution + adjacency matrix | 1-2 days |
| Abstractness + Distance | Existing `kind` enum (Interface, Struct, Enum, Type) + instability | 1 day |
| Boundary fitness functions | New config TOML section + edge iteration | 2-3 days |
| Stability gradient violations | Existing instability values + edge iteration | 0.5 day |
| Connascence Phase A (edge-kind weights) | Existing edge `kind` field | 0.5 day |
| Information hiding ratio | Existing module resolution + BFS | 1 day |
| Consumer completeness (ISP) | Existing cross-module edges | 1 day |
| Feature envy | Existing per-function outgoing edges | 0.5 day |
| Law of Demeter violations | Existing module-level dependency graph | 0.5 day |
| HITS algorithm | Existing adjacency matrix | 0.5 day |
| Complexity distribution shape | Existing metric values (kurtosis/skewness computation) | 0.5 day |
| Conceptual integrity | Existing per-module metric vectors | 0.5 day |
| **Total** | | **~10 days** |

### 8.2 What Requires Parser Enrichment (Separate Track)

| Metric | Required Parser Change | Effort (Parser) |
|--------|----------------------|-----------------|
| Parameter count per function | Emit `param_count` in function node properties | 1 day |
| Nesting depth per function | Compute max AST depth per function | 1 day |
| Cyclomatic complexity | Count decision nodes per function | 1-2 days |
| LCOM4 | Emit Variable nodes + intra-struct Uses edges | 2-3 days |
| Connascence of Meaning | Extract literal constants from AST | 2 days |
| Connascence of Position | Use param_count + cross-module caller analysis | 0.5 day (after param_count exists) |
| **Total** | | **~8 days** |

### 8.3 What Requires Multi-Snapshot Infrastructure

| Metric | Required Infrastructure | Effort |
|--------|----------------------|--------|
| Architectural erosion rate | Per-commit health report storage + delta computation | 2-3 days |
| Propagation cost trend | Same as above | 0 (included) |
| Boundary violation trend | Same as above | 0 (included) |

### 8.4 Recommended Execution Order

**Sprint 1 (Tier 1 — highest leverage, no parser changes):**
1. Propagation cost (module-level)
2. Abstractness + Distance from Main Sequence
3. Stability gradient violations
4. Connascence Phase A (edge-kind weighting)
5. Boundary fitness functions (config format + evaluation)

**Sprint 2 (Tier 2 — decomposed metrics, no parser changes):**
6. Information hiding ratio
7. Consumer completeness (ISP)
8. Feature envy
9. Law of Demeter violations
10. Complexity distribution shape (kurtosis/skewness on existing metrics)
11. Conceptual integrity

**Sprint 3 (Tier 3 — system-level + parser enrichment, parallel tracks):**
12. HITS algorithm
13. Parser: param_count, nesting depth, cyclomatic complexity
14. Function complexity profile (uses parser enrichment)
15. Connascence Phase B (meaning, position — uses parser enrichment)
16. LCOM4 (uses parser enrichment)

**Sprint 4 (Trajectory):**
17. Multi-snapshot infrastructure
18. Erosion rate computation
19. Trend tracking for all metrics

---

## 9. YouTube Content Strategy

### 9.1 The Teaching Framework

Every video follows the same arc:

1. **The invisible problem** — a structural reality developers live with daily but can't see.
2. **The consequence they've experienced** — a real scenario (refactor gone wrong, bug that spread, onboarding hell) that the audience recognizes.
3. **The metric that makes it visible** — the specific measurement, shown on real code graphs.
4. **The proof** — concrete graph examples from real open-source projects (using the calibration population).
5. **The action** — what to do about it, with before/after graph comparisons.

No video is theoretical. Every video uses real graphs from real projects. The dashboard/visualization is the star — it's the product demo embedded in education.

### 9.2 Video Series: Structural Blindness

**15 videos. 3 narrative arcs. Each video standalone, but together they build a complete understanding of structural health.**

---

#### Arc 1: The System-Level Truths (Videos 1–5)

These cover the metrics that apply to the *whole system* — the things you can't see by reading individual files.

**Video 1: "The Invisible Tax in Every Codebase (And How to Finally See It)"**
- Hook: Every sprint, your team spends time on something nobody planned, nobody budgeted, and nobody can explain to the PM.
- Core: Propagation cost. What it is. How to compute it. What the numbers mean.
- Evidence: Show propagation cost across 5 calibration projects. Compare a well-modularized project (PC=0.08) vs a tangled one (PC=0.45). Visualize the visibility matrix as a heatmap.
- Payoff: The audience can now see, in one number, how much of their system is coupled to every other part. The "invisible tax" has a price tag.
- Duration: 12-15 min.

**Video 2: "Why Your Refactors Keep Breaking Things Nobody Touched"**
- Hook: You changed auth. Tests pass. You deploy. Payments breaks. How?
- Core: Blast radius + stability gradient violations. Stable modules depending on unstable ones create invisible propagation channels.
- Evidence: Walk through a real dependency chain where a stable core module depends on a volatile handler module. Show the 7 downstream dependents that get affected.
- Payoff: The audience understands *why* distant things break. The fix (dependency inversion) is shown as a graph transformation.
- Duration: 10-12 min.

**Video 3: "The Architecture Metric That Predicts Your Next Production Incident"**
- Hook: What if one number could tell you which module will cause your next outage?
- Core: Martin's Distance from Main Sequence. The Zone of Pain. Modules that are stable, concrete, heavily depended upon, and impossible to change safely.
- Evidence: Identify Zone of Pain modules across 3 real projects. Show how their blast radius and fan-in correlate with defect hotspot data.
- Payoff: The audience knows how to identify their most dangerous modules *before* they cause incidents.
- Duration: 12-15 min.

**Video 4: "Your Architecture Is Eroding — Here's the Proof"**
- Hook: Your codebase was clean 6 months ago. What happened?
- Core: Architectural erosion rate. Running analysis across commits to see metrics trend over time.
- Evidence: Show propagation cost, coupling, and boundary violation trends across 20 commits of a real project. Identify the exact commit where erosion accelerated.
- Payoff: The audience understands that architecture isn't a one-time decision — it's a property that degrades unless measured and maintained.
- Duration: 10-12 min.

**Video 5: "The 3 Numbers That Tell You If Your Codebase Is Dying"**
- Hook: You don't need 50 metrics. You need 3.
- Core: Propagation cost + tangle index + boundary violation count. The "vital signs" of a codebase.
- Evidence: Show how these 3 numbers correlate across the calibration population. Projects where all 3 are healthy vs projects where one or more are critical.
- Payoff: The audience has a minimal, actionable health check they can run today.
- Duration: 8-10 min.

---

#### Arc 2: The Module-Level Reality (Videos 6–10)

These cover how individual modules succeed or fail structurally.

**Video 6: "Your Modules Are Lying About Their Boundaries"**
- Hook: You drew the boundaries. The code ignores them.
- Core: Boundary fitness functions + information hiding ratio. The gap between intended and actual architecture.
- Evidence: Declare 3 simple boundary rules on a real project. Show the violations. Show how IHR reveals that a "well-encapsulated" module with 2 exports is actually 90% reachable from outside.
- Payoff: The audience learns to declare architectural intent and measure drift automatically.
- Duration: 12-15 min.

**Video 7: "Not All Coupling Is Equal — And You're Measuring It Wrong"**
- Hook: Two modules both have coupling score 0.65. One is fine. The other is a time bomb. Why?
- Core: Connascence taxonomy. Coupling quality vs coupling quantity.
- Evidence: Compare two modules with identical coupling scores but different connascence profiles. One has mostly name-level coupling (safe). The other has meaning and position coupling (dangerous).
- Payoff: The audience stops treating coupling as a single number and starts asking *what kind* of coupling.
- Duration: 15-18 min.

**Video 8: "The Interface Nobody Uses (And Why It's Costing You)"**
- Hook: You exported 15 functions. Each consumer uses 3. What went wrong?
- Core: Consumer completeness (ISP violations). Fat interfaces that force consumers to depend on things they don't need.
- Evidence: Walk through a real utils module. Show per-consumer usage. Show how splitting the module based on consumer clusters improves coupling, reduces blast radius, and makes the dependency graph cleaner.
- Payoff: The audience has a concrete method for deciding when and how to split modules.
- Duration: 10-12 min.

**Video 9: "This Function Is in the Wrong Module (And Your Tests Prove It)"**
- Hook: A test breaks. You check git blame. The function hasn't changed. But the module it *depends on* did.
- Core: Feature envy. Functions that reference another module more than their own.
- Evidence: Identify feature envy functions in a real project. Show how moving them to the envied module reduces cross-module edges, improves coupling scores, and makes the dependency graph simpler.
- Payoff: The audience has a refactoring pattern grounded in graph metrics, not gut feeling.
- Duration: 8-10 min.

**Video 10: "Why Copy-Paste Is Sometimes Better Than Abstraction"**
- Hook: You extracted a shared utility. Now 8 modules depend on it. Was that the right call?
- Core: Zone of Pain (stable concrete modules), Zone of Uselessness (unused abstractions), and the abstraction trade-off.
- Evidence: Show a real shared utility module that became a bottleneck. Compare the propagation cost before and after the extraction. Show cases where duplication would have been structurally cheaper.
- Payoff: The audience learns that abstraction has a structural cost, and sometimes controlled duplication is the healthier choice.
- Duration: 12-15 min.

---

#### Arc 3: The Forward-Looking Reality (Videos 11–15)

These cover what changes as AI, agents, and MCP reshape software architecture.

**Video 11: "Why AI Agents Will Destroy Architectures Humans Got Away With"**
- Hook: AI generates code 10× faster. Your architecture was designed for human speed. What breaks?
- Core: When code volume increases and change frequency increases (because agents ship faster), every structural weakness amplifies. High propagation cost at human pace is tolerable. At agent pace, it's catastrophic.
- Evidence: Simulate doubling the change frequency against propagation cost. Show how "rework cost per sprint" scales with PC × change frequency.
- Payoff: The audience understands that AI doesn't remove the need for architecture — it makes architecture the *only* thing that matters.
- Duration: 15-18 min.

**Video 12: "Your MCP Server Is a Dependency Graph (And It Has Bugs You Can't See)"**
- Hook: You wired 8 MCP tools together. Looks fine. But what happens when one fails?
- Core: Applying structural health metrics to tool/agent topology. Fan-in, fan-out, cycles, blast radius — applied to MCP server capabilities instead of functions.
- Evidence: Model a real MCP server setup as a graph. Run the same metrics. Show the "hotspot tool" and the "cycle between tools" that create invisible failure modes.
- Payoff: The audience sees that the same structural thinking applies to agent systems, not just code.
- Duration: 12-15 min.

**Video 13: "The 40-Year-Old Architecture Idea Silicon Valley Forgot"**
- Hook: In 1977, an architect (not a software architect — a *buildings* architect) solved the problem every software architect still struggles with.
- Core: Christopher Alexander's pattern theory. Not the Gang of Four version — the *original* idea about forces in tension resolving into form. His 15 properties of living systems. How they map to software structure.
- Evidence: Take Alexander's "strong centers" and "boundaries" properties and show how they correlate with the metrics: modules with high conceptual integrity (consistent dependency signatures) exhibit "strong centers." Modules with high information hiding ratio exhibit healthy "boundaries."
- Payoff: The audience discovers a framework for architectural quality that predates and subsumes SOLID, Clean Architecture, and every other heuristic they've been taught.
- Duration: 18-22 min.

**Video 14: "Supervision Trees: The Error Handling Pattern That Prevents Cascading Failures"**
- Hook: Your service crashed. The retry logic crashed. The fallback crashed. The alert fired 47 times. Sound familiar?
- Core: Joe Armstrong's supervision tree model from Erlang/OTP. Failure is a structural topology problem, not a code problem. How "let it crash" is an *architectural decision* about where failures are contained.
- Evidence: Model a real service's error handling as a dependency graph. Show how the blast radius of failure maps directly to the supervision hierarchy (or lack thereof). Compare a flat error-handling architecture vs a hierarchical one.
- Payoff: The audience gets a concrete pattern for failure containment that applies to any language, especially to AI agent orchestration systems.
- Duration: 12-15 min.

**Video 15: "How to Read Your Codebase Like a Structural Engineer Reads a Building"**
- Hook: A structural engineer doesn't read every brick. They read the load paths. You can read your codebase the same way.
- Core: The complete dashboard. All metrics working together. The drill-down from headline (propagation cost) to module health (coupling, distance, ISP) to node risk (hotspot, connascence, feature envy) to trajectory (erosion).
- Evidence: Live walkthrough of a full health report on a real project. Start from the headline, drill into the worst finding, trace it through the graph, and propose a fix. Then show the projected impact of the fix on propagation cost.
- Payoff: The audience has a complete mental model for reading architectural health. The dashboard is the interface. The graph is the evidence. The metrics are the vocabulary.
- Duration: 20-25 min.

---

### 9.3 Series Summary

| # | Title | Core Metric | Arc | Duration |
|---|-------|------------|-----|----------|
| 1 | The Invisible Tax in Every Codebase | Propagation cost | System | 12-15 min |
| 2 | Why Your Refactors Keep Breaking Things | Blast radius + stability gradient | System | 10-12 min |
| 3 | The Metric That Predicts Your Next Incident | Distance from Main Sequence | System | 12-15 min |
| 4 | Your Architecture Is Eroding — Here's Proof | Erosion rate | System | 10-12 min |
| 5 | The 3 Numbers That Tell You If Your Codebase Is Dying | PC + tangle + boundary violations | System | 8-10 min |
| 6 | Your Modules Are Lying About Their Boundaries | Boundary fitness + IHR | Module | 12-15 min |
| 7 | Not All Coupling Is Equal | Connascence taxonomy | Module | 15-18 min |
| 8 | The Interface Nobody Uses | Consumer completeness / ISP | Module | 10-12 min |
| 9 | This Function Is in the Wrong Module | Feature envy | Module | 8-10 min |
| 10 | Why Copy-Paste Is Sometimes Better Than Abstraction | Zone of Pain + abstraction cost | Module | 12-15 min |
| 11 | Why AI Agents Will Destroy Architectures Humans Got Away With | PC × change frequency | Forward | 15-18 min |
| 12 | Your MCP Server Is a Dependency Graph | Structural metrics on tool graphs | Forward | 12-15 min |
| 13 | The 40-Year-Old Architecture Idea Silicon Valley Forgot | Alexander's 15 properties | Forward | 18-22 min |
| 14 | Supervision Trees: The Pattern That Prevents Cascading Failures | Armstrong's supervision model | Forward | 12-15 min |
| 15 | Read Your Codebase Like a Structural Engineer | Full dashboard walkthrough | Forward | 20-25 min |
| | **Total** | | | **~195 min** |

### 9.4 Content Strategy Notes

**Each video is a product demo disguised as education.** The dashboard, the graph visualization, the drill-down — these are GridSeak's interface. The audience learns structural thinking AND sees the product in action, without the video ever feeling like a sales pitch.

**The calibration population is the evidence base.** Every claim is grounded in data from 300-500 real open-source projects. "Healthy projects have PC below 0.15" is not an opinion — it's a statistical fact from the population.

**Videos 11-14 position GridSeak for the future.** By showing that these metrics apply to MCP/agent systems, the audience sees GridSeak as forward-looking, not just a static analysis tool for legacy code.

**Video 13 (Alexander) is the differentiator.** No other tool or content creator has connected Alexander's original architectural theory to measurable software metrics. This video establishes intellectual credibility beyond "we built another linter."

---

## 10. Relationships Between All Metrics

The following diagram shows how every metric in this document relates to every other. Arrows indicate "explains" or "contributes to."

```
HEADLINE METRIC
    Propagation Cost ←──────────────────────────────────────────┐
        ↑ explained by                                          │
        │                                                       │
TIER 1 (Why is propagation high?)                               │
    ├── Module Coupling ────────→ contributes to PC              │
    ├── Stability Gradient Violations ──→ amplifies propagation  │
    ├── Cycle Count ────────────→ creates unbounded propagation  │
    ├── Distance (Zone of Pain) ──→ no abstraction to absorb     │
    └── Connascence Weight ─────→ harder to decouple = stickier  │
                                                                │
TIER 2 (Why is the module unhealthy?)                           │
    ├── Information Hiding Ratio ──→ explains coupling quality   │
    ├── Consumer Completeness ────→ explains interface bloat     │
    ├── Feature Envy ─────────────→ explains misplaced coupling  │
    ├── Law of Demeter ───────────→ explains transitive coupling │
    ├── LCOM4 ────────────────────→ explains intra-module mess   │
    └── Function Complexity ──────→ explains per-node risk       │
                                                                │
TIER 3 (What's the big picture?)                                │
    ├── HITS (backbone) ──────────→ identifies leverage points   │
    ├── Layering Violations ──────→ structural rule-breaking ────┘
    ├── Conceptual Integrity ─────→ design consistency
    ├── Complexity Distribution ──→ risk tail detection
    └── Erosion Rate ─────────────→ trajectory (getting worse?)

EXISTING METRICS (already in ge-analyze spec):
    Fan-in/Fan-out, Blast radius, IFC, Hub score, API surface,
    Tangle index, Dead code, Depth, Instability
    → All feed upward into Tier 1 and Tier 2 explanations
```

The critical insight: **Propagation Cost is the "GDP" of the codebase.** It's the single number that captures system-level health. Every other metric exists to *explain* why PC is what it is and *prescribe* how to improve it. The dashboard is organized around this hierarchy — headline first, then drill into causes, then drill into evidence.

---

## 11. Open Design Questions

1. **Should propagation cost be computed at function-level too?** Module-level is the headline. But function-level PC (sampled) would reveal which specific functions contribute most to system-wide propagation. Cost: the sampled BFS approach is feasible. Benefit: "this function contributes 12% of total system propagation" is a powerful finding.

2. **How should connascence interact with the composite percentile?** Option A: CWC replaces raw coupling in the percentile computation. Option B: CWC is a separate metric with its own percentile. Recommendation: Option B initially, migrate to Option A once connascence detection matures past Phase A.

3. **Should boundary rules be version-controlled with the project?** If the rules live in a TOML file in the repo, they evolve with the code. But they also need a way to be *suggested* by ge-analyze for projects without declared rules. The auto-inference (Section 3.3.5) should produce a candidate `.gridseak/boundaries.toml` that the developer can review and commit.

4. **How should the dashboard handle projects with no population data?** Layer A metrics (absolute) always work. Layer B percentiles require population. For the Tier 1 additions, propagation cost has well-established benchmarks (MacCormack papers). Distance has a clear scale (0-1). Boundary violations are binary. These can be interpreted without population data. Recommendation: add "benchmark" interpretation alongside percentile (when available).

5. **Should LCOM4 be gated behind a parser capability flag?** If the parser doesn't emit Variable nodes for a given language, LCOM4 silently skips. But should the report indicate "LCOM4 not available: parser does not emit field-level data for TypeScript"? Recommendation: yes, in the `analysis_errors` block. Transparency about what wasn't measured is as important as what was.

---

## 12. LLM Enrichment: Reducing Developer-in-the-Loop Without Sacrificing Reliability

### 12.1 The Problem

Ten points in this document require developer intent or contextual judgment that the graph alone cannot provide. Without addressing these, the system either demands manual configuration (reducing adoption) or produces findings that miss intent (reducing trust).

### 12.2 Every Point Requiring Developer Intent

| # | What | Section | Why Human Needed | Consequence if Missing |
|---|------|---------|-----------------|----------------------|
| 1 | Boundary rule declarations | 3.3 | Only the developer knows the *intended* architecture | Entire "Dependency Discipline" dashboard group has a hole — no boundary violations, no layering violations |
| 2 | Layer ordering + module-to-layer mapping | 3.3 | Developer must say "src/routes = handlers layer" | Layering violations can't be computed |
| 3 | Module depth setting | Main spec 5.0.3 | Default 2 works for most, but some projects need 1 or 3 | Wrong module boundaries → noisy coupling scores |
| 4 | Dead code entry point patterns | Main spec 4.4 | Project-specific frameworks invoke functions the heuristic list doesn't cover | False positives destroy trust fastest of any metric |
| 5 | Connascence of Meaning classification | 3.5 | Is a shared constant intentional protocol or accidental coupling? | False positives in connascence findings |
| 6 | Feature envy justification | 4.4 | Adapters, mediators, and facades *intentionally* reference foreign modules | Valid patterns flagged as problems |
| 7 | Zone of Pain assessment | 3.2 | A stable-concrete core module might be intentional design | Findings on correct architecture waste developer attention |
| 8 | LCOM4 interpretation | 4.6 | A facade class *should* have multiple independent method groups | Split recommendations on intentional facades |
| 9 | Finding suppression / acceptance | 6.3 | Developer says "I know, it's intentional" | Same findings re-appear every run, noise |
| 10 | Ecosystem profile for mixed repos | Main spec 5.0.1 | A repo with both TypeScript and Rust needs per-subtree profiles | Wrong heuristics applied to wrong files |

### 12.3 LLM Inference Reliability Classification

#### High Confidence — Reliable Enough to Be Default-On When LLM Is Enabled

**Boundary rule inference (#1, #2):** The agent reads module names (from path-prefix grouping, already computed), the inter-module dependency matrix (already computed), README.md or top-level docs (if they exist), and module export patterns (which modules export to which). From this, it proposes: "This project has a layered structure. `src/db` is data access (only imports from libraries, everyone calls into it). `src/services` is business logic (calls db, called by routes). `src/routes` is the handler layer (calls services, imports from framework)."

The LLM isn't guessing — it's reading the same structural signals a human architect reads, plus directory naming conventions that carry strong semantic signal.

**Dead code false positive reduction (#4):** The agent reads each potentially-dead function's source code (first 30 lines + signature), FQN, and module context. It determines: "This function has the signature `(ctx: Context, next: Next) => Promise<void>` and is exported from `src/middleware/cors.ts`. It's a Koa middleware invoked by the router — not dead."

**Module role classification (#10, feeds into #2):** The agent reads module-level exports and primary dependencies to classify each module as `data_access`, `business_logic`, `presentation`, `infrastructure`, `utility`, `test`, or `integration`.

#### Medium Confidence — Opt-In, Clearly Labeled as LLM-Inferred

**Feature envy justification (#6):** The agent reads the function source + its module context + the envied module context. It distinguishes: "`PaymentFormatter` in `src/utils` only references `src/payments` types — it should move" vs "`PaymentAdapter` in `src/integration` bridges `src/payments` to an external API — the cross-module references are its *purpose*."

**Zone of Pain assessment (#7):** The agent reads the module's exports, its dependents, and its source summary. It assesses: "`src/core/types.ts` is stable and concrete because it defines fundamental types. Every module imports it. This is correct design." vs "`src/utils/helpers.ts` is stable and concrete because 30 modules dump unrelated functions here. This is a dumping ground."

**Connascence of Meaning (#5):** The agent reads shared constant values + usage context. It determines: "Both modules use `MAX_RETRIES = 3` — shared policy that should be in config" vs "Both modules use `200` — HTTP success code, not coupling."

#### Low Confidence — Only On Explicit Request

**LCOM4 interpretation (#8):** Whether splitting a class is appropriate requires deep domain knowledge. The LLM provides a *suggestion* but should not auto-classify.

**Finding suppression (#9):** Not LLM-inferred. When a developer says "I know about this," the system stores that decision as a committed rule in the boundaries TOML. No AI needed — it's a workflow feature.

### 12.4 Architecture: Separate Enrichment Stage

**`ge-analyze` stays purely deterministic.** The LLM enrichment is a separate, composable pipeline stage.

```
STAGE 1 (deterministic, always runs):
    ge-analyze --db graph.sqlite --output health.json

STAGE 2 (non-deterministic, opt-in):
    ge-enrich --health health.json \
              --db graph.sqlite \
              --source-root ./src \
              --output enriched-health.json \
              --config enrichment.toml
```

**Why two binaries, not a flag:**

- Preserves the determinism guarantee (Section 7.3 of main spec): same input → identical output
- `ge-analyze` requires no network access, runs offline, runs in CI reproducibly
- The enrichment stage can improve independently (better models, better prompts) without touching the analysis engine
- A developer who doesn't trust LLM inference gets zero LLM involvement — not by suppressing it, but because the LLM code literally never executes

### 12.5 MCP Integration: How ge-enrich Talks to the LLM

`ge-enrich` is an MCP client that calls a configured LLM server. Two primary MCP tools:

**Tool 1: `assess_finding`**

```json
{
  "tool": "assess_finding",
  "input": {
    "finding": {
      "type": "feature_envy",
      "severity": "info",
      "description": "formatReceipt in src/utils references 7 symbols in src/payments but only 2 in src/utils",
      "metric_value": 0.78
    },
    "function_source": "export function formatReceipt(payment: Payment, config: ReceiptConfig): string {\n  const amount = payment.amount.toFixed(2);\n  ...",
    "module_context": {
      "name": "src/utils",
      "exports": ["formatReceipt", "formatDate", "truncateString", "..."],
      "dependencies": ["src/payments", "src/config"]
    },
    "graph_neighborhood": {
      "incoming_edges": [{"from": "src/routes/checkout::renderConfirmation", "kind": "Call"}],
      "outgoing_edges": [{"to": "src/payments/types::Payment", "kind": "Type"}, "..."]
    }
  },
  "output": {
    "assessment": "likely_intentional",
    "confidence": 0.82,
    "reasoning": "formatReceipt is a presentation adapter that formats payment data for display. Its purpose is cross-module translation — the feature envy pattern is the function's design intent.",
    "suggested_action": "suppress"
  }
}
```

**Tool 2: `infer_boundaries`**

```json
{
  "tool": "infer_boundaries",
  "input": {
    "modules": [
      {"name": "src/db", "node_count": 45, "exports": ["getUserById", "saveTransaction", "..."], "dependencies": []},
      {"name": "src/services", "node_count": 120, "exports": ["PaymentService", "UserService", "..."], "dependencies": ["src/db"]},
      {"name": "src/routes", "node_count": 80, "exports": ["paymentRouter", "userRouter", "..."], "dependencies": ["src/services"]},
      {"name": "src/middleware", "node_count": 30, "exports": ["authMiddleware", "corsMiddleware", "..."], "dependencies": []}
    ],
    "dependency_matrix": [[0,0,0,0],[1,0,0,0],[0,1,0,0],[0,0,0,0]],
    "readme_excerpt": "Express-based payment processing API...",
    "directory_tree": "src/\n  db/\n  services/\n  routes/\n  middleware/\n  config/"
  },
  "output": {
    "proposed_rules": [
      {"type": "layer_order", "layers": ["db", "services", "routes"], "confidence": 0.92,
       "reasoning": "Dependency flow is strictly db ← services ← routes with zero reverse edges"},
      {"type": "no_depend", "from": "src/routes", "to": "src/db", "confidence": 0.88,
       "reasoning": "Routes access db only through services — 0 direct edges. This boundary appears intentional."}
    ],
    "module_roles": {
      "src/db": {"role": "data_access", "confidence": 0.95},
      "src/services": {"role": "business_logic", "confidence": 0.90},
      "src/routes": {"role": "presentation", "confidence": 0.87},
      "src/middleware": {"role": "infrastructure", "confidence": 0.93}
    }
  }
}
```

### 12.6 Context Assembly Strategy

For each LLM call, assemble the **minimum context that guarantees reliable inference** — not the entire codebase.

| Assessment Type | Context Assembled | Token Budget | Batch Strategy |
|----------------|------------------|-------------|---------------|
| Dead code assessment | Function signature + first 30 lines + FQN + parent file name | ~500 tokens/function | Batch 20 functions per call |
| Feature envy | Function source + own module exports + envied module exports | ~800 tokens/function | Batch 5 per call |
| Boundary inference | Module names + dependency matrix + README + directory tree | ~2K tokens total | Single call per project |
| Finding assessment | Finding details + function source + module context | ~600 tokens/finding | Batch 10 per call |
| Module role classification | Module name + top 5 exports + primary dependencies | ~300 tokens/module | Batch all modules in one call |

**Total LLM cost for a full enrichment pass on a 10,000-node project:** approximately 15K–25K tokens input, 3K–5K output. Under $0.05 at current API pricing. Latency: 5–15 seconds.

### 12.7 Configuration

```toml
[llm_enrichment]
enabled = false                    # Master switch. Default OFF. Pure structural only.
provider = "anthropic"             # or "openai", "local", "mcp_server"
model = "claude-sonnet-4-20250514"          # Specific model for reproducibility notes

[llm_enrichment.capabilities]
infer_boundaries = true            # Propose boundary rules from structure
reduce_dead_code_fps = true        # Assess potentially-dead functions
assess_findings = true             # Add intentional/debt classification to findings
classify_module_roles = true       # Infer data/service/ui/infra roles

[llm_enrichment.controls]
confidence_threshold = 0.75        # Only include assessments above this confidence
max_tokens_per_run = 50000         # Budget cap per enrichment run
batch_size = 20                    # Functions per LLM call
timeout_seconds = 30               # Per-call timeout

[llm_enrichment.transparency]
label_all_inferences = true        # Every LLM conclusion is visibly tagged in output
include_reasoning = true           # Show the LLM's reasoning in finding detail
include_model_version = true       # Record which model produced each assessment
```

### 12.8 Output Contract: LLM Conclusions Never Replace Structural Facts

Every LLM-derived conclusion is wrapped in a distinct `llm_enrichment` object that **annotates** a finding — it never replaces or removes it:

```json
{
  "id": "envy-3",
  "type": "feature_envy",
  "severity": "info",
  "description": "formatReceipt in src/utils references 7 symbols in src/payments but only 2 in src/utils",
  "node_ids": ["node-formatReceipt"],
  "metric_value": 0.78,
  "recommendation": "Consider moving formatReceipt to src/payments where it has the strongest affinity.",

  "llm_enrichment": {
    "assessment": "likely_intentional",
    "confidence": 0.82,
    "reasoning": "formatReceipt is a presentation adapter that formats payment data for display. Its purpose is cross-module translation. The feature envy pattern is the function's design intent, not misplacement.",
    "suggested_action": "suppress",
    "model": "claude-sonnet-4-20250514",
    "assessed_at": "2026-02-25T14:30:00Z"
  }
}
```

When `llm_enrichment.enabled = false`, the `llm_enrichment` key is absent. The finding is identical to what ge-analyze produces. The dashboard renders findings with or without enrichment — the enrichment is purely additive.

### 12.9 Impact Ranking: Where LLM Enrichment Pays Off Most

| Rank | Capability | Impact | Why |
|------|-----------|--------|-----|
| 1 | **Boundary inference** | Eliminates the biggest manual step. Unlocks "Dependency Discipline" for projects with zero config. | Without this, boundary violations are empty for ~95% of projects — those that don't write TOML rules. |
| 2 | **Dead code FP reduction** | Directly improves trust. Dead code has the highest false positive rate. | One bad flag = "this tool doesn't understand my code." |
| 3 | **Finding assessment** | Turns "14 findings" into "8 real problems + 6 intentional patterns." | Developers drown in findings they must manually triage. The LLM does triage. |
| 4 | **Module role classification** | Feeds into boundary inference + stability gradient interpretation. | "This is data access" vs "this is business logic" is the prerequisite for automatic layer analysis. |
| 5 | **Feature envy / Zone of Pain** | Reduces noise on medium-confidence findings. | These are the findings where "yeah but I did that on purpose" is most common. |

### 12.10 The Bootstrap Pattern: LLM as One-Time Intent Bootstrapper

The key design principle: **the LLM bootstraps intent declarations; the graph math enforces them permanently.**

```
First run (with LLM):
    ge-analyze → health.json
    ge-enrich  → enriched-health.json
        Proposes: 5 boundary rules, 29 module role classifications
        Reduces: 143 dead code findings → 89 confirmed + 54 likely FPs
        Assesses: 14 findings → 8 real + 6 likely intentional

Developer reviews proposed boundaries → commits to .gridseak/boundaries.toml
Developer reviews dead code suppressions → commits to .gridseak/suppressions.toml

Every subsequent run (no LLM needed):
    ge-analyze --config .gridseak/boundaries.toml → health.json
        Boundary violations: computed deterministically from committed rules
        Dead code: suppressed functions excluded deterministically
        All findings: fully structural, fully reproducible, zero LLM dependency
```

The LLM gets the developer to the point where the deterministic system takes over. It's not an ongoing dependency — it's a setup accelerator. Subsequent runs are fully offline, fully deterministic, fully reproducible. The LLM never touches the analysis again unless the developer explicitly re-runs `ge-enrich` to reassess after major structural changes.

### 12.11 What the Developer Experiences

**Without LLM (default):**

```
$ ge-analyze --db graph.sqlite --output health.json

14 findings, 3 high severity
No boundary violations (no rules declared)
Dead code: 143 functions (includes some FPs)
Feature envy: 12 functions (includes some adapters)
Propagation cost: 0.23
```

**With LLM enabled (first run):**

```
$ ge-analyze --db graph.sqlite --output health.json
$ ge-enrich --health health.json --db graph.sqlite --source-root ./src --output enriched.json

14 findings → 8 confirmed problems + 6 likely intentional (labeled)
5 boundary rules proposed → review and commit to .gridseak/boundaries.toml
Dead code: 143 → 89 confirmed + 54 likely entry points (labeled)
Feature envy: 12 → 7 real + 5 adapter patterns (labeled)
Module roles: all 29 modules classified (data/service/ui/infra)
```

**After committing boundaries (all subsequent runs, no LLM):**

```
$ ge-analyze --db graph.sqlite --config .gridseak/boundaries.toml --output health.json

8 findings, 3 high severity
3 boundary violations detected (deterministic, from committed rules)
Dead code: 89 functions (suppressed list applied)
Feature envy: 7 functions (suppressed list applied)
Propagation cost: 0.23
Layering violations: 2 upward edges detected
```

The system improves permanently from a single LLM-assisted session.