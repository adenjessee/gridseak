# P3 — Test-gap closure on shipped T1/T3/T4 work

> Authored against [docs/workstreams/universal-fidelity/tasks/TEMPLATE.md](TEMPLATE.md).

## Retrospective header

**What shipped.** T1 (EdgeKind namespacing + `Framework`/`Declarative` families + hand-rolled stable-str + `CallSite.edge_kind_hint`), T2 (content-based stable IDs), T3 (dual-metric emission with `FidelityGap` on eight Layer-3 metrics), T4 (`MeasuredFidelityTier` on `ResolutionQuality`). All four landed without per-task design docs.

**What the post-ship review found missing.** Regression coverage is strong at the unit level (`measured_fidelity_tests` in [graphengine-analysis/src/health/report.rs](../../../../graphengine-analysis/src/health/report.rs)) but thin at the integration level:

- Only two of the eight `FidelityGap`-carrying metrics are asserted non-zero on the divergent fixture ([t3f_fidelity_gap_regression.rs](../../../../graphengine-analysis/tests/t3f_fidelity_gap_regression.rs)).
- No test proves the all-High case yields a zero gap — a "fidelity is always non-zero" bug would read as success.
- The `MeasuredFidelityTier` thresholds (40 % / 80 %) have unit coverage against a hand-built `EdgesByConfidence`, but no end-to-end fixture locks the boundary direction against a real `parse.db → run_analysis → tier` round-trip.
- The `EdgeKind` hand-rolled `to_stable_str` / `from_stable_str` wire format is not pinned against accidental change. The unit round-trip test proves round-trip, not stability.

**Rework ticket that owns the correction.** This task (P3). Lives before [P1-T1-rework.md](T1-rework.md) because the rework must not lose regression signal mid-change.

---

## 1. Problem statement

Four distinct behavioural invariants shipped with T1/T3/T4 have no integration-level test:

1. **Per-metric fidelity plumbing.** `MetricsReport.{coupling, cohesion, hotspot_concentration, depth, tangle_index, distance_from_main_sequence}.fidelity` could silently regress to `None`, or the swap-guard logic in `compute_health` could accidentally compute the high-only path against the full edge set, and the sole behavioural test ([t3f_fidelity_gap_regression.rs](../../../../graphengine-analysis/tests/t3f_fidelity_gap_regression.rs)) would still pass. It only checks `cycles` and `dead_code`.
2. **Zero-gap invariant.** When every edge carries `Confidence::High`, every metric's `absolute_gap` must be exactly `0.0`. A "divergent even on uniform graphs" bug reads as success on the current fixture because the fixture is constructed to be divergent.
3. **Boundary direction on `MeasuredFidelityTier`.** `from_call_edges` uses `r >= 0.80` (inclusive) and `r >= 0.40` (inclusive). Changing these to `>` without a test update would still pass the hand-built unit suite but would flip the classification of any canary sitting exactly on the boundary. The canary-level prediction from D1 (NPSP rev 9 → `SyntacticOnly`) is not end-to-end asserted.
4. **Pre-rework wire format.** P1.b is about to replace `to_stable_str` with serde-tagged serialization. Without a pinned-string test of the current wire format, the migration can change the on-disk representation silently and existing parse DBs become unreadable on a rollback.

## 2. Non-goals

- **Not rewriting T1/T3/T4.** This task adds tests only. Code changes to the rework targets live in [P1-T1-rework.md](T1-rework.md).
- **Not adding percentile or norms coverage.** Percentiles are loaded from a separate population database; they are not part of the T3/T4 plumbing P1 is about to rework.
- **Not validating dead-code classifier per-reason accuracy.** The existing classifier unit tests cover that; this task focuses on `FidelityGap` plumbing only.
- **Not measuring Layer-2 adapter behaviour.** T6 owns Rust Layer 2 fixtures; they are unrelated to the shipped T3/T4 plumbing under test here.

## 3. Five diagnostic questions

