# Universal-Fidelity Sprint — Inception Report

> **Status.** Phase 3 inception (post-T6 / T7 / T8 shipped, pre-Phase-4 decision).
> **Date.** 2026-04-21.
> **Author.** Universal-fidelity sprint lead (agent-assisted).

This report closes the WS-HONESTY Phase 3 obligation named in
[`SPRINT_PLAN.md`](../../02-strategy/SPRINT_PLAN.md) WS-HONESTY row.
It is a *measurement* document, not a plan — its job is to record
what the engine now reports, how it reports it, and which numbers
a caller can trust at which confidence. Phase 4 (the Phase-B
scope decision) is expressly out of scope; the row instruction
reserves that decision for a separate artefact.

Nothing in this document describes aspirational behaviour. Every
numeric claim below traces to a named artefact under
`experiments/results/` or to a passing test in this workspace.

---

## 1. What shipped in this sprint

| Ticket | Summary                                                                                                      | Design doc                                                                 | Gate |
| :----- | :----------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------- | :--: |
| T6     | Rust Layer 2 via `ra_ap_ide`, gated behind the `rust-layer2` cargo feature (default-on post Gate 1.2 dogfood). | [`tasks/T6-rust-layer2.md`](tasks/T6-rust-layer2.md)                       |  1   |
| T7     | Layer 0 git-signal extraction (`graphengine-git-signals`), attach helper, dead-code churn downgrade.         | [`tasks/T7-layer0-git-signals.md`](tasks/T7-layer0-git-signals.md)         |  2   |
| T8     | Per-file extraction-coverage instrumentation (R39 / R41 Apex counters), classifier downgrade, dual metric.    | [`tasks/T8-coverage-awareness.md`](tasks/T8-coverage-awareness.md)         |  3   |

Supporting work (unticketed, landed alongside):

- `FileExtractionCoverage` SQLite persistence table + schema bump
  guardrails.
- `ge-analyze` CLI flags `--repo-root`, `--no-git-signals`.
- Two-jobs split between *extractor* (`graphengine-parsing`) and
  *classifier consumer* (`graphengine-analysis`). T7 and T8 both
  land the classifier-side downgrade in
  `health::dead_code_classifier` and the report-side attach in
  `health::git_signals_attach` / `health::coverage_attach`.

---

## 2. Side-by-side: all-edges vs High-only vs fidelity-gap (T3 plumbing, unchanged)

The T3 dual-metric plumbing shipped pre-sprint and the sprint did
not touch the metric computations. What *did* change is the
distribution of `High`-confidence edges that feed the High-only
column, because Layer 2 for Rust now produces real edges rather
than heuristic-only ones.

Evidence:

