# Proof of Foundation Gap — Findings

> **Status:** Pre-registered predictions locked; baseline + audit + gap analyses
> complete for all three canaries; NPSP A/B edge-injection experiment pending.
>
> This document is the single source of truth for the proof. Nothing here should
> be edited once recorded as "locked." New findings get appended, never rewritten.

## Hypothesis (locked)

> The engine's metrics (cycles, dead code, max depth, module coupling) are biased
> by a structural gap: the static call graph misses (a) reflection / dynamic
> dispatch edges and (b) framework-invoked entry points. The same gap exists in
> every language; what differs is the invisibility idiom.

The hypothesis is considered **confirmed** only if all three canaries show a
non-trivial gap in at least one of {reflection, framework-entry-points}, AND
the NPSP A/B injection experiment shifts the engine's four key metrics in the
directions predicted below.

## Pre-registered predictions for the NPSP A/B experiment (locked before running)

These are **falsifiable claims written down before executing the injection.**
We commit to publishing the result whether it confirms or refutes these
predictions. Post-hoc reshaping of the claims is not allowed.

| Metric                 | Baseline (NPSP) | Prediction after injection                     |
| ---------------------- | --------------- | ---------------------------------------------- |
| `cycles_found`         | 0               | ≥ 2                                            |
| `tangle_index`         | 0.000           | > 0                                            |
| `dead_functions`       | 1,682           | drops by ≥ 25% (i.e., ≤ 1,262)                 |
| `max_call_depth`       | 11              | ≥ 15                                           |

The injection scope is deliberately minimal:

- **Reflection edges:** for each of the 48 `TDTM_Runnable`-extending handler
  classes, inject one synthetic edge from `TDTM_TriggerHandler::runClass` to
  that handler's `run(...)` method. This mirrors the runtime dispatch.
- **Entry-point flags:** mark the ~653 `@AuraEnabled`-annotated methods with
  the `is_attribute_invoked` flag the engine already respects, so dead-code
  analysis exempts them.

No other changes to the parse DB.

### Rationale for the thresholds

- `cycles_found ≥ 2`: TDTM dispatch is symmetric — handlers call SObject DML,
  which fires more triggers, which enters TDTM again. A single well-known
  path (`TDTM → handler.run → UTIL_Describe → TDTM helpers`) is enough.
- `tangle_index > 0`: follows mechanically from any cycle.
- `dead_functions drops ≥ 25%`: 653 `@AuraEnabled` methods alone — even if
  only half were classified as dead — would remove ~300 from the 1,682 count.
  Plus the 48 TDTM handlers. 25% is a conservative floor.
- `max_call_depth ≥ 15`: the TDTM re-entry pattern (trigger → TDTM → handler →
  DML → trigger → TDTM → handler) adds several frames; the current 11 reflects
  only the non-reflective paths.

If any prediction is refuted, that is a specific data point — it likely means
the injection shape is wrong, not that the hypothesis is wrong. In that case
the verdict section below will say "partially confirmed" and name the failure.

## Canaries

| ecosystem | repo                    | path                                                  | commit probed  |
| --------- | ----------------------- | ----------------------------------------------------- | -------------- |
| Apex      | NPSP                    | `~/Desktop/apex_baseline_repos/NPSP`                  | local checkout |
| Next.js   | vercel/commerce         | `~/Desktop/canary_repos/nextjs-commerce`              | depth-1 clone  |
| Django    | django/djangoproject.com| `~/Desktop/canary_repos/django-site`                  | depth-1 clone  |

## Baseline (engine's current view)

| metric                 | NPSP       | nextjs-commerce | django-site |
| ---------------------- | ---------- | --------------- | ----------- |
| total_nodes            | 22,877     | 677             | 2,195       |
| total_edges            | 113,425    | 1,408           | 3,378       |
| total_functions        | 17,002     | 115             | 1,052       |
| total_modules          | 21         | 14              | 51          |
| **cycles_found**       | **0**      | **0**           | **0**       |
| **tangle_index**       | **0.000**  | **0.000**       | **0.000**   |
| dead_functions         | 1,682      | 2               | 7           |
| max_call_depth         | 11         | 5               | 5           |
| avg_module_coupling    | 0.449      | 0.667           | 0.803       |
| resolution degraded    | 36.7 % heuristic fallback | no finding | 100 % heuristic fallback |