1. **Are we forcing one type to do two jobs?** No. This task adds tests against types that already exist.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** Partially — the new fixtures hardcode canary shapes. Mitigation: the fixtures are named after their invariants (e.g., `boundary_41_pct_is_heuristic_primary`), not after canaries. If the thresholds change, the fixture names stop matching their assertions, which is the intended "brittle" failure mode: it forces the author of the threshold change to look here.
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** No. Tests read the end-state `HealthReport`; no stage-crossing involved.
4. **Is the trade-off about serialization format?** Yes — invariant 4 is explicitly about serialization format stability. Dissolving element: pin the wire string **literally** in the test, not via round-trip assertion alone. See §4.
5. **Is the trade-off between two modes of failure?** Yes — invariant 3 asks "is the off-by-one at the boundary a silent classification flip, or a loud test failure?". Dissolving element: four boundary fixtures at 39 % / 41 % / 79 % / 81 % prove direction, not just endpoints.

## 4. Chosen shape

Four test files, each pinning one behavioural invariant. Each file is behavioural: it asserts invariants, not implementation shape, so P1's rework can delete / restructure any intermediate plumbing as long as the final `HealthReport` still satisfies the named invariants.

### 4.1 `graphengine-analysis/tests/t3f_fidelity_gap_regression.rs` (extend)

Extend the existing divergent-fixture test to assert `Some(fidelity)` and `absolute_gap.abs() > 0.0` on every metric that carries `fidelity: Option<FidelityGap>`:

- `metrics.cycles` (already covered)
- `metrics.dead_code` (already covered)
- `metrics.coupling`
- `metrics.cohesion` (wrapped in `Option<CohesionMetricDetail>`; test asserts `Some` then drills in)
- `metrics.hotspot_concentration`
- `metrics.depth`
- `metrics.tangle_index`
- `metrics.distance_from_main_sequence` (wrapped in `Option`; test tolerates `None` only when the fixture is single-module, which this one is not)

The fixture builder stays unchanged.

### 4.2 `graphengine-analysis/tests/t3_uniform_high_zero_gap.rs` (new)

Build a DB where every edge's `provenance.confidence` is `"High"`. Run `run_analysis`. Assert:

- Every metric that carries `fidelity: Option<FidelityGap>` has `fidelity.absolute_gap == 0.0` (exact equality — `FidelityGap::from_values` computes the difference, so when both inputs are equal the result is exactly zero, not a float-epsilon-near-zero).
- `fidelity.all_edges_count == fidelity.high_only_edges_count`.
- `fidelity.relative_gap == Some(0.0)` when `all_edges_value` is non-zero; `None` otherwise.

### 4.3 `graphengine-analysis/tests/t4_canary_tier_classification.rs` (new)

Six named fixtures, each constructing a `parse.db` with a controlled ratio of High call-like edges, running `run_analysis`, and asserting the resulting `resolution_quality.measured_fidelity.tier`.

| Fixture name | High | Medium + Low | Expected tier | Invariant locked |
| --- | --- | --- | --- | --- |
| `npsp_rev9_shape_is_syntactic_only` | 1.1 % | 98.9 % | `SyntacticOnly` | D1 canary prediction for NPSP |
| `hypothetical_85pct_high_is_authoritative` | 85 % | 15 % | `Authoritative` | D1 target for a post-T6 Rust scan |
| `boundary_39_pct_is_syntactic_only` | 39 % | 61 % | `SyntacticOnly` | 40 % is inclusive on the HeuristicPrimary side |
| `boundary_41_pct_is_heuristic_primary` | 41 % | 59 % | `HeuristicPrimary` | symmetric |
| `boundary_79_pct_is_heuristic_primary` | 79 % | 21 % | `HeuristicPrimary` | 80 % is inclusive on the Authoritative side |
| `boundary_81_pct_is_authoritative` | 81 % | 19 % | `Authoritative` | symmetric |

The fixtures use `Call` edges only (no `Framework` / `Declarative`) so the denominator in `high_ratio_on_calls` is predictable. The builder uses a common helper: given `n_high, n_low`, produce that many function nodes and one call edge each with the designated confidence, to a shared sink function, plus enough file/folder scaffolding for `AnalysisGraph` loading to succeed.

### 4.4 `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` (new, pre-rework phase)

