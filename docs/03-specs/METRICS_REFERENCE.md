# Metrics Reference

Complete list of metrics measured by `ge-analyze` during codebase analysis.

## Graph-Level Metrics

| Metric | Description |
|--------|-------------|
| **Cycle Detection** | Finds circular dependency chains where functions call each other in a loop (A → B → C → A) |
| **Tangle Index** | Percentage of structural edges that participate in cycles; measures how "tangled" the dependency graph is |
| **Max Call Depth** | Longest chain of function calls from entry point to leaf; indicates how deep execution flows go |

## Function-Level Metrics

| Metric | Description |
|--------|-------------|
| **Fan-In** | Number of other functions that call a given function (incoming dependencies) |
| **Fan-Out** | Number of functions a given function calls (outgoing dependencies) |
| **Blast Radius** | Count of downstream nodes transitively affected if a function changes |
| **Hotspot Detection** | Flags functions in the top percentile of fan-in that also have high blast radius |
| **Hub Score** | Measures how many distinct modules a function bridges as caller/callee; identifies cross-cutting bottlenecks |
| **Information Flow Complexity** | Fan-in × fan-out; quantifies how much data flows through a function |
| **Dead Code** | Functions with zero incoming call edges (potentially unreachable) |
| **Entry Points** | Functions reachable from outside the codebase (main, exported APIs, framework hooks) |
| **Cyclomatic Complexity** | Number of independent execution paths through a function |
| **Cognitive Complexity** | Weighted measure of how hard a function is to understand (nesting, breaks in linear flow) |
| **LOC** | Lines of code per function |
| **God Function** | Functions exceeding thresholds on all three: cyclomatic complexity, fan-out, and LOC simultaneously |

## Module-Level Metrics

| Metric | Description |
|--------|-------------|
| **Coupling** | Ratio of external edges to total edges for a module; measures dependency on other modules |
| **Cohesion (LCOM4)** | Whether a module's functions form connected groups or disconnected clusters of unrelated logic |
| **Instability** | Efferent coupling / (afferent + efferent); measures a module's susceptibility to change |
| **Abstractness** | Ratio of abstract types (traits/interfaces) to total types in a module |
| **Distance from Main Sequence** | \|Abstractness + Instability − 1\|; flags modules in the "zone of pain" (concrete + heavily depended on) or "zone of uselessness" (abstract + rarely used) |
| **API Surface Ratio** | Percentage of a module's functions called externally; low encapsulation = too much exposed |
| **Layer Violations** | Detects when higher-level code bypasses intermediate abstraction layers to call deep infrastructure directly |
| **Layer Depth** | BFS-inferred layer assignment (entry, controller, service, infrastructure) based on call depth |

## Temporal Metrics

Requires `--git-dir` flag.

| Metric | Description |
|--------|-------------|
| **Temporal Coupling** | File/module pairs that frequently change together in git commits, especially those with no import relationship (hidden coupling) |

## Classification & Quality Signals

| Metric | Description |
|--------|-------------|
| **Structural Classification** | Identifies test, test-support, vendor, generated, and production files |
| **Resolution Quality** | Reports whether import edges used LSP (full), heuristic, or none; affects confidence of cross-file metrics |
| **Per-Metric Confidence** | Tags each metric dimension (depth, dead code, coupling, blast radius) with High/Medium/Low reliability based on graph quality |

## Composite Score

| Metric | Description |
|--------|-------------|
| **Health Score** | Weighted 0–100 composite of cycle severity, coupling, hotspot concentration, dead code ratio, depth, complexity, cohesion, distance, and temporal coupling |

---

# Formula Cheat Sheet








### Variables

| Symbol | Meaning |
|---|---|
| `E_total` | total structural edges (non-containment) |
| `E_cycle` | edges participating in any SCC of size > 1 |
| `N` / `N_dead` | function count / functions with zero incoming call edges |
| `Ca` / `Ce` | module afferent (incoming) / efferent (outgoing) coupling |
| `Na` / `Nc` | abstract types / total types in module |
| `fan_in(f)` / `fan_out(f)` | #distinct callers / callees of `f` |
| `A` / `I` | abstractness / instability of a module |

### Function-level

| Metric | Formula |
|---|---|
| `fan_in`, `fan_out` | edge counts (Call + framework-dispatch kinds) |
| `blast_radius(f)` | `#nodes reachable via reverse-BFS over production edges` |
| `IFC(f)` | `(fan_in · fan_out)²` |
| `hub_score(f)` | `(#src_modules · #tgt_modules) / #modules²` |
| `cyclomatic`, `cognitive`, `LOC` | parser-emitted (tree-sitter traversal) |
| `god_function(f)` | `cc ≥ 10 ∧ fan_out ≥ 8 ∧ loc ≥ 40` |
| `hotspot(f)` | top `(100−hotspot_percentile)%` of `fan_in` ∧ `blast_radius ≥ median` |

### Module-level

| Metric | Formula |
|---|---|
| `coupling(m)` | `E_external / (E_internal + E_external)` |
| `instability(m)` | `Ce / (Ca + Ce)` |
| `abstractness(m)` | `Na / Nc` |
| `distance(m)` | `abs(A + I − 1)` |
| `cohesion(m)` (LCOM4) | `1` if 1 connected component, else `1 / components` |
| `api_surface(m)` | `exported_fns / total_fns` |
| `layer_violation(u→v)` | `layer_gap ≥ layer_violation_min_gap` |

### Graph-level

| Metric | Formula |
|---|---|
| `cycles` | Tarjan SCCs of size > 1 |
| `tangle_index` | `E_cycle / E_total` |
| `max_call_depth` | longest topological path through Call-like edges (cycles broken) |

### Temporal (requires `--git-dir`)

| Metric | Formula |
|---|---|
| `co_change(a,b)` | #commits touching both `a` and `b` |
| `coupling_score(a,b)` | `co_change(a,b) / max_changes(a,b)` |
| `hidden_coupling` | `coupling_score ≥ temporal_hidden_high_score` ∧ no structural edge |

### Composite health score

```
score = Σᵢ wᵢ · percentileᵢ      (when norms DB present)
      = Σᵢ wᵢ · fallbackᵢ(value) (otherwise)
```

| `wᵢ` | cycle | coupling | hotspot | dead_code | depth | complexity | cohesion | distance | temporal |
|---|---|---|---|---|---|---|---|---|---|
|  | 0.20 | 0.18 | 0.15 | 0.10 | 0.10 | 0.10 | 0.07 | 0.05 | 0.05 |

### Reliability gates

| Rule | Condition |
|---|---|
| cycle / tangle shown only if | `E_total ≥ 50` |
| depth shown only if | `call_edges ≥ 20` |
| framework-invisible if | `heuristic_fallback_rate > 0.30` (declarative ecosystems) |