# T8 — classifier extraction-coverage awareness

Authored against [`TEMPLATE.md`](TEMPLATE.md). Every section is answered, no skipped headings.

> **Gate.** T8 is gated on **discovery question D2** in `docs/workstreams/universal-fidelity/DISCOVERY_REPORT.md`: "What is the per-file extraction-coverage shape across the NPSP rev 9 corpus, and which gap patterns account for ≥ 80 % of false positives in `dead_code.no_callers`?" Do not start implementation until D2 has a signed answer. D2 is tracked as a measurement task, not a design task — the T8 design below assumes the D2 result names at least R39 (property accessors) and R41 (map-literal field initializers) as dominant gap patterns, consistent with the Round 5 audit. If D2 surfaces a different dominant pattern, §4 and §5 of this doc must be revisited before implementation starts.

---

## 1. Problem statement

The Round 5 audit on NPSP `rev 9` drew 10 random `no_callers` samples, hand-verified each one, and found 4 were wrong — 40 % false-positive rate on a flagship metric. Three of the four failures traced to extractor gaps (not classifier bugs, not resolver bugs):

- **R39** (2 failed) — property accessor bodies (`get { ... }` / `set { ... }`) are not walked by the Apex extractor, so calls from inside them are invisible to the graph. The enclosing method looks like it has `fan_in = 0` even though the real codebase contains the call.
- **R41** (1 failed) — map-literal field initializers (`Map<Id, Foo> m = new Map<Id, Foo>{ ... }`) are not walked. Identical symptom: calls inside the literal are invisible.
- **R45** (1 failed) — chained call on the return value of another call (`obj.first().second()`) is a resolver gap, not an extraction gap. Out of scope for T8.

226 `.cls` files across the NPSP corpus match R39's shape; 45 match R41's shape. If even 10 % of those patterns contain real live calls we are currently invisible to, that is a systematic false-positive floor we cannot drop by tuning the classifier in place — because the classifier **does not know** these files have unwalked regions.

Fixing the extractors to walk these regions is the structurally correct answer. It is also **Phase B work** explicitly deferred from this sprint (§6 of the primer). T8's job is the honest workaround: attach a per-file signal that quantifies how much of the file's AST the extractor actually walked, and let downstream classifiers decide what to do with a file whose coverage is low.

Concrete consequences this task exists to fix:

- `dead_code.no_callers` ships a single `High`-confidence classification on every file regardless of whether the file contains 0 or 226 unwalked property accessors. The signal lies.
- There is no caveat in the health report naming extraction coverage; a customer reading the report cannot tell they are looking at an R39-shape false positive.
- T4 (measured fidelity tier) rolls up at the *scan* level, but the T8 gap is *per file*. Even on a scan that classifies `HeuristicPrimary`, individual files may be `SyntacticOnly` or worse in practice.

## 2. Non-goals

- **Fixing the extractors.** R39 / R41 walker implementations are Phase B. T8 is explicitly the workaround while that work is deferred.
- **Inventing a new classifier.** T8 is a *signal* — it modifies the confidence or filter behaviour of existing classifiers. No new headline metric is introduced.
- **Detecting extraction coverage for non-Apex languages.** Rust, Java, TypeScript, Go have their own coverage shapes, but the Round 5 audit evidence only exists for Apex. D2 is Apex-specific. Extending to other languages is tracked as **T8.b** and gated on a per-language D2-equivalent.
- **Attempting to re-walk unwalked regions to rescue the signal.** A light fallback walk would re-introduce all the complexity T8 is trying to avoid. We count the unwalked regions; we do not parse them.
- **Silencing R39/R41 candidates entirely from the headline metric.** The user-visible metric still reports the candidates; T8 just annotates them with a `Medium` (not `High`) confidence and a grep-able caveat naming the gap.

## 3. Five diagnostic questions

