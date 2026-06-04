# GridSeak — Size Bias and Normalization

**Created:** 2026-03-07
**Purpose:** Document which metrics are size-biased, why that's a problem, and the exact fixes needed. This is an engineering concern that directly affects product credibility.

---

## The Problem

Some metrics naturally produce worse scores for larger codebases, regardless of actual code quality. If a 50,000-function enterprise monolith gets a worse health score than a 500-function utility library simply because it's bigger, the score is measuring size, not health.

An engineering manager who sees a score of 45/100 needs to trust that means "your code has structural problems" — not "your code is big."

---

## Audit: Which Metrics Are Size-Biased?

### Already Size-Normalized (Safe)

These use ratios or averages — they don't grow with repo size:

| Metric | Formula | Why it's safe |
|---|---|---|
| Cycle ratio | `cycle_nodes / total_nodes` | Proportion, not count |
| Coupling | `external_edges / total_edges` per module, then averaged | Per-module ratio, then averaged |
| Dead code ratio | `dead_functions / total_functions` | Proportion |
| Hotspot concentration | `sum_hotspot_fan_in / total_fan_in` | Proportion of fan-in captured by hotspots |
| Avg cyclomatic complexity | Mean across functions | Average, not sum |
| Avg cohesion (LCOM4) | Mean across modules | Average |
| Avg distance from main sequence | Mean across modules | Average |
| Temporal coupling | `hidden_pairs / total_pairs` | Proportion |
| Tangle index | `edges_in_cycles / total_structural_edges` | Proportion |

### Size-Biased (Needs Attention)

| Metric | Current Formula | Size Problem |
|---|---|---|
| **Max call depth** | `fallback_depth = 100 × (1 - max_depth/30)` | Absolute threshold of 30. A 50K-function enterprise system WILL have deeper call chains than a 500-function library — not because of bad design, but because there's more functionality layered together. Depth 15 is excellent for an enterprise system but penalized the same as depth 15 in a tiny library. |
| **Hotspot detection threshold** | Top percentile of fan-in | The top-percentile approach IS relative, so the count is size-independent. However, the absolute fan-in values reported in findings will be higher for larger repos, which could mislead users comparing across repos. |
| **Blast radius** (absolute count) | Transitive reverse BFS | Blast radius of 200 in a 50K-function repo is proportionally small. Blast radius of 200 in a 500-function repo means 40% of the codebase. The raw number is misleading without context. |
| **Finding count** | Count of findings per type | More modules → more coupling findings. More functions → more complexity findings. The raw count makes large repos look worse. |

### Potentially Biased (Subtle)

| Metric | Concern |
|---|---|
| **Module coupling average** | More modules → more cross-module edges → potentially higher average coupling. But the per-module ratio formula (external/total) should normalize this. The concern is that small modules (3-5 functions) have less opportunity for internal edges, pushing their coupling toward 1.0 naturally. The `min_module_size = 3` filter helps but may not be enough. |
| **Cycle detection** | More nodes → more opportunities for cycles. The ratio normalizes the score, but the finding count does not. A 50K-node graph might have 20 cycle findings vs 2 for a 500-node graph, making the larger repo look more problematic even if its cycle ratio is lower. |
| **Dead code detection** | Larger repos with multiple entry points and framework hooks have more functions that appear dead due to indirect invocation patterns. The exemption heuristics help but larger repos have more edge cases. |

---

## Recommended Fixes

### Fix 1: Size-Tiered Depth Normalization

**Problem:** `max_depth / 30` penalizes large repos.

**Solution:** Define size tiers and adjust the depth ceiling per tier.