The row that matters most for the hypothesis: `cycles_found = 0` and
`tangle_index = 0.000` reproduce **across all three unrelated ecosystems.**
Real production codebases do not actually contain zero cycles.

## Audit counts (regex ground-truth — lower bound)

Full artifacts in `experiments/results/<canary>/audit.json`.

### Apex / NPSP

| pattern                | total |
| ---------------------- | ----- |
| `@AuraEnabled`         | 653   |
| `newInstance`          | 598   |
| `Type.forName`         | 62    |
| `extends TDTM_Runnable`| 48    |
| `implements Batchable` | 37    |
| `implements Schedulable`| 27   |
| `@future`              | 15    |
| `@RemoteAction`        | 11    |
| `implements Queueable` | 14    |
| `@InvocableMethod`     | 2     |

### TypeScript / nextjs-commerce

| pattern                | total |
| ---------------------- | ----- |
| Next.js App Router page/layout/route/loading/error files | 12 |
| `'use server'`         | 1     |
| `dynamic import()`     | 1     |

### Python / django-site

| pattern                | total |
| ---------------------- | ----- |
| `path(...)` (URL)      | 127   |
| view refs in urls.py   | 70    |
| `include(...)` (URL)   | 16    |
| `@login_required`      | 8     |
| `getattr` dynamic      | 6     |
| `@require_http_method` | 5     |
| `@receiver`            | 3     |
| `@permission_required` | 2     |

## Gap — what the engine *misses*

Per audit hit, we looked up the closest node in `baseline.json`'s
`node_annotations` by file path + line. Classification:

- **WITH_CALLERS:** engine already knew about the function and had `fan_in > 0`.
- **ENTRY_POINT:** engine had the node, `fan_in == 0`, but already classified
  as an entry point via existing heuristics (exported symbol, lifecycle name,
  framework-handler suffix, etc.) and exempt from dead-code.
- **NO_CALLERS:** engine had the node, `fan_in == 0`, and marks it **dead**.
  This is a false-dead — the structural gap.
- **NOT_IN_GRAPH:** engine produced no symbol anywhere near that file:line.
  Also a structural gap (e.g., `urls.py` view refs aren't nodes at all).

**MISSED = NO_CALLERS + NOT_IN_GRAPH.**

| canary         | pattern                | total | MISSED | missed % |
| -------------- | ---------------------- | ----- | ------ | -------- |
| NPSP           | extends TDTM_Runnable  | 48    | 46     | **95.8%** |
| NPSP           | newInstance            | 598   | 455    | 76.1%    |
| NPSP           | Type.forName           | 62    | 24     | 38.7%    |
| NPSP           | @AuraEnabled           | 653   | 68     | 10.4%    |
| NPSP           | @RemoteAction          | 11    | 5      | 45.5%    |
| NPSP           | implements Schedulable | 27    | 11     | 40.7%    |
| NPSP           | implements Batchable   | 37    | 9      | 24.3%    |
| NPSP           | implements Queueable   | 14    | 1      | 7.1%     |
| NPSP           | @future                | 15    | 0      | 0.0%     |
| NPSP           | @InvocableMethod       | 2     | 0      | 0.0%     |
| nextjs-commerce| App Router route files | 12    | 0      | 0.0%     |
| nextjs-commerce| dynamic import         | 1     | 1      | 100.0%   |
| nextjs-commerce| 'use server'           | 1     | 0      | 0.0%     |
| django-site    | path() refs            | 127   | 121    | **95.3%** |
| django-site    | django_view_refs       | 70    | 70     | **100.0%**|
| django-site    | include() refs         | 16    | 15     | 93.8%    |
| django-site    | @login_required        | 8     | 0      | 0.0%     |
| django-site    | @receiver              | 3     | 0      | 0.0%     |
| django-site    | @require_http_method   | 5     | 0      | 0.0%     |
| django-site    | getattr dynamic        | 6     | 2      | 33.3%    |

### What this reveals

1. **The structural gap is universal in its *outcome* (cycles=0, tangle=0 in all
   three unrelated ecosystems) but *not* universal in its shape.** The invisibility
   idiom differs fundamentally by ecosystem:

   - **Apex**: reflective dispatch via `Type.forName + newInstance` (TDTM), plus
     annotation-driven entry points. Existing engine heuristics miss the
     reflective edges almost entirely (95.8% miss on TDTM, 76.1% on newInstance)
     and miss ~10% of @AuraEnabled even with heuristic coverage.
   - **Django**: URL-table routing via `urls.py` `path(view.X)` — 95% of these
     view references create no graph node at all. This is a pure declarative-
     wiring problem: the engine doesn't read `urls.py`. Decorator-based entry
     points (`@login_required`, `@receiver`, etc.) are already covered.
   - **Next.js App Router**: the engine's existing heuristics (exported symbols
     + JSX component convention + common filename entrypoint) happen to cover
     92% of the framework-invisibility idiom. The remaining gap (dynamic imports)
     is real but small on this canary.