1. **Are we forcing one type to do two jobs?** **Yes.** Today `ExtractionStats` (or equivalent) conflates "what tree-sitter nodes we saw" with "what we extracted from them." T8 separates them. The new type `FileExtractionCoverage` records the gap — nodes present in the AST but not walked by any extractor query — as a first-class signal.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** **Yes.** The naive design hardcodes "if R39 or R41 present, downgrade no_callers" in every classifier. The dissolving element is a `CoverageGap` enum where each variant carries the classifier-relevant predicates, and classifiers query predicates rather than variant lists (same discipline as P1.a).
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** **Yes.** The extractor knows which AST node kinds it does not walk; the classifier knows which metrics rely on exhaustive walking. The dissolving element is a typed channel `FileExtractionCoverage` propagated from extraction to classification — not a stringly-typed "R39" boolean on every file row.
4. **Is the trade-off about serialization format?** **No.** The signal is persisted alongside existing per-file rows in the analysis SQLite using the existing `serde` pattern.
5. **Is the trade-off between two modes of failure?** **Yes.** The false choice is "filter R39 candidates out of the metric" vs "keep them in." Both are wrong in isolation: filtering hides real bugs, keeping hides extractor gaps. The dissolving element is to **emit both** — the headline metric stays unchanged for backward compat, but a companion `dead_code.no_callers_high_confidence` field filters candidates in files with R39/R41 gaps, and the health report exposes the delta.

Interpretation: **four of five questions are `Yes`**. §4 supplies all four dissolving elements.

## 4. Chosen shape

### 4.1 Types introduced

```rust
// graphengine-parsing/src/application/ports.rs (co-located with SyntaxResults)
pub struct FileExtractionCoverage {
    pub file_path: PathBuf,
    pub language: String,
    pub walked_node_count: u32,
    pub unwalked_node_count: u32,          // AST nodes the extractor's query set did not match
    pub coverage_gaps: Vec<CoverageGap>,
    pub confidence: Confidence,            // High when we trust the count; Low on parse errors
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "sub")]
pub enum CoverageGap {
    ApexPropertyAccessor { count: u32 },       // R39
    ApexMapLiteralInitializer { count: u32 },  // R41
    ApexTriggerBodyUncaptured { count: u32 },  // R40 — declared but not yet emitted by the extractor (Phase-B dependency; the trigger-body walker hasn't been wired yet). Keeping the variant declared so downstream consumers can pattern-match exhaustively today; count will remain zero until Phase B ships.
    // Further variants added as D2-equivalent rounds surface new patterns.
}

impl CoverageGap {
    /// Does this gap invalidate `dead_code.no_callers` for the file?
    pub fn invalidates_no_callers(&self) -> bool {
        matches!(
            self,
            CoverageGap::ApexPropertyAccessor { .. } | CoverageGap::ApexMapLiteralInitializer { .. }
        )
    }

    /// Does this gap invalidate `cycles`? (no — cycles only use edges that exist)
    pub fn invalidates_cycles(&self) -> bool {
        false
    }
}
```

### 4.2 Extractor instrumentation

`graphengine-parsing/src/syntax/language/apex/extractor.rs` (and eventually each language) adds a post-extract pass that walks the AST once and counts nodes of kinds known to the current query set vs kinds not matched. The Apex implementation counts `property_accessor`, `map_literal_field`, and `trigger_body` nodes specifically; unmatched kinds go into `unwalked_node_count` but only the three named kinds emit specific `CoverageGap` variants.

The coverage record is appended to `SyntaxResults`:

```rust
pub struct SyntaxResults {
    // … existing …
    pub extraction_coverage: Vec<FileExtractionCoverage>,   // one per parsed file
}
```

The orchestrator persists the vector alongside other per-file signals to a new `file_extraction_coverage` SQLite table.

### 4.3 Classifier integration

`graphengine-analysis/src/health/dead_code_classifier/mod.rs` gains a coverage-aware pass:

```rust
pub struct DeadCodeConfig {
    pub filter_low_coverage_files_from_headline: bool,   // default true post-T8
    pub downgrade_confidence_on_coverage_gap: bool,      // default true
}

impl DeadCodeClassifier {
    pub fn classify(
        &self,
        graph: &AnalysisGraph,
        coverage: &HashMap<PathBuf, FileExtractionCoverage>,
    ) -> DeadCodeClassification {
        for candidate in raw_candidates {
            if let Some(cov) = coverage.get(&candidate.file_path) {
                if cov.coverage_gaps.iter().any(CoverageGap::invalidates_no_callers) {
                    candidate.confidence = Confidence::Medium;  // from High
                    candidate.caveats.push("extraction_coverage_r39_r41");
                }
            }
        }
        // Headline filters out Medium-and-below; companion metric keeps them.
    }
}
```