```rust
fn size_adjusted_depth_ceiling(total_functions: usize) -> usize {
    match total_functions {
        0..=200 => 15,        // Small library
        201..=1000 => 20,     // Medium project
        1001..=5000 => 25,    // Large application
        _ => 35,              // Enterprise monolith
    }
}

fn fallback_depth(max_call_depth: usize, total_functions: usize) -> u32 {
    let ceiling = size_adjusted_depth_ceiling(total_functions);
    let normalized = (max_call_depth as f64 / ceiling as f64).clamp(0.0, 1.0);
    (100.0 * (1.0 - normalized)).round().max(0.0) as u32
}
```

The ceilings are based on the observation that well-designed systems scale depth approximately as `log(N)` — a 10x larger codebase adds roughly 1 layer of depth, not 10x.

**Where:** `health_score.rs` line 469–472.

### Fix 2: Relative Blast Radius

**Problem:** Blast radius of 200 means different things at different scales.

**Solution:** Report blast radius as both absolute count AND as a percentage of total functions.

```rust
pub struct BlastRadiusAnnotation {
    pub absolute: usize,
    pub relative: f64,  // absolute / total_functions
}
```

In findings and priority scoring, use relative blast radius:

```rust
let blast_radius_factor = 1.0 + (relative_blast_radius * 10.0).ln_1p();
```

**Where:** `report.rs` (add `blast_radius_pct` to NodeAnnotation), `priority.rs` (use relative in formula).

### Fix 3: Normalized Finding Counts

**Problem:** More code → more findings → repo looks worse.

**Solution:** Report findings per 1,000 functions (finding density), not raw count.

```rust
pub finding_density: f64,  // findings.len() / (total_functions / 1000.0)
```

In the summary card, show: "4.2 findings per 1K functions" instead of "84 findings."

**Where:** `report.rs` (add `finding_density` to Summary), frontend (display in summary card).

### Fix 4: Size-Segmented Percentiles

**Problem:** When percentile scoring compares against the population, a 50K-function enterprise system is ranked against 500-function libraries. The enterprise system will naturally have higher absolute depth, more cycles, etc.

**Solution:** Segment the population by size tier when computing percentiles.

```rust
pub enum SizeTier { Small, Medium, Large, XLarge }

fn size_tier(func_count: usize) -> SizeTier {
    match func_count {
        0..=200 => SizeTier::Small,
        201..=1000 => SizeTier::Medium,
        1001..=5000 => SizeTier::Large,
        _ => SizeTier::XLarge,
    }
}
```

When computing percentiles:
1. Filter population to same size tier
2. If tier has < 20 projects, fall back to global percentile but note "compared against all projects (insufficient same-size population)"
3. Report both global and tier percentile when possible

**Where:** `health_score.rs` (modify `build_percentiles`), `norms.rs` (add func_count to PopulationRow), `report.rs` (add `size_tier` to PercentilesReport).

### Fix 5: Module Size Guard for Coupling

**Problem:** Modules with 3-5 functions have few internal edges, pushing coupling toward 1.0 even when the code is well-structured.

**Current mitigation:** `min_module_size = 3` filter.

**Additional fix:** Weight module coupling by module size when computing the average. Large modules' coupling scores count more than tiny modules'.

```rust
let weighted_avg = modules.iter()
    .map(|(_, m)| m.coupling_score * m.total_nodes as f64)
    .sum::<f64>()
    / modules.iter()
    .map(|(_, m)| m.total_nodes as f64)
    .sum::<f64>();
```

This prevents a single 3-function module with coupling 1.0 from dragging the average up.

**Where:** `coupling.rs` (add weighted average alongside current simple average).

---

## How This Affects the Health Score

The health score formula itself doesn't need to change. Most of its inputs are already ratios/averages. The fixes above correct the inputs, not the formula:

| Fix | What changes |
|---|---|
| Size-tiered depth | The depth sub-score input becomes relative to repo size |
| Relative blast radius | Priority formula uses relative, not absolute |
| Normalized finding count | Presentation only — doesn't affect score |
| Size-segmented percentiles | Percentiles become more meaningful, score derived from them |
| Weighted coupling average | The coupling sub-score input becomes less noisy for small modules |

---

