# Customer Readiness Gate — Universal-Fidelity Sprint

> **Status.** Gate 4.3 itemisation (2026-04-21).
> **Companion doc.** [`INCEPTION_REPORT.md`](INCEPTION_REPORT.md) — measurement artefact.
> **Sprint plan anchor.** [`../../02-strategy/SPRINT_PLAN.md`](../../02-strategy/SPRINT_PLAN.md), WS-HONESTY row.

This document enumerates — at the same level of detail an on-call
engineer would — what is actually shippable to a customer today
from the universal-fidelity sprint's engine-side deliverables, and
what is not. It does **not** sequence work or estimate days; that is
Phase 4's job. It does deliberately separate *engine-ready* from
*product-ready*: many of the remaining blockers live outside this
workstream entirely.

The honesty bar: if a line in §3 says "Ready," a customer pointing
the engine at a supported repo today will get the stated output,
from a tested code path, at the stated confidence. If a line in
§4 says "Not ready," the named blocker is real, not a conservative
hedge.

---

## 1. What "customer-ready" means in each bar

Three progressively stricter bars, used throughout §3–§5.

**Bar A — Internal validation.** Engine produces a `HealthReport`
JSON on a supported repository. A Gridseak engineer can read the
output, interpret it, cite caveats honestly, and make a
recommendation. No UI dependence. No customer-visible copy. No
SLA. This is where we are today for several signals.

**Bar B — Design-partner pilot.** A customer we chose, at a repo
we pre-scanned, with an engineer in the loop to narrate caveats
and catch edge-cases. Output may arrive as raw JSON + a curated
slide. Requires: no regressions vs. hand-audits on the chosen
repo; all caveats visible; stale-DB and shallow-clone paths tested.
No dev-license shortcut; the customer sees the real licensing
flow (WS-DESKTOP-D is allowed inside the team but not in pilot
builds).

**Bar C — General availability.** Customer installs the desktop
app, clicks scan, sees a report, understands it without an
engineer. Requires: desktop flow end-to-end (WS-DESKTOP-A),
licensing (WS-DESKTOP-C), population percentiles (WS-DESKTOP-B),
all caveats surfaced in UI, `ResolutionDegraded` gating shipped
(WS-PROOF-R3), and a Salesforce-grade Apex story
(WS-APEX-A through WS-APEX-D). We are **not** at this bar and no
part of Phase 3 alone promotes us to it — Bar C is Phase 4-plus
work.

---

## 2. Scope of this doc

In scope:

- Engine-side deliverables that shipped in the universal-fidelity
  sprint (T6, T7, T8) and the T3 dual-metric plumbing they
  consume.
- Named Apex / truth / proof-foundation items that directly
  affect whether a customer can trust the engine's output on
  Apex code today.

Out of scope:

- Desktop UI wiring (WS-DESKTOP-A / C / D).
- Population norms pipeline (WS-DESKTOP-B) — independent sprint.
- Telemetry / licensing / billing (WS-DESKTOP-C.1..C.6,
  WS-DESKTOP-F.*).
- Phase B resolver expansion (whichever shape it takes).
- Any customer-facing copy changes.

---

## 3. Ready now — what ships with today's engine

### 3.1 Cross-language structural metrics (T3 dual emission)

| Capability                                                             | Bar | Evidence                                                                              |
| :--------------------------------------------------------------------- | :-: | :------------------------------------------------------------------------------------ |
| Cycles, blast radius, fan-in / fan-out, depth, layer violations        |  A  | `graphengine-analysis/tests/t3f_fidelity_gap_regression.rs` + `ge-analyze` happy path |
| Per-metric `all_edges` + `high_only` + `fidelity_gap`                  |  A  | Same test fixture; JSON shape lives in `health::report::FidelityGap`                  |
| Dead-code findings with reason breakdown (`NoCallers`, `VisibilityPrivateUnused`, `FrameworkAnnotationUnresolved`, etc.) |  A  | `health::dead_code_classifier::DeadCodeReason::all()` + 50+ classifier unit tests      |
| Integrity caveats (`schema_caveats`, `CAVEAT_STALE_PARSE_DB_V1`)       |  A  | `health::report::CAVEAT_*` constants + integration tests                              |

Internal readers today can ship reports from this layer for any
supported language. The fidelity-gap number is the honest
"we see what we see" signal.

### 3.2 Rust Layer 2 (T6)

| Capability                                                        | Bar | Evidence                                                                         |
| :---------------------------------------------------------------- | :-: | :------------------------------------------------------------------------------- |
| `ra_ap_ide`-backed semantic resolution for Rust calls             |  A  | `graphengine-ra-ide-adapter`, feature `rust-layer2` default-on                    |
| Graceful fallback when no Cargo project model is found            |  A  | `CAVEAT_SEMANTIC_RESOLVER_PROJECT_MODEL_MISSING_V1` stamped + heuristic continues |
| Proc-macro known-miss tested                                      |  A  | `graphengine-parsing/tests/rust_layer2_proc_macro_known_miss.rs`                  |
| Measured `high_ratio_on_calls = 10.17 %` on `gridseak-self`        |  A  | `experiments/results/gate1-2-t6-pr2-dogfood/rollup.json` + T6 §10                 |