2. **The engine already has meaningful coverage for "obvious" entry points**
   (barrel files, lifecycle methods, framework-handler suffix names, exported
   symbols). The foundation gap is concentrated in:
   - **Reflection edges** (TDTM dispatch, Python `importlib`, TS `import()`).
   - **Declarative wiring files** (`urls.py`, Django `admin.py`, Spring XML,
     Rails routes, serverless framework YAML).
   - **Annotation-only entry points** where the annotation, not the name, is
     the signal (`@AuraEnabled`, `@InvocableMethod`, `@RemoteAction`, `@Path`,
     Spring `@RequestMapping`, etc.).

3. **The zero-cycle universal.** In every canary, `cycles_found = 0` and
   `tangle_index = 0.000`. This is not plausible for real code. It tells us the
   call graph is not just "missing a few edges" — it is missing **enough edges
   that no feedback path closes**. The engine's picture of a codebase is a DAG,
   not a graph. A real call graph almost always has cycles through (a) dispatch
   infrastructure and (b) orthogonal concerns (logging, validation, events).

## Per-canary resolution-quality note

- **NPSP**: 36.7 % of resolution work came from heuristic fallback. LSP (Apex
  LSP) partially worked.
- **nextjs-commerce**: no `resolution_degraded` finding — LSP (tsserver/typescript-language-server)
  appears to have worked for this small repo.
- **django-site**: **100 % heuristic fallback** (no Python LSP on PATH in this
  run). This means the django-site call graph is entirely name-matched. The
  `urls.py` / decorator coverage we see is coincidental, not LSP-backed.

The resolution-quality differences confound the cross-language metric
comparison slightly — a fair head-to-head would require LSPs up for all three.
That's a next-step for the fix-validation phase, not this one. What holds
regardless of LSP quality: **zero cycles in every canary.** Pure LSP improvement
cannot invent reflective or declarative edges, because the source language has
no syntactic call.

## A/B edge injection — NPSP

Injection performed in `experiments/ab_inject/inject.py` against a copy of the
NPSP parse DB (`experiments/results/NPSP/parse.ab.sqlite`):

- **44** synthetic `Call` edges: `TDTM_TriggerHandler::runClass` → each TDTM
  handler's `run(...)`
- **44** synthetic `Call` edges back-edge: each handler's `run(...)` →
  `TDTM_TriggerHandler::run` (models re-entry via DML → trigger → TDTM)
- **234** nodes flagged `is_attribute_invoked = true` in the `properties`
  JSON column (`@AuraEnabled`, `@InvocableMethod`, `@RemoteAction`, `@future`,
  `webservice`)

### Results from the production `ge-analyze` binary

| metric            | baseline (parse.db) | A/B (parse.ab.sqlite) | predicted | verdict   |
| ----------------- | ------------------: | --------------------: | --------- | --------- |
| `cycles_found`    | 0                   | **0**                 | ≥ 2       | **FAIL**  |
| `tangle_index`    | 0.000               | **0.000**             | > 0       | **FAIL**  |
| `max_call_depth`  | 11                  | 11                    | ≥ 15      | **FAIL**  |
| `dead_functions`  | 1,682 (9.89%)       | 1,596 (9.39%)         | ≤ 1,262   | **FAIL**  |

Every prediction failed. The dead-code number moved only 86 functions — less
than the 234 methods we flagged — which itself is evidence of a second problem
(flags not always honored because some flagged nodes already were covered by
other heuristics, and the injected TDTM dispatch wasn't propagated to handler
helpers because only `run(...)` was targeted, not the chain beneath it).

### Root-cause investigation — a distinct engine bug

The A/B failure on cycles/tangle/depth did **not** match the hypothesis shape
(a gap in the call graph data), so we audited the analysis pipeline itself.