Phase 1 (this task): pin the **current** hand-rolled wire format with literal-string assertions per variant.

```rust
// Current pre-rework format: hand-rolled `to_stable_str`.
assert_eq!(EdgeKind::Call.to_stable_str(), "Call");
assert_eq!(EdgeKind::Contains.to_stable_str(), "Contains");
assert_eq!(
    EdgeKind::Framework(FrameworkKind::VisualforcePage).to_stable_str(),
    "Framework:VisualforcePage"
);
assert_eq!(
    EdgeKind::Declarative(DeclarativeKind::Flow).to_stable_str(),
    "Declarative:Flow"
);
// ... every variant.
```

Phase 2 (updated in P1.b, not this task): replace the pinned strings with the serde-tagged format (e.g., `r#"{"kind":"Framework","sub":"VisualforcePage"}"#`). The P1.b commit that flips the wire format MUST also flip every pinned string in this file, so the diff is grep-able audit-trail evidence that someone reviewed the format change.

## 5. Acceptance criteria

- `cargo test --test t3f_fidelity_gap_regression` passes with assertions on all eight fidelity-carrying metrics.
- `cargo test --test t3_uniform_high_zero_gap` passes a new fixture with `absolute_gap == 0.0` on every metric.
- `cargo test --test t4_canary_tier_classification` passes all six fixtures.
- `cargo test --test t1_edgekind_roundtrip` (in `graphengine-parsing`) passes literal-string assertions on every currently-shipping variant.
- No existing test is edited to pass these. If any existing test needs to change, that signals the invariant being locked was already broken and the test was hiding it.

## 6. Test plan

This task IS the test plan. All four deliverables are test files. Test strategy for each:

| File | Style | Isolation |
| --- | --- | --- |
| `t3f_fidelity_gap_regression.rs` (extended) | integration; synthesized `parse.db`; full `run_analysis` | temp-dir DB per test, cleaned up on success |
| `t3_uniform_high_zero_gap.rs` | integration; synthesized `parse.db`; full `run_analysis` | temp-dir DB per test, cleaned up on success |
| `t4_canary_tier_classification.rs` | integration; six synthesized DBs; full `run_analysis` | temp-dir DB per fixture function, cleaned up on success |
| `t1_edgekind_roundtrip.rs` | unit-style integration in a separate test file; no DB | no shared state |

A shared fixture helper is acceptable **only** in the T4 canary file, where all six fixtures use an identical `nodes_and_edges_with_ratio` builder. The divergent and uniform T3 fixtures do not share a helper — their fixture shapes are too different to benefit from sharing.

## 7. Rollback criterion

A single named signal: if `cargo test --workspace` has new test failures after P3 lands **and** the failure is in code not touched by P3 (i.e., pre-existing behaviour broke), revert. P3 is test-only; no production code changes.

There is no expected behaviour change to any existing test from P3's landing. If one appears, it is a Heisenbug the new tests exposed, and the correct response is to file a bug ticket against the exposed behaviour before deciding whether to keep P3 merged.

## 8. Out-of-scope follow-ups

- **Per-language classifier fixtures.** D1 promises tier predictions for commons-lang (Java), django-site (Python), nextjs-commerce (TS), serilog (C#). We only lock NPSP here. Follow-up ticket: extend `t4_canary_tier_classification.rs` with one fixture per D1 canary once the Rust Layer 2 adapter (T6) lands, because the TS / Rust post-Layer-2 prediction depends on the adapter. Not blocking P1.
- **Edge-count invariant on mixed fixtures.** A fixture with `n_high + n_low` call edges should produce `fidelity.all_edges_count` in every metric equal to the structural-edge-set size, and `high_only_edges_count == n_high`. Asserting this tightens the contract. Follow-up ticket: add to the T4 canary file once the six baseline fixtures are green.
- **Serde snapshot test after P1.b.** Once P1.b replaces `to_stable_str`, the literal-string assertions in `t1_edgekind_roundtrip.rs` become the serde wire-format contract. No separate snapshot tool needed; pinning in a test file is the contract. Follow-up only if the variant list grows substantially and the file becomes unmanageable.