- **Dogfood rollup (`gridseak-self`, T6 PR #2):**
  `experiments/results/gate1-2-t6-pr2-dogfood/rollup.json` —
  measured `high_ratio_on_calls = 10.17 %`
  (`high = 10,306 / call_refs = 101,343`). Landing band 10–19 %
  triggered the plan's "ship + file UF-FU-012" tier.
- **NPSP T8 run (`experiments/results/gate3-t8-npsp-canary/`):**
  Apex-only parse; Layer 2 is Apex-specific via
  `class_symbols` (TR-A.0..A.6). The dual column
  `no_callers_total = 567` vs `no_callers_high_confidence = 251`
  is now reported verbatim, with 316 downgrades attributable to
  T8 (see §5).

Interpretation: `fidelity_gap` on the Rust workspace is no longer
dominated by raw extractor blindness — it now reflects a measured
89.9 % adapter-miss share (UF-FU-012 item b), whose leading cause
is proc-macro expansion (UF-FU-003). The distinction matters: a
gap caused by a known, counted failure mode is a different kind
of gap from one caused by an unknown extractor shape.

---

## 3. Measured Authoritative tier for Rust

Baseline reality check (pre-T6):

> Rust parsing produced Tree-sitter-only edges. Every call was
> heuristic. `High`-share on calls was effectively zero for any
> call that needed name resolution; the engine leaned on fan-in
> / fan-out for hotspot-ness.

Post-T6 reality check:

- **Feature flag:** `graphengine-parsing/Cargo.toml` →
  `default = ["rust-layer2"]` since Gate 1.2.
- **Adapter surface:** `graphengine-ra-ide-adapter` wraps
  `ra_ap_ide` at pin `=0.0.307` (UF-FU-008 tracks the pin).
- **Measurement:** Dogfood on `gridseak-self` reports
  `high_ratio_on_calls = 10.17 %` — not 20 %+ "ship clean"; not
  < 10 % "rollback". Within the stated 10–19 % band, documented in
  T6 §10 with UF-FU-012 investigations a/b/c filed.
- **Known-miss contract:** `graphengine-parsing/tests/
  rust_layer2_proc_macro_known_miss.rs` — the proc-macro miss is
  *tested to fail*, so a future expansion-aware improvement can
  be recognised by a green test.
- **Honest status of the Rust tier:** `Heuristic + Layer-2-partial`.
  Not `Authoritative`. The tier label `Merged` exists in
  `ResolverTier` (see WS-TRUTH-R34) but no resolver produces it
  yet. When the miss-share investigation in UF-FU-012 lands, the
  label choice will be revisited.

The honest one-liner: *Rust has Layer 2, and it helps ~10 % of
calls.* That is both a real win and an unfinished one.

---

## 4. Git-signal hotspots (Layer 0) — what a real caller can do with them

T7 ships the extractor and the consumer predicate trait
(`GitSignalConsumer`). What callers get:

- `GitSignalReport.per_file: BTreeMap<PathBuf, FileSignals>` with
  `change_frequency`, `distinct_authors`, `last_touched_days`,
  `ownership_dispersion`, `hotspot_score`, `confidence`.
- Three thresholds (constants, not magic numbers):
  `ACTIVE_RECENT_MAX_DAYS = 30`,
  `HIGH_CHURN_MIN_COMMITS = 5`,
  `HOTSPOT_MIN_AUTHORS = 3`. Each predicate is gated on
  `Confidence::High` — a shallow clone never passes the guard,
  by design.
- Caveats stamped on `HealthReport.integrity_status.schema_caveats`:
  `CAVEAT_LAYER0_GIT_SIGNALS_V1` whenever the report was attached;
  `CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1` on shallow / bare;
  `CAVEAT_LAYER0_NON_GIT_V1` on non-git roots.

Correlation evidence (`experiments/results/gate2-t7-correlation/`):

- `gridseak-self.json` hand-rank top-10 matches a spot-check
  against `git log --first-parent` counts for the same paths.
- UF-FU-015 records that 2 of the 3 D3-named repos
  (`commons-lang`, `django-site`) were not cloned into this
  workspace. One-of-three corroboration is honest but not
  definitive; the follow-up gates on the remaining two.
- UF-FU-014 caught a real data-cleanliness issue:
  `graphengine5.db` + `.db-wal` appeared in the top-10 hotspots.
  The extractor is behaving correctly (they *do* change often);
  the fix belongs in `.gitignore`, not the extractor.

---

## 5. Layer 2 agreement rate

"Agreement" is interpreted here as: does Layer 2 (resolver) agree
with Layer 1 (tree-sitter + name heuristics) on the existence of
a call edge? We do **not** measure it by running Layer 1 and
Layer 2 independently and diffing; the engine already emits the
union with provenance. The relevant agreement metric is the
*`High`-confidence call-edge share* on each canary.

Measured numbers:

| canary            | language      | `high_ratio_on_calls` | source                                                         |
| :---------------- | :------------ | :-------------------: | :------------------------------------------------------------- |
| gridseak-self     | Rust          |         10.17 %       | `gate1-2-t6-pr2-dogfood/rollup.json`                           |
| NPSP (apex only)  | Apex          |         10.0 %        | parse log `gate3-t8-npsp-canary/parse.apex.log`                |

Both figures are under the plan's "ship clean" bar of 20 %. The
10.17 % on Rust triggers UF-FU-012 investigations; the 10.0 % on
Apex is a fully-heuristic pipeline (Layer 2 for Apex is
`ApexClassSymbols`-based, not LSP-based, so the `High` bucket on
calls is pulled by TR-A-era work, not by T6). The numbers are
comparable in magnitude for two very different reasons and should
not be aggregated across languages.

---

## 6. Coverage-adjusted dead-code

T8's headline contribution, measured directly.

| axis                                         | value          |
| :------------------------------------------- | :------------: |
| Parsed Apex files                            | 1,070          |
| Files with at least one invalidating gap     | 318 (29.7 %)   |
| R39 (property accessor) instances            | 733 across 231 files |
| R41 (map-literal initialiser) instances      | 357 across 124 files |
| `no_callers` verdicts (production)           | 567            |
| `no_callers` at `High` after T8              | 251            |
| `no_callers` downgraded `High → Medium` by T8 | 316 (55.7 %)  |
| Git-signal churn downgrade contribution       | 0 (shallow clone on the canary) |

Raw artefact: `experiments/results/gate3-t8-npsp-canary/report.t8.json`.

Interpretation discipline (repeated here because it is the
load-bearing honesty guardrail):

- A `Medium`-confidence dead-code verdict is **still** a dead-code
  candidate. T8 did not *remove* any verdict; it *demoted* the
  claim.
- The 55.7 % downgrade rate is a single-repo datum. It does not
  generalise to "T8 demotes half of all dead-code findings
  everywhere". It does mean that on NPSP, pre-T8 confidence
  reporting was systematically overclaimed.
- `no_callers_high_confidence = 251` is the number a caller
  should use when ranking candidates for delete-safe review.
  `no_callers_total = 567` is the number they should use when
  sizing the dead-code backlog.

Mutually-exclusive failure mode the dual metric now rules out: a
future extractor-fix PR that silently drops `no_callers_total`
would be caught by the canary — it should drop the downgraded
count and raise the high-confidence count, never the total.

---

## 7. What Phase 3 does *not* prove

- **That the remaining 251 `High` verdicts are all true.** T8
  only covers R39 / R41. R40, R45, and any unknown coverage
  shapes still hide outgoing edges. Expanding coverage is the
  T8.b / T8.c line of follow-up work.
- **That the `fidelity_gap` numbers will survive Phase B.** Phase
  B is expected to move edges from the heuristic bucket to the
  `High` bucket; today's gap numbers are an artefact of today's
  extractor, not a steady-state fact.
- **That T7 is enough to defend "hotspot" claims on shallow
  clones.** The shallow-clone guard correctly reports `Low`
  confidence on all file-signals, which means the churn downgrade
  did nothing on the NPSP canary. The fix is a full-depth clone,
  not more engine work.

---

## 8. Open follow-ups with measurement citations

(summary index — full detail in `FOLLOWUPS.md`)

- UF-FU-012 — Rust Layer 2 adapter miss-share (88.9 %) and
  SymbolIndex loss rate (36 %). Raw data:
  `experiments/results/gate1-2-t6-pr2-dogfood/rollup.json`.
- UF-FU-013 — T7 shallow-clone acceptance test location
  (`graphengine-analysis/tests/t7_npsp_layer0_shallow_caveat.rs`).
- UF-FU-014 — `graphengine5.db{,-wal}` leakage into the
  `gridseak-self` hotspot list.
- UF-FU-015 — D3 two-repo corroboration for hand-rank agreement.
- UF-FU-016 — T7 RSS delta vs baseline (absolute number is not
  cleanly measurable).
- UF-FU-017 — 2 NPSP coverage rows with `Low` confidence and empty
  `coverage_gaps`.
- UF-FU-018 — T8 per-gap baseline re-measurement after Phase-B-R39
  / Phase-B-R41 lands.

---

## 9. Reader's digest

If you read only three lines:

1. Rust now has Layer 2, and it helps ~10 % of calls (T6 §10).
2. Git signals are live, shallow-clone-safe, and stamped with
   `Confidence::Low` on every non-full repo (T7 §9).
3. T8 demoted 56 % of NPSP's `no_callers` verdicts from `High` to
   `Medium`. The total stayed flat. The claim is honestly
   smaller, not differently sized (T8 §9).