## Other Concerns Not Yet Raised

### Language-Specific Structural Patterns

Different languages have inherently different structural profiles:

| Language | Natural pattern | Effect on metrics |
|---|---|---|
| **Java** | Type-heavy, deep class hierarchies, many small classes | High module count, high type edges → potentially higher coupling. The Java coupling baseline (0.65) partially addresses this. |
| **Go** | Flat package structure, minimal inheritance, explicit interfaces | Naturally lower coupling, shallower depth. Go repos may score artificially well. |
| **Rust** | Trait-heavy, module tree with `use` re-exports | High coupling between small modules using shared domain types. Rust baseline (0.75) addresses this. |
| **TypeScript/JS** | Import-heavy, barrel files re-exporting everything | Barrel files create artificial hub nodes with extreme fan-out. |
| **Python** | Dynamic dispatch, `__init__.py` package pattern | Many calls unresolvable statically → confidence impacts scores. |

**Current mitigation:** Ecosystem-aware coupling baselines in `ThresholdConfig::for_ecosystem()`.

**What's missing:** Ecosystem-specific depth ceilings, ecosystem-specific hotspot thresholds. The size-tiered depth fix above should also be ecosystem-aware:

```rust
fn size_adjusted_depth_ceiling(total_functions: usize, ecosystem: Ecosystem) -> usize {
    let base = match total_functions {
        0..=200 => 15,
        201..=1000 => 20,
        1001..=5000 => 25,
        _ => 35,
    };
    match ecosystem {
        Ecosystem::Java | Ecosystem::CSharp => base + 5,  // deeper hierarchies are normal
        Ecosystem::Go => base - 3,                         // flat structure expected
        _ => base,
    }
}
```

### Monorepo vs Multi-Repo

A monorepo containing 5 services analyzed as one project will have:
- Higher coupling (cross-service imports counted as cross-module)
- More cycle opportunities
- Deeper call chains
- More findings

This is a real structural concern (services shouldn't be tightly coupled) but the absolute numbers can be misleading compared to analyzing each service separately.

**Future consideration:** A `--workspace-roots` flag that defines service boundaries within a monorepo, so coupling is measured within and between services explicitly. Not needed for MVP.

### Test Code Ratio Effects

Repos with extensive test suites that are NOT excluded via `--exclude-tests` will have:
- Higher dead code (test helpers often have no incoming edges)
- Different coupling patterns (test files import production modules heavily)

**Current mitigation:** `--exclude-tests` flag exists. When enabled, test code is filtered before analysis.

**Gap:** Users who don't use the flag (including the web product, which doesn't expose it) get noisy results. The web product should default to `exclude_tests = true` in the API gateway's analysis call.

---

## Where These Fixes Live in the Sprint Plan

| Fix | Sprint | Ticket | Effort |
|---|---|---|---|
| Size-tiered depth normalization | Sprint 1 | T4 | Part of T4 (1 day total) |
| Relative blast radius | Sprint 1 | T4 | Part of T4 |
| Normalized finding count | Sprint 1 | T4 | Part of T4 |
| Size-segmented percentiles | Sprint 2 | T5 | Part of T5 (add func_count to population table) |
| Weighted coupling average | Sprint 1 | T4 | Part of T4 |
| Ecosystem-specific depth ceilings | Sprint 1 | T4 | Part of T4 |
| Default exclude_tests in web product | Sprint 2 | T5 | One-line change in API analyze route |

---

## The Principle

Every metric should answer: "Is this code well-structured **for its context**?" — not "Is this code small?"

The user should be able to compare:
- A 50K-function Java enterprise monolith
- A 500-function Go microservice
- A 2K-function TypeScript web app

And trust that all three are scored fairly relative to what "healthy" means for their size and ecosystem.

Percentile-based scoring handles this naturally IF the population is segmented by size tier and ecosystem. The fallback formulas need explicit normalization because they don't have a population to compare against.