What a Rust customer would see: `High`-confidence call edges on
~10 % of calls. The rest are heuristic. The header indicator is
honest about this. Bar B readiness on a Rust codebase is a
per-repo question — proc-macro-heavy projects will see lower
`High` share.

### 3.3 Layer 0 git signals (T7)

| Capability                                                               | Bar | Evidence                                                            |
| :----------------------------------------------------------------------- | :-: | :------------------------------------------------------------------ |
| Per-file `change_frequency` / `distinct_authors` / `last_touched_days` / `ownership_dispersion` / `hotspot_score`              |  A  | `graphengine-git-signals` unit + integration tests (30+ passing)    |
| Repository-shape classification (`Full` / `Shallow` / `Bare` / `NonGit`) |  A  | `RepoShape::detect` + 8 shape-detect unit tests                     |
| Shallow-clone safety: every `FileSignals.confidence = Low` on non-Full   |  A  | Integration test `t7_shallow_clone_safety` + NPSP canary run        |
| Caveats: `CAVEAT_LAYER0_GIT_SIGNALS_V1`, `..._INSUFFICIENT_HISTORY_V1`, `..._NON_GIT_V1`                                     |  A  | Constants in `graphengine-git-signals::lib`                         |
| Consumer predicate trait `GitSignalConsumer` (is_active_recent / is_high_churn / is_shared_ownership / is_actionable)         |  A  | 5 predicate unit tests; all gated on `Confidence::High`             |
| Dead-code churn downgrade integrated into `ge-analyze`                   |  A  | `health::git_signals_attach` + 5 regression tests                   |

What a customer gets with a **full-clone** repository: honest
hotspot ranking and churn-aware dead-code confidence.
What they get with a **shallow clone**: the caveat, `Low`
confidence on every file signal, and no churn downgrade.
The shallow-clone behaviour is tested and should not surprise
anyone.

### 3.4 Extraction-coverage awareness (T8)

| Capability                                                   | Bar | Evidence                                                                         |
| :----------------------------------------------------------- | :-: | :------------------------------------------------------------------------------- |
| Per-file `FileExtractionCoverage` emitted by the parser      |  A  | 2 integration tests + 6 Apex-counter unit tests + NPSP run (1,070 rows)          |
| SQLite persistence (`file_extraction_coverage` table)        |  A  | `SqliteRepository::upsert_file_extraction_coverage_sync` + read path             |
| `CAVEAT_EXTRACTION_COVERAGE_GAPS_V1` stamped on the report    |  A  | `health::coverage_attach::attach_extraction_coverage` + unit tests               |
| Classifier downgrade `NoCallers: High → Medium` on gaps      |  A  | 6 new classifier unit tests + 5 report-level integration tests                   |
| Dual metric `no_callers_total` + `no_callers_high_confidence`|  A  | Unit-tested + NPSP canary shows 567 / 251                                        |
| NPSP canary measurement                                      |  A  | `experiments/results/gate3-t8-npsp-canary/report.t8.json`; T8 §9                  |

Covered extractor gaps today: **R39** (Apex property accessors),
**R41** (Apex map-literal initialisers). Declared but not yet
emitted: `ApexTriggerBodyUncaptured`. No coverage for Java, Rust,
TypeScript, Python yet — T8.b is that follow-up.

---

## 4. Not ready — remaining blockers by bar

### 4.1 Bar B (design-partner pilot) engine-side blockers

None for a **Rust** pilot. The engine produces `HealthReport`s with
honest caveats on real Rust code today. The UF-FU-012
investigations are "make the `High` share bigger", not "fix
bugs" — shipping a pilot at 10.17 % `High` share is honest, not
broken.

For an **Apex** pilot:

- **WS-APEX-A — LSP verification loop** (Open). Java 17 +
  apex-jorje provisioning; `apex_lsp_smoke` green; integration
  tests pass. Without this, we ship the heuristic-only resolver
  for Apex. That path is correct (TR-A shipped), but the
  `ResolutionDegraded` story needs WS-APEX-D to emit the caveat
  structured. **Net:** Bar B on Apex today means "heuristic
  resolver only, no LSP narrative." Feasible with a design
  partner who knows the story.
- **WS-APEX-D — LSP robustness** (Open). Retry, restart,
  health-check, structured `ResolutionDegraded`. Needed for
  Bar-B-without-engineer-babysitting. Today `ResolutionDegraded`
  is a `Critical` finding on the NPSP run (see canary analyze
  log) but it is *not* a first-class header indicator — that is
  WS-PROOF-R3's scope.

### 4.2 Bar C (general availability) blockers

All of the following are **out of scope** for the universal-fidelity
sprint but gate GA:

- **WS-DESKTOP-A** — Interactive scan validation (native picker,
  progress events, report render, enrich_report with dev JWT,
  mid-flight cancel). Pending UI click-through.