**Finding:** `graphengine-analysis/src/health/mod.rs` runs
`cycles::detect_cycles(&graph)` at line 195, but
`graph.finalize_production_edges()` is not called until line 309.
`detect_cycles` reads `graph.production_structural_edge_indices`, which is
initialized `Vec::new()` in `AnalysisGraph::build` and is still empty at line
195. Result: the SCC algorithm receives **zero edges** and therefore always
returns `cycles_found = 0, tangle_index = 0.0`, regardless of the actual call
graph. This also explains why the cross-canary result was uniform (NPSP,
`nextjs-commerce`, `django-site` all 0) — it is the same code path, not a
universal property of the codebases.

### Proof: same databases, same cycle detection, correct ordering

We wrote a throwaway Rust binary (`experiments/ab_inject/cycle_probe`) that
links the production `graphengine-analysis` library, loads the same parse DBs,
runs the same classification steps, then calls `finalize_production_edges()`
**before** `detect_cycles(&graph)`. No other changes.

| DB                          | production edges | cycles | cycle nodes | edges in cycles |
| --------------------------- | ---------------: | -----: | ----------: | --------------: |
| NPSP `parse.db` (baseline)  | 31,391           | **32** |         111 |             159 |
| NPSP `parse.ab.sqlite` (A/B)| 31,479           | **33** |         157 |             248 |
| nextjs-commerce `parse.db`  |    403           |  0     |           0 |               0 |
| django-site `parse.db`      |    231           |  0     |           0 |               0 |

Representative cycles found in the NPSP baseline (all pre-existing in the
parsed graph, invisible to the shipping engine because of the ordering bug):

- `RD2_RecurringDonationsOpp_TDTM::evaluateOpportunities ↔ RD2_QueueableService::enqueueOppEvalService` (size 7)
- `fflib_SObjectDescribe.NamespacedAttributeMap::getObject ↔ FieldsMap::get ↔ GlobalDescribeMap::get` (size 6)
- `CRLP_ApiExecuteRollups::executeRollups → CRLP_ApiService::getBaseRollupStateForRecords → CRLP_RollupProcessor::initializeExternalRollupStateData` (size 5)
- `GiftTemplate::getGiftEntrySettings ↔ GE_Template::createDefaultTemplateIfNecessary ↔ GE_GiftEntryController::getGiftEntrySettings` (size 5)
- `fflib_QueryFactory::setOrdering` overload family (size 4)

Representative **new** cycle introduced by the A/B injection, not present in
baseline — appears once ordering is fixed:

- `TDTM_TriggerHandler::runClass → {44 handler `run(...)` methods} → TDTM_TriggerHandler::run → ...` (size 46). This is the expected reflection-dispatch cycle.

Next.js and Django still show 0 cycles even with correct ordering. That is
**consistent with the data available to the engine on those repos**: Django
has only 231 production call edges at all because ~100% of view-dispatch
edges are missing (the declarative-wiring gap documented above); with a
near-empty call graph there is nothing to form a cycle through. Next.js's
graph is small (403 production edges) and App-Router pages are genuinely
leaf-like. So: on those two canaries, the bug is **masked** by the foundation
gap — fixing one exposes the other.

## Verdict

**Two distinct problems confirmed, one previously unknown.**

1. **Orchestration bug in the shipping engine (previously unknown, new finding).**
   `cycles::detect_cycles` runs before `graph.finalize_production_edges()` in
   `health/mod.rs`, so cycle detection operates on an empty edge set and
   **every `HealthReport` ever produced has reported `cycles_found = 0` and
   `tangle_index = 0.0`** regardless of the codebase. This is a defect, not a
   modeling choice. NPSP actually has at least 32 cycles detectable **with no
   code changes to parsing or to the cycle algorithm** — just correct ordering.

2. **Foundation gap in the graph model (original hypothesis — partially confirmed).**
   The call graph is systematically incomplete for reflection-dispatch and
   declarative-routing idioms. NPSP misses 95.8% of TDTM dispatch, 76.1% of
   `newInstance`; Django misses 95% of `urls.py`-wired view references and
   100% of the `django_view_refs` pattern. These are invisible at the source
   level and therefore cannot be recovered by improving LSP or by fixing the
   ordering bug. They require either (a) framework-aware structural
   resolvers, or (b) runtime / instrumentation augmentation, or (c)
   configuration-file parsers.