The dual-metric emission discipline from T3 applies: `no_callers_total` is every candidate; `no_callers_high_confidence` is candidates in files with no invalidating coverage gaps. The fidelity gap between the two is the new T8 signal.

### Compile-time guarantees

- `CoverageGap` is a closed enum; adding a variant without extending the predicate set is a review signal.
- Every predicate is named per-metric (`invalidates_no_callers`, `invalidates_cycles`, …); adding a new metric that should be gap-aware requires adding a predicate, not a variant check.
- `FileExtractionCoverage.confidence` is mandatory; a coverage record cannot exist without a declared confidence level.

### Predicate contracts

New predicates on `CoverageGap`:
- `invalidates_no_callers() -> bool`
- `invalidates_cycles() -> bool`
- `invalidates_fan_in_metrics() -> bool`
- `invalidates_coupling() -> bool`

Every metric consumer in `graphengine-analysis/src/health/` queries these predicates; none pattern-match `CoverageGap` variants directly.

## 5. Acceptance criteria

1. `cargo test --workspace` green with the new `FileExtractionCoverage` struct + the new `file_extraction_coverage` SQLite table.
2. Synthetic Apex fixture `graphengine-parsing/tests/fixtures/apex_r39_property_accessor/` containing a `.cls` file with one `get { }` body emits `FileExtractionCoverage { coverage_gaps: [CoverageGap::ApexPropertyAccessor { count: 1 }] }`. Test: `graphengine-parsing/tests/t8_coverage_r39.rs → r39_property_accessor_emits_gap`.
3. Companion fixture for R41: `graphengine-parsing/tests/fixtures/apex_r41_map_literal/` emits `CoverageGap::ApexMapLiteralInitializer { count: 1 }`. Test: `t8_coverage_r41.rs → r41_map_literal_emits_gap`.
4. `DeadCodeClassifier` regression: `graphengine-analysis/tests/t8_dead_code_respects_coverage.rs` creates a graph with a candidate in a file whose coverage record contains `ApexPropertyAccessor`; asserts classification confidence is `Medium`, not `High`, and `"extraction_coverage_r39_r41"` appears in caveats.
5. Companion metric fixture: `graphengine-analysis/tests/t8_dual_metric_no_callers.rs` verifies `no_callers_total > no_callers_high_confidence` on a graph where at least one candidate lives in a coverage-gap file. Named assertion: `high_confidence_filters_low_coverage_candidates`.
6. NPSP `rev 9` canary scan emits at minimum 226 `CoverageGap::ApexPropertyAccessor` records (one per file matching the R39 shape) and 45 `CoverageGap::ApexMapLiteralInitializer` records. Verified by `graphengine-diagnostic/tests/npsp_coverage_gaps_r39_r41.rs`.
7. Health report: NPSP canary emits a non-empty `file_extraction_coverage` section and the top-level `integrity_caveats` list includes `CAVEAT_EXTRACTION_COVERAGE_GAPS_V1` when any file has an invalidating gap.
8. `rg -n 'matches!\s*\(\s*.*CoverageGap::' graphengine-analysis/src/` returns zero matches outside predicate definitions. Classifiers consume predicates, not variants.
9. Baseline fidelity-gap measurement: on NPSP rev 9 before T8, `dead_code.no_callers` false-positive rate is 40 % (Round 5 audit). After T8, the `no_callers_high_confidence` companion metric must drop the false-positive rate on the same 10-sample hand-verified set to ≤ 10 %. This is the primary quantitative evidence T8 works. Measured via `docs/workstreams/universal-fidelity/verification/t8_fp_rate.md` retrospective doc.

## 6. Test plan