- **WS-DESKTOP-C** — Phase-2 server routes: license mint, desktop
  magic-link, summary upload, percentile lookup, telemetry
  ingest, trend endpoint. 4–6 d.
- **WS-DESKTOP-D** — Dev-license shortcut. 0.5 d, unblocks local
  pilot demos.
- **WS-DESKTOP-B** — Population seed. Gates the percentile column.
- **WS-PROOF-R3** — Elevate `resolution_quality` to first-class
  header indicator + R34 `actual_tier` propagation. ~1.5 d.
- **WS-APEX-A / -B / -D** — Apex verification + robustness.
- **T8.b** — Non-Apex coverage signals. Deferred; gated on
  measured false-positive evidence per language.

None of these are engine-fidelity problems. They are product-
shell problems. Shipping GA without any of them would either
silently regress a claim (percentiles without a population) or
confuse a customer (report without a progress UI).

### 4.3 Engine-side follow-ups (not blockers, noted for honesty)

- **UF-FU-012** — Rust Layer 2 adapter miss-share 88.9 %.
  Investigations (a)/(b)/(c) queued; item (a) alone could push
  `High` share from ~10 % to ~16 %.
- **UF-FU-015** — D3 two-repo corroboration for git-signal
  hand-rank agreement. 1-of-3 corroboration is what we have.
- **UF-FU-016** — T7 RSS-delta measurement. Current absolute
  number is not cleanly interpretable.
- **UF-FU-018** — T8 per-gap baseline re-measurement after
  Phase-B-R39 / R41 extractor fixes.

Each of these is an engine-side delta. None stops a Bar B pilot.
Each matters for Bar C narrative quality.

---

## 5. Staged customer bars — concrete offer table

| Customer shape                                  | Bar | Engine requires                                          | Product shell requires                                  | Status |
| :---------------------------------------------- | :-: | :------------------------------------------------------- | :------------------------------------------------------ | :----: |
| Rust infra team, full-clone repo                | A   | Landed (T6 + T7 + T3)                                    | n/a                                                     | Ready  |
| Rust infra team, full-clone repo, design-partner pilot | B | Same as A                                                | Dev-license shortcut (WS-DESKTOP-D) OR raw JSON delivery | **Ready pending WS-DESKTOP-D (0.5 d) or engineer-delivered JSON** |
| Apex customer, heuristic-only, design-partner pilot | B | Landed (TR-A + T8 + T3); acceptable per design-partner script | Dev-license OR raw JSON delivery                     | Ready if customer accepts heuristic narrative |
| Apex customer, LSP-backed, design-partner pilot | B   | + WS-APEX-A verification + WS-APEX-D robustness          | Dev-license OR raw JSON delivery                        | **Blocked — WS-APEX-A / -D** |
| Any customer, GA                                | C   | + T8.b expansion for each of their languages             | + WS-DESKTOP-A/B/C, + WS-PROOF-R3 header indicator      | **Blocked — multi-workstream** |

This table is the "what can we sell next" view. It is deliberately
conservative about Bar B on Apex-with-LSP; the heuristic path is
complete but the "full-spec" LSP path is not.

---

## 6. What we are explicitly not claiming

- We are not claiming that the 251 `High`-confidence
  `no_callers` verdicts on NPSP are all true dead code.
  `High`-confidence means "no T7 or T8 signal demotes the
  verdict", not "verified against runtime behaviour".
- We are not claiming that the `fidelity_gap` numbers will shrink
  by a specific amount when Phase B lands. We know the gaps come
  from counted failure modes; we do not know the exact magnitude
  until each failure mode ships a fix.
- We are not claiming that T8 covers every extractor blind spot.
  R39 and R41 are shipped. R40 is declared and not yet emitted.
  The rest of the extractor's walk-completeness story is a T8.b
  scope that has not been audited per-language.
- We are not claiming desktop-shell readiness. Everything about
  the UI — from the scan progress bar to the report render to
  the licensing flow — is another workstream. The engine could
  ship its output into `curl` today; nothing more.

---

## 7. Open questions for Phase 4

Phase 4 is out of scope here, but these are the questions that
need an answer before customer outreach begins:

1. Which customer shape is the target for the first pilot? The
   "Rust full-clone infra team" path is the shortest; the "Apex
   customer with LSP" path is the largest story but requires
   WS-APEX-A + -D to land first.
2. Do we ship Bar B without WS-DESKTOP-D? If yes, pilot delivery
   is engineer-in-the-loop via raw JSON, which is acceptable but
   constrains scale. If no, we need to allocate ~0.5 d to
   WS-DESKTOP-D.
3. Does Bar B tolerate the shallow-clone narrative? (A pilot
   customer who only exposes a shallow mirror will see
   `Confidence::Low` on every git signal, which is correct but
   flattens half the narrative. The fix is always "deeper clone,"
   not "more engine work".)
4. What is the first non-Apex T8.b language? The T8 design doc
   §8 names the candidates; picking one buys us an honest signal
   in another language's dead-code reports but consumes the
   extractor team for its duration.

Each question is an input to a Phase 4 scope decision; none is an
engine-side task.