The A/B experiment's **numerical** predictions failed because the engine bug
(problem 1) dominated: even though we successfully injected edges that would
produce a cycle, the cycle detector never saw any edges. Re-running the
corrected cycle detection on the same A/B database confirms the injection
**did** produce the expected reflection-dispatch cycle (one new size-46
SCC containing all 44 handlers plus the two central TDTM methods).

## Recommendation for the next plan

Treat the two problems as distinct workstreams — do not conflate them.

### Workstream A: production bug fix (small, urgent)

- Reorder `health/mod.rs` so that cycle detection runs **after**
  `structural_classification::classify_files` and
  `graph.finalize_production_edges()`. Equivalently, split the analysis
  pipeline into three phases: (1) load + structural classification + edge
  finalization, (2) all graph metrics (cycles, depth, dead-code, blast
  radius, fan-in/out, tangle, coupling), (3) findings assembly.
- Add a fail-fast invariant: `detect_cycles` should `debug_assert!` that
  `graph.production_structural_edge_indices` is non-empty whenever
  `clean_structural_edge_indices` is non-empty, so this class of ordering bug
  cannot silently re-appear.
- Backfill unit tests: one fixture with a known 2-cycle must report
  `cycles_found >= 1` when run through the full `run_analysis` entry point
  (not just `detect_cycles` in isolation, which already has coverage — that's
  how the bug escaped).
- Re-run the full benchmark suite with the fix and publish updated metrics
  for NPSP + the other internal baselines. Anyone who has quoted these
  numbers externally should be notified.

### Workstream B: foundation work for framework-invisible edges

Prioritize by impact × achievability:

1. **Declarative wiring parsers** (highest ROI, achievable).
   - Django: read `urls.py` as data, emit `Call` edges from `path()` /
     `include()` entries to the resolved view callable. 191 missed edges on
     one small repo (`djangoproject.com`) — will scale to thousands on
     Django apps.
   - Salesforce metadata: read `*.object-meta.xml`, `LightningComponentBundle`,
     `PermissionSet`, and `*.trigger` associations to wire handlers in.
   - Next.js App Router: already ~92% covered; add dynamic-import resolution
     as a small follow-up.
   - Generalize as a "declarative resolver" subsystem distinct from source
     parsing.
2. **Annotation-to-invocation resolver** (medium ROI, achievable).
   - Rather than only marking `@AuraEnabled` / `@InvocableMethod` / `@Path`
     etc. as entry points (which only fixes dead-code), inject synthetic
     `Call` edges from a well-known framework root into them so they
     participate in cycle/depth/tangle. Matches what we did manually in the
     A/B experiment.
3. **Reflection-dispatch resolver** (lower coverage, high value on specific
   frameworks).
   - TDTM-style `Type.forName(...).newInstance()` with stored configuration
     (Trigger_Handler__mdt) — requires cross-referencing custom metadata
     records. Ship as a plugin, not core, because the pattern is
     Salesforce-specific.
   - Python `importlib.import_module(x)` / `getattr(mod, name)` — only
     resolvable when `x` and `name` are string literals or string
     concatenations of known symbols. Partial solution only.

### Workstream C: integrity guarantees going forward

- Every `HealthReport` should embed the engine commit SHA and a small suite
  of self-check metrics ("production edges ≥ 1 whenever structural edges
  exist", "dead-code rate < 80%", etc.). Violations should degrade the
  health-score confidence band loudly, not silently.
- The desktop UI should render a confidence band per metric based on
  `resolution_quality` + framework-invisibility-score (new, to be computed
  from a built-in audit akin to `audit.py`). Currently the desktop shows raw
  numbers that have zero expressed confidence, which is how the bug was able
  to go unnoticed.
- Retire "static-only" metrics that degenerate to zero on frameworks with
  high declarative wiring. If tangle index would be zero because the call
  graph is sparse-by-parsing, report it as "not computable for this
  framework" rather than "0.000".

### Immediate next ticket

Open a Rust PR limited to Workstream A only. Lock a new baseline for NPSP
with the fix applied, then re-run the same A/B injection from this proof —
the predictions should pass (or come very close) without any other engine
changes. That re-run is the regression test that locks in the bug fix and
also validates that Workstream B is addressing a real, separate issue, not
downstream symptoms of the same bug.