### 6.1 Unit tests

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `coverage_gap_predicates_exhaustive` | `graphengine-parsing/src/application/ports.rs #[cfg(test)]` | Every `CoverageGap` variant has a non-panicking answer for every `invalidates_*` predicate. |
| `property_accessor_count_is_exact` | `graphengine-parsing/src/syntax/language/apex/coverage.rs #[cfg(test)]` | AST with 3 `property_accessor` nodes yields `count: 3`. |
| `map_literal_count_is_exact` | same file | AST with 2 `map_literal_field` initializer nodes yields `count: 2`. |
| `confidence_low_on_partial_parse` | same file | Fixture with a tree-sitter parse error → `FileExtractionCoverage.confidence == Low`. Locks the guard. |

### 6.2 Integration tests

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `r39_property_accessor_emits_gap` | `graphengine-parsing/tests/t8_coverage_r39.rs` | See criterion #2. |
| `r41_map_literal_emits_gap` | `graphengine-parsing/tests/t8_coverage_r41.rs` | See criterion #3. |
| `dead_code_respects_coverage_gap` | `graphengine-analysis/tests/t8_dead_code_respects_coverage.rs` | See criterion #4. |
| `high_confidence_filters_low_coverage_candidates` | `graphengine-analysis/tests/t8_dual_metric_no_callers.rs` | See criterion #5. |
| `npsp_coverage_gaps_match_audit_shape` | `graphengine-diagnostic/tests/npsp_coverage_gaps_r39_r41.rs` | See criterion #6. |
| `integrity_caveats_name_coverage_gap_presence` | `graphengine-analysis/tests/t8_caveat_emission.rs` | Health report integrity caveats include `CAVEAT_EXTRACTION_COVERAGE_GAPS_V1` when any file has an invalidating gap. |

### 6.3 Regression fixture

NPSP rev 9 is the primary canary: 226 + 45 = 271 coverage records must appear on that scan. A T8 regression that drops those (e.g., extractor instrumentation stopped counting, or wire format drifted) is immediately visible.

The retrospective measurement at `docs/workstreams/universal-fidelity/verification/t8_fp_rate.md` is the secondary regression check: a new 10-sample hand audit against `no_callers_high_confidence`, re-run each time T8 is modified, must stay ≤ 10 % false-positive rate.

## 7. Rollback criterion

**Single named signal:** the NPSP 10-sample hand audit against `no_callers_high_confidence` regresses above 15 % false-positive rate.

This is the only quantitative signal that directly validates T8's thesis. Any other regression — test failures, SQLite shape changes — is a pre-merge block. The hand-audit retrospective is the only metric that can drift *after* merge and the only one worth reverting for.

## 8. Out-of-scope follow-ups

- **Fix R39 / R41 extractors.** Actual Phase B work. After T8 ships, these fixes *reduce* the gap-count to 0 and the T8 downgrade stops firing for those files. No T8 rework needed; the signal self-retires. Proposed: **Phase-B-R39**, **Phase-B-R41** tickets.
- **R45 chained call resolution.** Resolver work, not extractor. Separate task. Proposed: **Phase-B-R45**.
- **T8.b — non-Apex coverage signals.** Java enum value bodies, Rust macro expansions, TypeScript JSX prop-bag walks, etc. each needs its own D2-equivalent audit. Deferred. Gated on measured false-positive evidence per language.
- **T8.c — coverage-aware fan-in metric.** Fan-in is a more sensitive victim of coverage gaps than `no_callers`. Deferred because `no_callers` is the flagship metric; fan-in can be retrofitted using the same `FileExtractionCoverage` wire.
- **Customer-visible UI surface for coverage gaps.** The health-report caveat makes them queryable; a dedicated UI pane would help triage but is product work, not engine work. Proposed: **desktop-shell T8 coverage panel**.

## 9. NPSP canary retrospective (2026-04-21)

Post-implementation measurement run, documented in
`experiments/results/gate3-t8-npsp-canary/`. Goals of the canary
(per §5 acceptance criterion #7): prove the pipeline runs
end-to-end on a real Apex workload, quantify how many `no_callers`
verdicts T8 pulls off `High`, and confirm no silent metric drift.

### 9.1 Measurement table

| axis                                      | value                           |
| ----------------------------------------- | ------------------------------- |
| Coverage rows persisted                   | **1,070** Apex files            |
| Files with at least one invalidating gap  | **318** (29.7%)                 |
| Files with unwalked property accessors    | **231** (R39)                   |
| Total R39 instances across repo           | **733**                         |
| Files with unwalked map-literal initialisers | **124** (R41)                |
| Total R41 instances across repo           | **357**                         |
| Total `no_callers` verdicts               | **567**                         |
| `no_callers` at `High` after T8           | **251** (44.3%)                 |
| Nodes downgraded by T8 (classifier or report pass) | **316** (55.7%)        |
| Git signals shape at analyse time         | `Shallow { depth: Some(1) }`    |
| Nodes downgraded by T7 churn rule         | **0** (shallow-clone guard)     |

### 9.2 Interpretation

The whole 316-node downgrade population is attributable to T8
alone — the NPSP local clone is shallow (depth 1), which flips
every file-signal confidence to `Low` and nullifies T7's churn
downgrade for the canary. This isolates the T8 contribution: of
567 `no_callers` dead-code verdicts the classifier would have
shipped pre-T8 at `High`, **55.7% are now honestly marked `Medium`**
because the file contains extraction-coverage shapes (R39 / R41)
that hide outgoing call edges the classifier cannot see.

The direction matches the D2 hand-audit hypothesis (see
`DISCOVERY_REPORT.md` §D2). The magnitude — more than half — is
larger than the median reviewer estimate, which is consistent
with the underlying cause: every Apex class with a VF controller
or service layer tends to have at least one `{ get; set; }`
property whose body could reference any of the class's methods.
The pre-T8 engine was systematically over-stating the precision
of its dead-code findings on this codebase.

### 9.3 What the numbers do not prove

- **Whether the 316 downgrades would have been false positives.**
  T8 is deliberately a precision-of-claim correction, not a
  ground-truth fix. A `Medium` verdict is still a plausible
  dead-code candidate; the downgrade just refuses to over-promise.
  The Phase-B-R39 / Phase-B-R41 tickets (§8) are what actually
  *retire* each gap by teaching the extractor to walk the body.
- **Whether the remaining 251 `High` verdicts are trustworthy.**
  A `High` confidence after T8 means: the file has no invalidating
  gap *that T8 currently knows about*. The R40 and R45-style gaps
  filed in `FOLLOWUPS.md` are not yet emitted as coverage gaps,
  so a call edge hidden inside one of them will still pass through
  T8 undiminished. T8.b / T8.c extend this net; today's coverage
  is Apex-only.
- **Whether the coverage counters counted what we intended.** The
  counters run at post-parse time against the same AST tree the
  extractor walked, so a tree-sitter regression on either side
  would drift the two readings in lock-step. The unit tests in
  `graphengine-parsing/src/syntax/language/apex/coverage.rs`
  anchor six fixture-level counts; the NPSP run anchors the
  aggregate shape (≥ 300 files with gaps on a real Apex workload).

### 9.4 Engineering follow-ups opened by this run

- **UF-FU-017** — investigate the two `file_extraction_coverage`
  records whose `confidence == Low` and `coverage_gaps == []`.
  These carry no actionable signal today but consume a SQLite row
  and a JSON field. Decide whether to drop them at serialise-time
  or tighten the coverage pass's `confidence` rule.
- **UF-FU-018** — baseline the Phase-B-R39 / Phase-B-R41 impact by
  re-running this canary after each extractor fix lands. Success
  criterion: the downgraded-node count drops toward 0 and the
  `no_callers_high_confidence` count rises, *not* the total. A
  run where `no_callers` itself drops indicates either a true
  extractor win (good) or a classifier regression (bad) — the
  dual metric lets us tell them apart without re-instrumenting.

### 9.5 Honest scope of this measurement

This is a single-repo post-implementation measurement, not a
statistical claim about Apex codebases in general. The sample
size is NPSP's ~1,070 Apex files. Cross-repo generalisation
requires the full T8.b per-language D2 audits named in §8. The
numbers here should be cited as "on NPSP, T8 demoted 56% of
`no_callers` verdicts from High to Medium confidence", never
as "T8 demotes ~56% of dead-code findings in practice".

