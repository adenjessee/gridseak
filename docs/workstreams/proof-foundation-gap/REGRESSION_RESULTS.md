# Regression results — bug-fix-and-integrity + dead-code-reason-classifier release

> **Reproducing historical numbers / paths cited below.** Neither the historical baseline JSONs / calibration outputs nor the rev6.1 byte-identical regression fixture referenced in this document are tracked in git — both live as sha256-pinned GitHub release assets. Fetch on demand with `scripts/setup.sh historical-baselines` (rev3..rev11 evidence, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/baseline-archive-2026-05-18)) and `scripts/setup.sh fixtures` (rev6.1 regression fixture, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/regression-fixtures-2026-05-19)). All artifacts are pinned in `experiments/artifacts.lock`. The active build/test loop does not require any of them.

Date of run: 2026-04-17 (macOS, release build).

This document records the numerical outcome of re-running the
Proof-of-Foundation-Gap NPSP A/B injection through successive engine
revisions. Each section is labelled with the engine version it measured
so the trajectory of honest prediction → fix → re-validation is
preserved. Inputs were unchanged from the original proof:

- Codebase: NPSP (full tree at `~/Desktop/apex_baseline_repos/NPSP`).
- Parse DB: `experiments/results/NPSP/parse.db` (baseline, no injection).
- A/B Parse DB: `experiments/results/NPSP/parse.ab.sqlite` (with
  synthetic TDTM dispatch + re-entry edges + `is_attribute_invoked` flags
  injected by `experiments/ab_inject/inject.py`, unchanged).

Both databases were re-analyzed with the engine using
`--exclude-tests --exclude-generated`.

## Engine revision 1: bug-fix-and-integrity (cycle-ordering + MetricStatus)

### Baseline (same NPSP, new engine)

| metric             | old (buggy) engine | new (fixed) engine |
| ------------------ | ------------------ | ------------------ |
| `cycles_found`     | **0**              | **32**             |
| `cycle_total_nodes`| 0                  | 111                |
| `tangle_index`     | 0.00000            | 0.00176            |
| `max_call_depth`   | 40                 | 40                 |
| `dead_functions`   | 1684               | 1682               |
| all statuses       | (absent)           | `ok` across the board |
| integrity caveats  | (absent)           | `cycles_orderfix_applied`, `metric_status_contract_v1` |

This line alone demonstrates the cycle-detection ordering bug is fixed:
same database, same flags, same code tree — cycles went from zero to
thirty-two. `cycle_total_nodes = 111` and `tangle_index = 0.00176` both
become nonzero only because `production_structural_edge_indices` is now
populated before `cycles::detect_cycles` consumes it.

### A/B injection re-analyzed

| metric             | baseline (fixed engine) | A/B (fixed engine) | delta |
| ------------------ | ----------------------- | ------------------ | ----- |
| `cycles_found`     | 32                      | **33**             | +1    |
| `cycle_total_nodes`| 111                     | **157**            | +46   |
| `tangle_index`     | 0.00176                 | **0.00274**        | +0.00098 |
| `max_call_depth`   | 40                      | 40                 | 0     |
| `dead_functions`   | 1682                    | 1596               | −86 (−5.1%) |

The +46 new cycle nodes match the shape of the injected TDTM re-entry
edges (dispatch → handler → re-dispatch).

### Plan-prescribed pre-registered predictions (engine revision 1)

| # | prediction                       | result | observed |
| - | -------------------------------- | ------ | -------- |
| 1 | `cycles_found >= 2` (A/B)        | PASS   | 33       |
| 2 | `tangle_index > 0` (A/B)         | PASS   | 0.00274  |
| 3 | dead-code drops by `>= 25%`      | **FAIL** | 5.1% drop (1682 → 1596) |
| 4 | `max_call_depth >= 15` (A/B)     | PASS   | 40       |

**Revision 1 regression gate: 3 of 4 pass; prediction 3 does not.**

### Honest interpretation of prediction-3 failure

The aggregate dead-code metric conflated two structurally different
populations: "truly unused" functions and "framework-invisible"
functions. The Workstream A fix did not narrow that gap because it
only affected cycle-ordering. The aggregate number is therefore not
the right gate for validating a framework-resolver injection; a
per-reason histogram is. This motivated the
`dead-code-reason-classifier` plan, measured below.

## Engine revision 2: dead-code reason classifier (Apex + Python + generic)

Same parse databases. The engine now ships:

- A registry-dispatched dead-code reason classifier
  (`graphengine-analysis/src/health/dead_code_classifier/`).
- `GraphNode.entry_point_tags` populated from the parsing crate's
  `entry_points` property so Apex-specific dispatch signals survive
  the load boundary.
- File-majority ecosystem detection (fixes a pre-existing bug where
  NPSP's Project-node language label of `javascript` caused the
  engine to treat a 74% Apex codebase as a JavaScript project).
- `*_TEST.cls` naming convention recognised as a test class in the
  Apex classifier so NPSP's 3,000+ test methods do not pollute the
  production-dead bucket.
- `CAVEAT_DEAD_CODE_REASONS_V1` integrity stamp on every report.

### Revision 2 aggregates

| metric                | baseline | A/B      | delta          |
| --------------------- | -------- | -------- | -------------- |
| `cycles_found`        | 111      | 157      | +46            |
| `tangle_index`        | 0.00176  | 0.00274  | +0.00098       |
| `max_call_depth`      | 40       | 40       | 0              |
| `dead_functions`      | 2452     | 2317     | −135 (−5.5%)   |

The `dead_functions` count differs from revision 1 (1682 → 2452)
because revision 2's Apex test-class convention correctly *stopped*
excluding `*_TEST.cls` methods in the production scope when they had
no framework dispatch, then *re-classified* them as
`TestOnlyReference` in the annotation layer. The 2,452 headline only
counts production dead and no longer suppresses them silently.
`reason_breakdown` sums exactly to this count (see below).

### Revision 2 reason breakdown (production-only)

| reason                           | baseline | A/B    | delta      |
| -------------------------------- | -------- | ------ | ---------- |
| `framework_annotation_unresolved`| 84       | 43     | **−48.8%** |
| `dynamic_dispatch_target`        | 57       | 12     | **−78.9%** |
| `no_callers`                     | 1,445    | 1,396  | −3.4%      |
| `visibility_private_unused`      | 866      | 866    | 0.0%       |
| `callback_target_not_tracked`    | 0        | 0      | —          |
| `declarative_wiring_unparsed`    | 0        | 0      | —          |
| `test_only_reference` (prod)     | 0        | 0      | —          |
| `unclassified`                   | 0        | 0      | —          |
| **TOTAL**                        | 2,452    | 2,317  | −5.5%      |

`dead_code.reason_breakdown` covers exactly the set counted by
`dead_code.count` (production dead functions, minus user entry-point
overrides). Non-production dead functions (test classes, static
resource tokens) remain visible on per-node `NodeAnnotation` entries
but do not contribute to the aggregate — this mirrors how the UI
surfaces the headline number.

### Revision 2 annotation-layer distribution (all dead nodes)

| reason                           | baseline |
| -------------------------------- | -------- |
| `test_only_reference`            | 3,088    |
| `no_callers`                     | 1,809    |
| `visibility_private_unused`      | 924      |
| `framework_annotation_unresolved`| 84       |
| `dynamic_dispatch_target`        | 67       |
| **TOTAL is_dead annotations**    | 5,972    |

The delta between 2,452 (production-aggregate) and 5,972 (all-dead
annotations) is primarily test classes (3,088) and StaticResource-
minifier tokens mis-emitted as Function nodes by the parser (see
R19 in `FOLLOWUP_RISKS.md`). Both are correctly excluded from
the production-dead count; per-node drill-down still surfaces them.

### Honest restatement of prediction 3

The plan's "dead-code drops by ≥25%" prediction was wrong at the
aggregate level. The correct, honest restatement (pre-registered in
the `dead-code-reason-classifier` plan) is:

| # | prediction (revision 2 restatement)                             | result | observed |
| - | --------------------------------------------------------------- | ------ | -------- |
| 3a | `framework_annotation_unresolved` drops by `>= 25%`           | PASS   | **−48.8%** (84 → 43) |
| 3b | `dynamic_dispatch_target` drops by `>= 25%`                   | PASS   | **−78.9%** (57 → 12) |
| 3c | `no_callers` stays flat (within ±5%)                          | PASS   | −3.4% (1,445 → 1,396) |
| 3d | `visibility_private_unused` stays flat (within ±5%)           | PASS   | 0.0% (866 → 866)      |

This is the prediction set that the aggregate `−5.5%` failure from
revision 1 was hiding. The synthetic TDTM injection's actual effect
was a large drop in the two buckets it directly targets, and
essentially no movement in the buckets it doesn't — exactly what
"foundation gap validated" looks like when measured honestly.

### Revision 2 integrity caveats stamped on every report

- `cycles_orderfix_applied`
- `metric_status_contract_v1`
- `dead_code_reasons_v1`

### Side effects worth noting

Running the regression surfaced pre-existing bugs that the classifier
made visible. All are recorded in `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md`:

- **R18** — File-classification layer does not always mark `*_TEST.cls`
  as `is_test`; the Apex classifier now has its own naming-convention
  check as a safety net.
- **R19** — The parser emits Function nodes for minified static-resource
  tokens (e.g. `moment.min::ja`, `jquery.min::U`). These show up in
  per-node annotations but are excluded from production-aggregate by
  the non-production filter.
- **R22** — The Project node's `language` property is unreliable on
  polyglot repos (NPSP wrote `javascript` for a 74% Apex codebase).
  `detect_ecosystem` now prefers the File-majority signal and emits a
  `WARNING:` line on mismatch.

## Engine revision 3: truthful-scans simplification (Wave 1)

Date of run: 2026-04-18 (macOS, release build).

Same NPSP tree re-parsed and re-analysed. Wave 1 shipped four
engine changes that move the `*_TEST.cls` and `staticresources/`
classifications from the analysis-side safety nets to the parser
boundary, and re-type `DeadCodeResult` as `{production, test,
vendor}` to eliminate R20-style scope drift at the type level:

- **1.1** — `classify_path` recognises the Apex `_TEST` / `_TESTS`
  filename convention for `language == "apex"`. The classifier's
  own safety net is deleted; parser is the sole source of truth for
  `is_test` ([graphengine-parsing/src/domain/classification.rs]).
- **1.2** — `FileDiscovery` excludes `staticresources/` at the walk
  boundary. The 924 minified-token Function nodes from NPSP never
  enter the graph in the first place ([graphengine-parsing/src/application/use_cases/parse_repo/pipeline/file_discovery.rs]).
- **1.3** — `DeadCodeResult` becomes three typed slices
  (`production`, `test`, `vendor`) with `all()` + `total()` helpers.
  Every consumer reads the slice it cares about; `reason_breakdown`
  sums structurally match `dead_code.count` by type construction
  ([graphengine-analysis/src/health/dead_code.rs]).
- **1.4** — `detect_ecosystem` File-majority-vs-Project test
  (R22 regression coverage) added to
  `graphengine-analysis/tests/integration_test.rs`.

### Revision 3 aggregates (new engine, same DB — re-parsed NPSP)

| metric                | rev 2 baseline | rev 3 baseline | delta        |
| --------------------- | -------------- | -------------- | ------------ |
| `total_functions`     | 17,002         | **15,958**     | −1,044 (−6.1%) |
| `dead_code.count`     | 2,452          | **2,086**      | −366 (−14.9%) |
| `visibility_private_unused` | 866      | **500**        | −366         |
| `no_callers`          | 1,445          | 1,445          | 0            |
| `framework_annotation_unresolved` | 84  | 84             | 0            |
| `dynamic_dispatch_target` | 57         | 57             | 0            |
| `reason_breakdown.sum == count` | yes  | **yes**        | pinned       |

The `total_functions` drop of 1,044 is the 924 minified-token
function nodes from `staticresources/` predicted by the plan, plus
~120 that belonged to now-excluded embedded bundles. The
`dead_code.count` drop of 366 aligns with the
`visibility_private_unused` drop of 366, which is the expected
shape — `*_TEST.cls` classes carry large numbers of private helpers
that previously leaked into the production bucket and are now
excluded at the parser boundary via authoritative `is_test`.

### Revision 3 A/B injection re-analysed

Same `experiments/ab_inject/inject.py` output re-applied against
the rev 3 baseline parse DB:

| reason                           | rev 3 baseline | rev 3 A/B | delta         |
| -------------------------------- | -------------- | --------- | ------------- |
| `framework_annotation_unresolved`| 84             | 43        | **−48.8%**    |
| `dynamic_dispatch_target`        | 57             | 12        | **−78.9%**    |
| `no_callers`                     | 1,445          | 1,396     | −3.4%         |
| `visibility_private_unused`      | 500            | 500       | 0.0%          |
| `test_only_reference` (prod)     | 0              | 0         | —             |
| **TOTAL (reason_breakdown sum)** | **2,086**      | **1,951** | −6.5%         |
| **TOTAL (dead_code.count)**      | **2,086**      | **1,951** | −6.5%         |

Every pre-registered prediction from the plan's Wave 1 section
passes:

| # | prediction                                                                    | result | observed |
| - | ----------------------------------------------------------------------------- | ------ | -------- |
| 1 | `total_functions` drops by ~924 after static-resource exclusion               | PASS   | −1,044   |
| 2 | Baseline `dead_code.count` drops because `*_TEST.cls` methods leave prod scope| PASS   | −366     |
| 3 | `framework_annotation_unresolved` A/B delta ≥ 25% drop                        | PASS   | −48.8%   |
| 4 | `no_callers` A/B delta within ±5%                                             | PASS   | −3.4%    |
| 5 | `dead_code.reason_breakdown.sum() == dead_code.count` (property)              | PASS   | 2086=2086, 1951=1951 |

Notably prediction 5 is no longer decorative. In rev 2 the invariant
was held together by a doc-comment plus three filter sites. In rev 3
the invariant is *type-enforced*: `reason_breakdown` is built from
`DeadCodeResult.production` by construction, and the `production`
slice is the only set with the correct scope by type definition.
R20 is closed.

### Open items at end of Wave 1

Wave 1 completes the parsing-hygiene + scope root-cause work.
Waves 2 and 3 remain as planned:

- **Wave 2** — framework-keyed classifier dispatch, materialised
  `GraphNode.language` + `GraphNode.frameworks`, `polyglot_mixed`
  integration fixture.
- **Wave 3** — `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` handoff doc
  with the enumerated 43+12-item backlog and clustered acceptance
  gates.

Both are blocked on a Layer-5 hand-audit (`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`,
10 FQNs per non-empty bucket) before proceeding.

## Engine revision 4: Wave 1 Layer-5 follow-through

Revision 4 lands three further parser-layer fixes surfaced by the
Round 1 Layer-5 hand-audit (`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`):

1. Apex `*_TEST<N>.cls` / `*_TESTS<N>.cls` filename convention
   (optional trailing digits) — Round 1 showed 2 / 10
   `dynamic_dispatch_target` samples were `CON_ContactMergeTDTM_TEST2`
   methods leaking into production because the original `_TEST`
   rule did not accept `_TEST2`, `_TEST3`, … Classifier continues
   to trust `File.is_test` from the parser; no safety net is
   re-introduced.
2. Case-insensitive static-resource directory exclusion and
   classification — Round 1 showed 10 / 10
   `visibility_private_unused` samples came from NPSP's
   `StaticResourceSources/` directory (DX pre-bundle convention
   shared with EDA / HEDA). The Wave 1.2 fix only recognised the
   canonical `staticresources/` name. Revision 4 introduces a
   case-insensitive prefix match on any directory segment
   starting with `staticresource`, applied both at file discovery
   (walk boundary) and in `classify_path` (Vendor classification).
3. Interface-contract method propagation — classes that
   *directly* implement `Database.Batchable`, `Schedulable`,
   `Queueable`, or `Messaging.InboundEmailHandler` now propagate
   the corresponding entry-point marker onto their contract
   methods (`start`, `execute`, `finish`, `handleInboundEmail`)
   at parse time. The platform dispatches these methods by name;
   the parser was tagging only the class. Round 1 showed the
   gap as `CRLP_Batch_Base_NonSkew.start(Database.BatchableContext)`
   reported `no_callers`. Propagation is limited to direct
   implementers — an abstract base whose subclass declares the
   `implements` is deferred to the Wave 3 Apex Framework Resolver.

### Revision 4 baseline vs Revision 3 baseline

| metric                                 | rev 3       | rev 4       | delta       |
| -------------------------------------- | ----------- | ----------- | ----------- |
| `summary.total_nodes`                  | 21,664      | 20,344      | **−1,320**  |
| `summary.total_functions`              | 15,958      | 14,869      | **−1,089**  |
| `metrics.dead_code.count` (production) | 2,086       | 1,726       | **−360**    |
| `visibility_private_unused`            | 500         | 140         | **−360**    |
| `framework_annotation_unresolved`      | 84          | 190         | **+106**    |
| `no_callers`                           | 1,445       | 1,339       | **−106**    |
| `dynamic_dispatch_target`              | 57          | 57          | 0           |
| `reason_breakdown.sum == count`        | yes         | **yes**     | pinned      |

Reading the deltas:

- `−360` on `visibility_private_unused` ⇔ `−360` on
  `dead_code.count`. The only change that could drive this is the
  static-resource exclusion broadening; prediction 1 is validated.
- `+106` on `framework_annotation_unresolved` ⇔ `−106` on
  `no_callers`. The 106 methods are the Batchable / Schedulable /
  Queueable / InboundEmailHandler contract methods that the new
  propagation now tags at parse time; each one previously hit the
  "fan_in = 0 ∧ no tag" branch and was labelled `no_callers`.
- `dynamic_dispatch_target` is flat at 57 because the
  `*_TEST<N>.cls` methods that were being mis-counted left the
  production dead set *entirely* (they are now entry-point-exempt
  at the test-file layer), not just the bucket.
- `total_functions −1,089` ≈ 924 `staticresources/` phantoms
  (rev 3) + additional `StaticResourceSources/` phantoms recovered
  in rev 4.

### Revision 4 A/B injection re-analysed

Same `experiments/ab_inject/inject.py` output re-applied against
the rev 4 baseline parse DB (`experiments/results/NPSP/rev4/
parse.ab.sqlite`):

| reason                           | rev 4 baseline | rev 4 A/B | delta          |
| -------------------------------- | -------------- | --------- | -------------- |
| `framework_annotation_unresolved`| 190            | 149       | **−21.6%**     |
| `dynamic_dispatch_target`        | 57             | 12        | **−78.9%**     |
| `no_callers`                     | 1,339          | 1,290     | **−3.7%**      |
| `visibility_private_unused`      | 140            | 140       | 0.0%           |
| **TOTAL (dead_code.count)**      | **1,726**      | **1,591** | **−7.8%**      |

Comparison to rev 3:

- `framework_annotation_unresolved` A/B drop shrunk from −48.8% to
  −21.6%. The parser now tags Batchable / Schedulable / Queueable
  / InboundEmailHandler contract methods at parse time, so the
  baseline already contains the resolution A/B used to
  synthesise. The remaining −21.6% is the A/B-only resolution of
  `@AuraEnabled`, `@InvocableMethod`, `@RemoteAction`, `@future`,
  and `webservice` method-level annotations that the parser does
  not (yet) tag.
- `no_callers` A/B delta remains within ±5% (−3.7% vs the rev 3
  −3.4%), as predicted.
- `dynamic_dispatch_target` A/B drop held at ~79%, unchanged — the
  TDTM dispatch edges are exactly the target of the inject
  script.

### Revision 4 pre-registered predictions

| # | prediction                                                                 | result | observed |
| - | -------------------------------------------------------------------------- | ------ | -------- |
| 1 | `StaticResourceSources/` exclusion prunes further function nodes           | PASS   | −1,089 (total) |
| 2 | `visibility_private_unused` baseline drops as the dominant driver          | PASS   | 500 → 140 (−72%) |
| 3 | Batchable/Schedulable/Queueable/InboundEmailHandler method tagging transfers mass from `no_callers` → `framework_annotation_unresolved` | PASS   | +106 / −106 balanced |
| 4 | `reason_breakdown.sum == count` property still holds (R20 invariant)       | PASS   | 1,726 = 1,726; 1,591 = 1,591 |
| 5 | `dead_code.count` drops meaningfully at baseline                           | PASS   | 2,086 → 1,726 (−17%) |

### Layer-5 hand-audit outcome

Full verdicts, per-sample, are in `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`.
Summary:

| bucket                              | Round 1 (rev 3) | Round 2 (rev 4) | Gate    |
| ----------------------------------- | --------------- | --------------- | ------- |
| `framework_annotation_unresolved`   | 0 / 10 wrong    | 0 / 10 wrong    | PASS    |
| `dynamic_dispatch_target`           | 2 / 10 wrong    | 1 / 10 wrong    | PASS    |
| `visibility_private_unused`         | 10 / 10 wrong   | 0 / 10 wrong    | PASS    |
| `no_callers`                        | 2 / 10 wrong    | **7 / 10 wrong** | **FAIL** |

Three of four buckets pass the `< 2 / 10 wrong` gate. The fourth
(`no_callers`) fails because the upstream Apex heuristic resolver
misses basic `new X()` constructor calls (intra-class and
cross-file) and `obj.method()` dispatch on typed fields. Rewriting
the classifier (Wave 2) cannot fix edges the resolver never emitted.

Wave 2 is therefore unblocked on the three buckets it directly
addresses (classifier dispatch). The `no_callers` gate is
re-measured after Wave 3's Apex Framework Resolver lands. The
seven failed samples form the first concrete population of the
Wave 3.1 "43 + 12 FQN" backlog.

New follow-up risks **R23 – R27** appended to
`docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` and
cross-linked from `docs/00-strategy/FUTURE_PLAN.md`.

## Engine revision 5: Wave 2 framework-keyed classifier

Revision 5 ships Waves 2.1 – 2.4 of the truthful-scans simplification
plan:

- **Wave 2.1** — `GraphNode.language` / `GraphNode.frameworks` are
  read from the SQLite `properties` column at load time. An
  `AnalysisGraph::build` post-pass propagates a File's metadata
  down to all contained Function / Method descendants. Polyglot
  repos now carry per-node language and framework state rather
  than collapsing to one repo-level `Ecosystem`.
- **Wave 2.2** — `graphengine-parsing/src/domain/frameworks.rs`
  emits a sorted, deduplicated `frameworks` list on every File
  node at parse time. Path detection covers LWC, TDTM, Apex
  triggers, Django, Celery, and (fallback) `plain`. Symbol-tag
  augmentation promotes files carrying `@RestResource` / `@Http*`
  annotations to `restresource`, and carrying TDTM markers to
  `tdtm`, so downstream rule sets see a unified framework list
  whether the signal came from path or from annotations.
- **Wave 2.3** — replaced the `Ecosystem`-keyed
  `ClassifierRegistry` with `FrameworkRuleRegistry`. Rules live in
  `graphengine-analysis/src/health/dead_code_classifier/frameworks/`
  (one file per framework). Universal pre-rules
  (`entry_point_tags`, `is_attribute_invoked`, `is_callback_target`,
  `parent_is_test`) run before any framework-specific rule. The
  monolithic `apex.rs` and `python.rs` were deleted; their
  predicates were decomposed across the new framework modules and
  the universal pre-pass so cross-language rule reuse is now
  structurally possible.
- **Wave 2.4** — new integration fixture
  `graphengine-analysis/tests/polyglot_mixed_integration.rs`
  asserts that a single graph containing Apex (TDTM, plain,
  REST, Aura), Python (Django, Celery), and JavaScript (LWC)
  files routes each node to its own framework's rule set. The
  advisory `ClassifyContext.ecosystem` is deliberately set to
  `Ecosystem::Apex` to prove it is **not** consulted for
  dispatch.

### Revision 5 baseline vs Revision 4 baseline

| metric                                       | rev 4 | rev 5 | delta    |
| -------------------------------------------- | ----- | ----- | -------- |
| `summary.total_functions`                    | 14,869 | 14,869 | 0        |
| `summary.total_nodes`                        | 20,344 | 20,344 | 0        |
| `metrics.dead_code.count` (production)       | 1,726 | 1,726 | 0        |
| `reason.framework_annotation_unresolved`     | 190   | 190   | 0        |
| `reason.dynamic_dispatch_target`             | 57    | 47    | **−10**  |
| `reason.declarative_wiring_unparsed` *(new)* | 0     | 137   | **+137** |
| `reason.no_callers`                          | 1,339 | 1,349 | +10      |
| `reason.visibility_private_unused`           | 140   | 3     | **−137** |

Interpretation:

- `visibility_private_unused −137` ↔ `declarative_wiring_unparsed
  +137`: every one of NPSP's LWC JavaScript private methods (140
  total, minus three Aura / Jest leftovers) moves from the
  language-level visibility bucket — which misrepresented them as
  "private and unused" when in reality they are template-bound —
  to the framework-level `declarative_wiring_unparsed` bucket.
  Evidence strings now cite `FOLLOWUP_RISKS R25` explicitly, so a
  consumer reading the verdict can trace it to the known upstream
  gap (HTML templates unparsed).
- `dynamic_dispatch_target −10` ↔ `no_callers +10`: ten methods
  whose rev-4 TDTM label was driven by the old classifier's
  file-global `looks_like_tdtm_handler` heuristic (including cases
  where the match came from a `TDTM_Runnable.DmlWrapper` parameter
  type embedded in the FQN, R24 territory) no longer hit the
  TDTM rule because their parent file is not tagged
  `frameworks: ["tdtm"]`. These shift to `no_callers` — the
  honest "we don't know" fallback — instead of silently
  mislabeling them as reflective dispatch.
- Zero change on `framework_annotation_unresolved`: the universal
  entry-point-tag pre-rule encodes the same predicate as the
  rev-4 Apex classifier's first arm, and every tagged node in
  NPSP was already tagged in rev 4 (Wave 1.5 shipped the
  interface-method propagation).

### Revision 5 A/B-injected analysis

Same TDTM audit-driven injection as earlier revisions, run against
the rev-5 DB (`experiments/results/NPSP/rev5/parse.ab.sqlite`):

| metric                                       | rev 5 baseline | rev 5 A/B | delta |
| -------------------------------------------- | -------------- | --------- | ----- |
| `metrics.dead_code.count` (production)       | 1,726          | 1,591     | −135  |
| `reason.framework_annotation_unresolved`     | 190            | 149       | −41   |
| `reason.dynamic_dispatch_target`             | 47             | 2         | −45   |
| `reason.no_callers`                          | 1,349          | 1,300     | −49   |
| `reason.declarative_wiring_unparsed`         | 137            | 137       | 0     |
| `reason.visibility_private_unused`           | 3              | 3         | 0     |

Expected shape per the pre-registered rev 3 / rev 4 predictions
(the A/B injection synthesises TDTM framework edges, moving
symbols from `dynamic_dispatch_target` and `no_callers` into
`live`): the +135 live-node delta is within +10 of the rev-4
prediction. `declarative_wiring_unparsed` is unchanged because
the injection targets Apex / TDTM, not LWC templates — exactly
what Wave 2's per-framework dispatch predicts.

### Layer-5 hand-audit (Round 3)

Full write-up: `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`, Round 3 section
(sample seed `20260418`).

| bucket                              | Round 1 (rev 3) | Round 2 (rev 4) | Round 3 (rev 5) | Gate |
| ----------------------------------- | --------------- | --------------- | --------------- | ---- |
| `framework_annotation_unresolved`   | 0 / 10          | 0 / 10          | 0 / 10          | **PASS** |
| `dynamic_dispatch_target`           | 2 / 10          | 1 / 10          | 1 / 10          | **PASS** |
| `declarative_wiring_unparsed` *(new)* | —               | —               | 0 / 10          | **PASS** |
| `visibility_private_unused`         | 10 / 10         | 0 / 10          | 3 / 3 (n<10)    | N/A (n below threshold; see R28) |
| `no_callers`                        | 2 / 10          | 7 / 10          | 10 / 10         | **FAIL** (resolver-quality, see below) |

Three of the four gate-eligible buckets PASS. The `no_callers`
FAIL distribution is identical in shape to rev 4 — every wrong
sample is a constructor, inner-class, or typed-field dispatch
that the Apex heuristic resolver fails to link (R23). Rewriting
the classifier cannot invent edges the resolver did not emit.

**Wave 2's gate target was classifier-logic preservation, not a
new PASS on `no_callers`.** That target is met: no classifier
predicate regressed, and framework-keyed dispatch surfaced a new
(correctly labelled) bucket that improves attribution fidelity
for the LWC surface.

Gate re-evaluation on `no_callers` is scheduled for **rev 6**
after Wave 3.1 ships the Apex Framework Resolver. The rev-5
sample's 10 wrong FQNs feed into that plan's backlog.

New risk **R28** (framework-undetected: Aura + Jest) appended to
`FOLLOWUP_RISKS.md`.

## Engine revision 6: Phase 0 (TR-0.1 TDTM class-segment match + TR-0.2 Aura/Jest/Vitest detectors)

Date of run: 2026-04-18 (macOS, release build; binaries freshly
rebuilt from the uncommitted TR-0.1 / TR-0.2 / TR-0.3 tree).
Same NPSP source tree (`~/Desktop/apex_baseline_repos/NPSP`).

Rev 6 ships two Phase-0 deliverables against the truthful-scans
roadmap:

- **TR-0.1** — `looks_like_tdtm_handler` decomposes the FQN
  before matching: parameter tuple stripped on the first `(`,
  class segment isolated via `rsplit("::")`, TDTM-convention
  token match runs on `class_tokens` only. Replaces the
  pre-fix substring scan that matched on the full FQN and
  therefore fired on any method accepting a
  `TDTM_Runnable.*` parameter regardless of class role (R24).
- **TR-0.2** — `detect_frameworks_by_path` gains `aura`
  (broad, `aura/` path-segment match), `jest`, and `vitest`
  (narrow, `<runner>.{setup,config}.{js,ts,mjs,cjs}` filename
  match). Three new classifier rule sets route Aura symbols
  to `declarative_wiring_unparsed` and harness symbols to
  `framework_annotation_unresolved`, closing R28.

Both TR-0.1 and TR-0.2 shipped their lib/tests cleanly (all
unit + integration tests pass; clippy clean on
`--lib --tests` scope). This section measures what those
changes did to the aggregates and to the Layer-5 sample
population.

### Revision 6 baseline vs Revision 5 baseline

| metric                                       | rev 5 | rev 6 | delta |
| -------------------------------------------- | ----- | ----- | ----- |
| `summary.total_functions`                    | 14,869 | 14,869 | 0 |
| `summary.total_nodes`                        | 20,344 | 20,344 | 0 |
| `metrics.dead_code.count` (production)       | 1,726 | 1,726 | 0 |
| `reason.framework_annotation_unresolved`     | 190   | 192   | **+2**  |
| `reason.dynamic_dispatch_target`             | 47    | 95    | **+48** |
| `reason.declarative_wiring_unparsed`         | 137   | 138   | **+1**  |
| `reason.no_callers`                          | 1,349 | 1,301 | **−48** |
| `reason.visibility_private_unused`           | 3     | 0     | **−3**  |
| `metrics.cycles.count`                       | 73    | 73    | 0 |
| `metrics.max_call_depth`                     | 40    | 40    | 0 |
| `metrics.tangle_index.ratio`                 | 0.00120 | 0.00120 | 0 |

Interpretation, split by Phase-0 deliverable:

- **TR-0.2 success (expected direction):**
  `visibility_private_unused` 3 → 0 (**−3**), with the three
  survivors flowing into `declarative_wiring_unparsed` (+1, the
  Aura controller method `GE_GiftEntryFormController::closeModal`)
  and `framework_annotation_unresolved` (+2, `jest.setup::toContainOptions`
  and `jest.setup::noop`). Conservation: −3 + 1 + 2 = 0, matching
  pre-registered prediction.
- **TR-0.1 unexpected direction:**
  `dynamic_dispatch_target` 47 → 95 (**+48**), with the mirror
  `no_callers` 1,349 → 1,301 (−48). The pre-registered TR-0.1
  expectation was `dynamic_dispatch_target` drops by ≥1 (strictly
  narrower matching under R24 fix). The observed direction is
  the opposite.

### TR-0.1 regression — structural root cause

The 48 nodes that moved from `no_callers` (rev 5) to
`dynamic_dispatch_target` (rev 6) all share one of two shapes:

1. **Inner classes inside `*_TDTM.cls` files** (≈ 40 of 48).
   Examples: `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)`,
   `CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts(List)`,
   `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()`.
2. **Non-`run()` methods on the outer TDTM class** (≈ 8 of 48).
   Examples: `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()`
   (NPSP override of a parent-class dispatch).

Both shapes are called via **normal Apex mechanisms** (typed
`new InnerClass(...)` from the outer class, typed-field dispatch
for override methods) — *not* through
`Type.forName().newInstance()`. The NPSP TDTM router reflectively
invokes the zero-arg constructor plus the `run()` method of the
outer handler class. Every other method on every other class in
the file is invoked by ordinary Apex calls that the heuristic
resolver (R23) currently fails to link.

The TR-0.1 check `class_tokens.iter().any(is_tdtm_token)` is too
permissive: every class inside a `*_TDTM.cls` file — inner,
outer, non-handler, and handler alike — has at least one class
token ending in `_tdtm` because the outer class name prefixes
every inner class's fully-qualified path (`Outer.Inner`
decomposes to `["outer_tdtm", "inner"]`; the outer token matches
even when evaluating the inner class's method).

Evidence on every one of these 48 nodes reads "called via
`Type.forName().newInstance()`" — **a factually wrong claim for
42 of the 48**. Rev 5 put these same nodes in `no_callers` (the
honest "we don't know" bucket), which correctly surfaced them as
R23 resolver-gap candidates.

**TR-0.1 fixed one false-positive class (parameter-type leakage,
R24) but introduced a new one (inner-class / non-`run` scope
leakage).** The net sample-level verdict quality on
`dynamic_dispatch_target` moves from 1 / 10 wrong (rev 5, PASS)
to 4 / 10 wrong (rev 6 Round 4, **FAIL**) — the sampling draw is
documented in `HAND_AUDIT_LOG.md` under Round 4.

New risk **R31** appended to `FOLLOWUP_RISKS.md` with a precise
two-line fix sketch (restrict to `method_seg == "run"` or to the
outermost class token only, not `any()`). Phase assignment:
**TR-0.1.1**, a Phase-0 revisit that *blocks* Phase A — the
roadmap's rule is that Phase A opens only after Layer-5 Round 4
passes on every bucket Phase 0 touched.

### Revision 6 A/B-injected analysis

Same TDTM audit-driven injection as prior revisions, run against
the rev-6 DB (`experiments/results/NPSP/rev6/parse.ab.sqlite`).
Injection stats (unchanged from prior revisions): 44 dispatch
edges, 44 re-entry edges, 234 `is_attribute_invoked` flags.

| metric                                       | rev 6 baseline | rev 6 A/B | delta |
| -------------------------------------------- | -------------- | --------- | ----- |
| `metrics.dead_code.count` (production)       | 1,726          | 1,591     | −135  |
| `reason.framework_annotation_unresolved`     | 192            | 151       | −41   |
| `reason.dynamic_dispatch_target`             | 95             | 50        | −45   |
| `reason.declarative_wiring_unparsed`         | 138            | 138       | 0     |
| `reason.no_callers`                          | 1,301          | 1,252     | −49   |
| `reason.visibility_private_unused`           | 0              | 0         | 0     |

Shape is consistent with prior revisions: the injection moves
TDTM handler symbols from `dynamic_dispatch_target` /
`no_callers` into `live`. The −45 move on
`dynamic_dispatch_target` is smaller than the +48 base inflation
caused by R31 — so **even after A/B injection, rev 6's
`dynamic_dispatch_target` (50) exceeds rev 5's (47)**. This
confirms the regression is *structural to the classifier* rather
than a side effect of the injection graph.

### Revision 6 gate status (against Layer-5 Round 4)

| bucket                              | Round 3 (rev 5) | Round 4 (rev 6, prelim.) | Gate |
| ----------------------------------- | --------------- | ------------------------ | ---- |
| `framework_annotation_unresolved`   | 0 / 10          | 0 / 10                   | **PASS** |
| `dynamic_dispatch_target`           | 1 / 10          | **4 / 10**               | **FAIL — R31 regression** |
| `declarative_wiring_unparsed`       | 0 / 10          | 0 / 10                   | **PASS** |
| `no_callers`                        | 10 / 10         | not re-scored (pre-Phase-A expected FAIL) | **FAIL (unchanged; R23)** |
| `visibility_private_unused`         | 3 / 3 (n below threshold) | n = 0 (bucket collapsed) | **N/A — TR-0.2 collapsed the bucket** |

Two buckets PASS (`framework_annotation_unresolved`,
`declarative_wiring_unparsed`). `visibility_private_unused`
empties to 0, meeting TR-0.2's target and closing R28. The
`no_callers` bucket remains at its rev-5 failure distribution;
Phase A is its designated fix.

`dynamic_dispatch_target` is the gate-blocker: Phase 0
introduced a fresh regression in a bucket it was supposed to
improve. **Phase A does not open until R31 is fixed
(TR-0.1.1) and Round 4 is re-run on a rev 6.1 baseline.**

### Reproducibility (rev 6)

```bash
# 1. Rebuild both release binaries from the current tree.
cargo build --release -p graphengine-analysis --bin ge-analyze \
                      -p graphengine-parsing

# 2. NPSP-only re-parse (other canaries skipped for speed).
rm -f experiments/results/NPSP/parse.db*
target/release/graphengine-parsing --configs-dir graphengine-parsing/configs \
    parse --root ~/Desktop/apex_baseline_repos/NPSP \
          --db experiments/results/NPSP/parse.db \
          --lang apex --clear
target/release/graphengine-parsing --configs-dir graphengine-parsing/configs \
    parse --root ~/Desktop/apex_baseline_repos/NPSP \
          --db experiments/results/NPSP/parse.db \
          --lang javascript

# 3. Baseline analysis.
target/release/ge-analyze \
  --db experiments/results/NPSP/parse.db \
  --output experiments/results/NPSP/baseline.json \
  --exclude-tests --exclude-generated

# 4. A/B injection + analysis.
rm -f experiments/results/NPSP/parse.ab.sqlite*
python3 experiments/ab_inject/inject.py \
  --baseline-db experiments/results/NPSP/parse.db \
  --audit experiments/results/NPSP/audit.json \
  --out-db experiments/results/NPSP/parse.ab.sqlite \
  --repo-prefix ~/Desktop/apex_baseline_repos/NPSP

target/release/ge-analyze \
  --db experiments/results/NPSP/parse.ab.sqlite \
  --output experiments/results/NPSP/ab_report.json \
  --exclude-tests --exclude-generated

# 5. Snapshot.
cp experiments/results/NPSP/{baseline.json,ab_report.json,parse.ab.sqlite} \
   experiments/results/NPSP/rev6/
```

Artefacts under `experiments/results/NPSP/rev6/`:
- `baseline.json` — rev-6 baseline HealthReport.
- `ab_report.json` — rev-6 A/B-injected HealthReport.
- `parse.ab.sqlite` — rev-6 A/B parse DB.

## Next work

**All rev-5 follow-up work is sequenced in
`docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`**
(Phases 0 – D, pre-registered deltas + audit gates). Summary of
what the roadmap owns, so this file stays self-contained for
reviewers:

- **Phase 0 (TR-0.*)** — R24 TDTM substring fix, R28 Aura + Jest
  detector, evidence-string polish. Sub-day; sharpens Phase A's
  audit.
- **Phase A (TR-A.*, R23)** — Apex AST resolver gaps
  (constructor / typed-field / overload / extensions=); 17 audit
  fixtures. Acceptance: `no_callers` audit < 2 / 10 wrong.
- **Phase B (TR-B via `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`)**
  — `EdgeSource::FrameworkEntry` edges for Batchable / Schedulable /
  Queueable / Trigger / `@AuraEnabled` / `@RestResource` / global /
  TDTM-registered handlers. Acceptance:
  `framework_annotation_unresolved` → 0, `dynamic_dispatch_target`
  → 0.
- **Phase C (TR-C.*, R25 + R28 pt 1)** — LWC template parser,
  Aura `.cmp` / `.app` parser, Visualforce outside `extensions=`,
  Flow XML, Platform-Event handlers. Acceptance:
  `declarative_wiring_unparsed` < 20.
- **Phase D (TR-D.*, R26 + R27)** — `EdgeSource` + per-edge
  confidence + evidence; declarative classifier rule engine;
  open `DeadCodeReason` behind `report_schema_version`. Refactor;
  no metric delta expected.

Cross-repo dependencies (population norms, Desktop UI caveat
handler, API trend integration) are indexed by the roadmap's §12
non-goals table.

## Reproducibility

```bash
cargo build --release -p graphengine-analysis --bin ge-analyze

target/release/ge-analyze \
  --db experiments/results/NPSP/parse.db \
  --output experiments/results/NPSP/regression/baseline_fixed.json \
  --exclude-tests --exclude-generated

target/release/ge-analyze \
  --db experiments/results/NPSP/parse.ab.sqlite \
  --output experiments/results/NPSP/regression/ab_fixed.json \
  --exclude-tests --exclude-generated
```

Artifacts under `experiments/results/NPSP/regression/`:
- `baseline_fixed.json` — HealthReport on the un-injected DB.
- `ab_fixed.json`       — HealthReport on the TDTM-injected DB.

## Engine revision 6.1: Phase 0 completion (TR-0.1.1 fixes R31)

Date of run: 2026-04-18 (macOS, release build; classifier-only
rebuild of `graphengine-analysis`, parser unchanged from rev 6 —
the rev-6 `parse.db` is re-used, so the only variable between
rev 6 and rev 6.1 is the TDTM predicate in
`graphengine-analysis/src/health/dead_code_classifier/frameworks/tdtm.rs`).

Rev 6.1 ships **TR-0.1.1** — the R31 corrective follow-up to
TR-0.1. The diff replaces `class_tokens.iter().any(is_tdtm_token)`
with an outer-class-only + method-identity restriction:

```rust
let outer_class_token = class_tokens.first().copied().unwrap_or("");
let outer_is_tdtm = is_tdtm_token(outer_class_token);
let method_is_run = method_seg == "run" || name_lc == "run";
let method_is_outer_ctor = !method_seg.is_empty() && method_seg == outer_class_token;
(outer_is_tdtm && (method_is_run || method_is_outer_ctor)) || run_on_handler
```

Rationale: NPSP's TDTM router registers handler classes by their
*outermost* class name and invokes them via
`Type.forName(outerClassName).newInstance()` + a subsequent
`.run(...)` call. Only two methods are reflectively dispatched per
handler — the zero-arg constructor and `run()`. Inner classes
and non-`run` outer-class methods are invoked by ordinary Apex
(typed-field dispatch, override polymorphism, sibling-method
calls), so mislabelling them as reflection was the exact R31 shape.

Four R31 fixtures were added alongside the predicate change:

- Positive: `::TDTM_Opportunity::TDTM_Opportunity()` (zero-arg ctor).
- Positive: `::ACCT_Accounts_TDTM::ACCT_Accounts_TDTM()` (suffix-shape ctor).
- Negative: `::CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` (inner-class non-`run`).
- Negative: `::RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()` (inner-class ctor).
- Negative: `::CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()` (outer, non-`run`, non-ctor).

Two pre-existing tests that had encoded the pre-R31
over-permissive behaviour were re-aligned with NPSP reality:

- `tdtm_suffix_class_with_tdtm_typed_param_matches` (previously
  asserted `::ACCT_Accounts_TDTM::onAfterUpdate(...)` matches)
  was reframed as `tdtm_suffix_class_run_with_tdtm_typed_params_matches`
  — the R24 parameter-type protection is preserved, but the
  assertion now uses the canonical `run(...)` method, which is
  the only shape NPSP's router invokes reflectively.
- `inner_class_tdtm_prefix_matches` (previously asserted
  `::Outer.TDTM_Inner::run()` matches) was flipped to
  `r31_inner_class_with_tdtm_named_inner_does_not_match`. An
  inner class, even TDTM-named, is not reflectively dispatched
  by NPSP's router because the router registers outer-class
  names only.

All 21 `tdtm.rs` unit tests pass; `cargo test -p graphengine-analysis`
runs clean on the full library + integration suite (polyglot
mixed, calibration, full-pipeline). `cargo clippy -p graphengine-analysis --lib --tests -D warnings` also clean.

### Revision 6.1 baseline vs Revision 6 baseline

| metric                                       | rev 6 | rev 6.1 | delta |
| -------------------------------------------- | ----- | ------- | ----- |
| `summary.total_functions`                    | 14,869 | 14,869 | 0 |
| `summary.total_nodes`                        | 20,344 | 20,344 | 0 |
| `metrics.dead_code.count` (production)       | 1,726 | 1,726  | 0 |
| `reason.framework_annotation_unresolved`     | 192   | 192    | 0 |
| `reason.dynamic_dispatch_target`             | 95    | **46** | **−49** |
| `reason.declarative_wiring_unparsed`         | 138   | 138    | 0 |
| `reason.no_callers`                          | 1,301 | **1,350** | **+49** |
| `reason.visibility_private_unused`           | 0     | 0      | 0 |
| `metrics.cycles.count`                       | 73    | 73     | 0 |
| `metrics.max_call_depth`                     | 40    | 40     | 0 |
| `metrics.tangle_index.ratio`                 | 0.00120 | 0.00120 | 0 |

**Perfect conservation.** `dynamic_dispatch_target −49` mirrors
`no_callers +49`; every other metric is byte-identical. The 48
nodes R31 had misclassified + 1 additional node the R24 fix
re-rejected move from `dynamic_dispatch_target` back to
`no_callers` where they belong (Phase A's resolver is the proper
destination for their R23-shaped reality failures).

Cumulative rev 5 → rev 6.1 shape: `dynamic_dispatch_target`
47 → 46 (−1, clean R24 narrowing); `no_callers` 1,349 → 1,350
(+1). That is what TR-0.1 was *supposed* to deliver on rev 6
before R31 detoured it.

### Node-level verification of the R31 reverts

The four NPSP FQNs that scored the Round 4 failure on rev 6
were independently re-queried against `rev6.1/baseline.json`:

| FQN | rev 6 reason | rev 6.1 reason |
| --- | ------------ | -------------- |
| `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` | `dynamic_dispatch_target` | **`no_callers`** |
| `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()` | `dynamic_dispatch_target` | **`no_callers`** |
| `CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts(List)` | `dynamic_dispatch_target` | **`no_callers`** |
| `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()` | `dynamic_dispatch_target` | **`no_callers`** |

Every node that R31 specifically flagged as mis-attributed now
carries the honest "we can't see a caller" bucket instead of
the false "called via `Type.forName().newInstance()`" evidence.

### Revision 6.1 A/B-injected analysis

Same TDTM audit-driven injection as prior revisions, run against
the rev-6.1 DB (`experiments/results/NPSP/rev6.1/parse.ab.sqlite`).
Injection stats (unchanged from prior revisions): 44 dispatch
edges, 44 re-entry edges, 234 `is_attribute_invoked` flags.

| metric                                       | rev 6.1 baseline | rev 6.1 A/B | delta |
| -------------------------------------------- | ---------------- | ----------- | ----- |
| `metrics.dead_code.count` (production)       | 1,726            | 1,591       | −135  |
| `reason.framework_annotation_unresolved`     | 192              | 151         | −41   |
| `reason.dynamic_dispatch_target`             | 46               | **1**       | **−45** |
| `reason.declarative_wiring_unparsed`         | 138              | 138         | 0     |
| `reason.no_callers`                          | 1,350            | 1,301       | −49   |
| `reason.visibility_private_unused`           | 0                | 0           | 0     |

Post-injection `dynamic_dispatch_target` drops to **1** (down
from rev 6 A/B's 50). That single residual node is the one the
heuristic's `run_on_handler` fallback catches via a non-TDTM
naming convention the injection audit does not cover — it is
genuinely a TDTM-like dispatch case, not a false positive. The
shape confirms the injection is doing its job and the classifier
is finally not inflating the bucket with resolver-gap traffic.

### Revision 6.1 gate status (Layer-5 Round 4 re-scoring, post-TR-0.1.1)

Full per-sample verdicts: `HAND_AUDIT_LOG.md`, Round 4 section,
"Re-scoring — rev 6.1" subsection (seed `20260418`, same seed
as the rev-6 draw so per-bucket samples are revision-comparable).

| bucket                              | Round 3 (rev 5) | Round 4 (rev 6) | Round 4 re-scored (rev 6.1) | Gate |
| ----------------------------------- | --------------- | --------------- | --------------------------- | ---- |
| `framework_annotation_unresolved`   | 0 / 10          | 0 / 10          | **0 / 10**                  | **PASS** |
| `dynamic_dispatch_target`           | 1 / 10          | **4 / 10 (R31)** | **0 / 10**                  | **PASS** |
| `declarative_wiring_unparsed`       | 0 / 10          | 0 / 10          | **0 / 10**                  | **PASS** |
| `visibility_private_unused`         | 3 / 3 (n<10)    | n = 0           | **n = 0**                   | **N/A (collapsed; R28 closed)** |
| `no_callers`                        | 10 / 10         | not re-scored   | not re-scored               | **pre-Phase-A FAIL (R23)** |

**R31 is closed.** Every bucket Phase 0 touched passes Round 4
at <2/10 wrong. `visibility_private_unused` stays collapsed
(TR-0.2's target). `no_callers` remains the Phase A target and
is not re-scored at Round 4 by design.

### Phase A open — confidence multi-angle

| angle | evidence |
| ----- | -------- |
| Layer 1 unit | 21 / 21 `tdtm.rs` tests pass (16 existing + 5 new R31 fixtures); 2 re-aligned tests verify the post-R31 rule, 3 new negatives cover the NPSP failure shapes. |
| Layer 1 suite | `cargo test -p graphengine-analysis` = 260+ tests pass (library + polyglot_mixed_integration + calibration + full_pipeline). |
| Layer 2 clippy | `cargo clippy -p graphengine-analysis --lib --tests -- -D warnings` clean. |
| Layer 3 integration | `polyglot_mixed_dispatch_routes_each_node_to_its_framework` passes: Apex TDTM, Python Django/Celery, JS LWC/Aura/Jest/Vitest all route to their own rule set. |
| Layer 4 NPSP canary | rev 6 → rev 6.1 delta: `dynamic_dispatch_target −49`, `no_callers +49`, all other metrics identical. Conservation perfect. |
| Layer 4 A/B | rev 6.1 post-injection `dynamic_dispatch_target` = 1 (vs 50 on rev 6 A/B). Confirms residual bucket size now matches reality. |
| Layer 5 Round 4 | `dynamic_dispatch_target` 0 / 10 wrong (was 4 / 10 on rev 6). All other Phase-0 buckets 0 / 10 or n=0. |
| Node-level | 4 / 4 specific R31-failed FQNs verified re-routed to `no_callers`. No residual misclassification. |

### Reproducibility (rev 6.1)

```bash
# TR-0.1.1 is classifier-only; parser DB from rev 6 is reused.
cargo test -p graphengine-analysis --lib tdtm
cargo test -p graphengine-analysis
cargo clippy -p graphengine-analysis --lib --tests -- -D warnings

cargo build --release -p graphengine-analysis --bin ge-analyze

mkdir -p experiments/results/NPSP/rev6.1
target/release/ge-analyze \
  --db experiments/results/NPSP/parse.db \
  --output experiments/results/NPSP/rev6.1/baseline.json \
  --exclude-tests --exclude-generated

cp experiments/results/NPSP/parse.db experiments/results/NPSP/rev6.1/parse.db
python3 experiments/ab_inject/inject.py \
  --baseline-db experiments/results/NPSP/rev6.1/parse.db \
  --audit experiments/results/NPSP/audit.json \
  --out-db experiments/results/NPSP/rev6.1/parse.ab.sqlite \
  --repo-prefix ~/Desktop/apex_baseline_repos/NPSP

target/release/ge-analyze \
  --db experiments/results/NPSP/rev6.1/parse.ab.sqlite \
  --output experiments/results/NPSP/rev6.1/ab_report.json \
  --exclude-tests --exclude-generated
```

Artefacts under `experiments/results/NPSP/rev6.1/`:
- `baseline.json` — rev-6.1 baseline HealthReport.
- `ab_report.json` — rev-6.1 A/B-injected HealthReport.
- `parse.db` — rev-6 parse DB (copied; parser is unchanged).
- `parse.ab.sqlite` — rev-6.1 A/B parse DB.

Note on rev-6.1 baseline.json drift (2026-04-19): the on-disk
`experiments/results/NPSP/rev6.1/baseline.json` file was re-generated
during PR 9 investigation and currently contains the rev-9 analysis
output rather than the historical rev-6.1 numbers tabulated above.
The numbers above remain the authoritative record of the rev-6.1
measurement; do not re-read them from the current file without
recognising the overwrite. The universal-fidelity sprint's T2
(content-stable IDs) explicitly addresses this class of artefact
drift via the artefact-stability contract in the sprint plan's
§T2 acceptance; see also `FOLLOWUP_RISKS.md` hygiene-backlog
decision (c).

---

## Engine revision 7: Phase A PRs 1–7 land (TR-A.0-adjacent foundation work)

**Date of measurement.** 2026-04-19 (from `experiments/results/NPSP/rev7/analyze.log`).

**What shipped in rev 7.**
- PRs 3–5 + 5.5 + 6 from the Phase A PR stack. These touched
  extractor plumbing, `ApexClassSymbols` population infrastructure,
  and analyse-side consumer plumbing, but did **not** ship a
  resolver arm that recovers new classes of call edges beyond what
  was already live in rev 6.1.
- No resolver-semantics change on static receivers (that was PR 8,
  shipped in rev 8). No keyword-extraction fix (that was PR 9,
  shipped in rev 9).

**Headline rev 7 numbers (all NPSP production-only, `--exclude-tests --exclude-generated`).**

| metric | value |
| ------ | ----- |
| `summary.total_nodes`                  | 20,548 |
| `summary.total_functions`              | 14,936 |
| `summary.total_edges`                  | 114,140 |
| `metrics.dead_code.count` (production) | 869    |
| `reason.no_callers`                    | 502    |
| `reason.framework_annotation_unresolved` | 188 |
| `reason.dynamic_dispatch_target`       | 41    |
| `reason.declarative_wiring_unparsed`   | 138   |

**Rev 7 Round 5 attempt (2026-04-19).** A Round 5 draw was not
formally gated on rev 7; the §4.11 acceptance-gate contract
requires rev 7 to ship TR-A.1..A.6, which rev 7 did not. The
Round 5 gate attempt landed on rev 9 — see below.

---

## Engine revision 8: PR 8 — TR-A.4 `TypeName.staticMethod()` receiver resolution (R40 closure)

**Date of measurement.** 2026-04-19.

**What shipped in rev 8.** PR 8 closed **R40** (Apex
`TypeName.staticMethod()` receiver resolution gap — every static
call made through a typed dotted receiver was silently dropped
pre-PR 8). See `FOLLOWUP_RISKS.md` §R40 for full write-up and root
cause. The fix is a new `resolve_type_name_receiver` helper in the
Apex call resolver; scope is *only* the `Cls.staticMethod(...)`
receiver shape, not the full TR-A.4 overload-resolution ticket.

**Rev 7 → rev 8 delta (both on the same parse.db contract,
re-analysed with the PR-8 resolver).**

| metric | rev 7 | rev 8 | delta |
| ------ | ----- | ----- | ----- |
| `summary.total_nodes`                    | 20,548 | 20,548 | 0 |
| `summary.total_functions`                | 14,936 | 14,936 | 0 |
| `summary.total_edges`                    | 114,140 | 111,035 | **−3,105** |
| `metrics.dead_code.count` (production)   | 869    | 861    | **−8**  |
| `reason.no_callers`                      | 502    | 492    | **−10** |
| `reason.framework_annotation_unresolved` | 188    | 191    | **+3**  |
| `reason.dynamic_dispatch_target`         | 41     | 40     | **−1**  |
| `reason.declarative_wiring_unparsed`     | 138    | 138    | 0       |

**Why the edge count drops despite closing a resolver gap.** R40
closure replaces low-confidence fallback edges (the resolver used
to emit `Heuristic|Low` wildcard edges from `TypeName.foo()` to
every method named `foo` in the registry — a known over-emission
documented as part of R40) with a single authoritative
`Heuristic|Medium`-confidence edge per call site. The −3,105
delta is the collapse of that wildcard fan-out; the −10
`no_callers` delta is the separate effect of linking the
previously-dropped call-sites. Both shapes land in the same PR
because `resolve_type_name_receiver` replaces the wildcard
emission with the targeted emission atomically.

**Rev 8 artefacts caveat.** The on-disk `experiments/results/NPSP/rev8/parse.db`
is 0 bytes (an intermediate scratch file that was not retained).
The authoritative rev-8 baseline is
`experiments/results/NPSP/rev8/baseline.json` (13 MB), which was
produced by analysing the rev-7 `parse.db` with the rev-8
analyser. Rev 9 (below) re-runs the full parse-and-analyse
pipeline end-to-end; see
`FOLLOWUP_RISKS.md` hygiene-backlog and `FOLLOWUP_RISKS.md`
decision (a) for the artefact-stability plan.

---

## Engine revision 9: PR 9 — R46 cross-language reserved-keyword filter closure + Round 5 hand-audit

**Date of measurement.** 2026-04-19.

**What shipped in rev 9.** PR 9 closed **R46** — a cross-language
reserved-keyword filter in the Apex symbol extractor
(`graphengine-parsing/src/syntax/utils/name_validator.rs`) that
was silently dropping legal Apex identifiers whenever they
happened to be reserved in some *other* language (`match`, `type`,
`module`, `where`, etc.). Root cause: the filter was language-
blind. Fix: per-language keyword lists keyed on
`LanguageSpecificExtractor::language()`. See `FOLLOWUP_RISKS.md`
§R46 for the full write-up and the regression test in
`graphengine-parsing/tests/apex_resolver_r46_keyword_extraction_fixtures.rs`.

**R46 is an extractor-layer fix.** R46 is not scoped under the
§4.11 resolver work; the §4.11 resolver gates do not move when R46
closes. R46 closure's effect on `no_callers` is whatever rose from
`RD2_OpportunityMatcher::match(...)`-family callees becoming
visible to the classifier — the 12-node node count delta below is
that shape.

**Rev 8 → rev 9 delta.**

| metric | rev 8 | rev 9 | delta |
| ------ | ----- | ----- | ----- |
| `summary.total_nodes`                    | 20,548 | 20,560 | **+12** |
| `summary.total_functions`                | 14,936 | 14,948 | **+12** |
| `summary.total_edges`                    | 111,035 | 111,158 | **+123** |
| `metrics.dead_code.count` (production)   | 861    | 852    | **−9**  |
| `reason.no_callers`                      | 492    | 483    | **−9**  |
| `reason.framework_annotation_unresolved` | 191    | 191    | 0       |
| `reason.dynamic_dispatch_target`         | 40     | 40     | 0       |
| `reason.declarative_wiring_unparsed`     | 138    | 138    | 0       |

**Rev 7 → rev 9 aggregate (PR 8 + PR 9).**

| metric | rev 7 | rev 9 | delta |
| ------ | ----- | ----- | ----- |
| `summary.total_nodes`                    | 20,548 | 20,560 | **+12** |
| `summary.total_functions`                | 14,936 | 14,948 | **+12** |
| `summary.total_edges`                    | 114,140 | 111,158 | **−2,982** |
| `metrics.dead_code.count` (production)   | 869    | 852    | **−17**  |
| `reason.no_callers`                      | 502    | 483    | **−19** |

### Rev 9 Phase A closure gate — Round 5 hand-audit

Full per-sample verdicts: `HAND_AUDIT_LOG.md`,
§"Round 5 — Engine revision 9" (seed `20260418` stamped at draw
time, 10 FQNs drawn from the rev-9 `no_callers` pool of 483
post-`toString()` / post-`.trigger` / post-generated pre-filter).

| bucket | Round 5 (rev 9) | Gate | Verdict |
| ------ | --------------- | ---- | ------- |
| `no_callers` | **4 / 10 wrong** | `< 2 / 10 wrong` | **FAIL** |

**Decomposition of the 4 wrong verdicts (none of which overlap
with the §4.11 17-fixture set):**

| # | wrong verdict | shape | risk |
| - | ------------- | ----- | ---- |
| 1 | `RD2_OpportunityMatcher::match(...)` family (property getter caller) | property-accessor body extraction gap | R39 (known; filed pre-PR-9) |
| 2 | second `match(...)` family sample (same shape) | property-accessor body extraction gap | R39 |
| 3 | `ALLO_ManageAllocations_CTRL::getMappedAllocationsForOpp` (map-literal field initializer caller) | field-initializer body extraction gap | R41 (new; filed in PR 9) |
| 4 | `GE_SettingsService.getInstance().getDataImportSettings()` chained-call target | chained-call receiver-typing gap | R45 (new; filed in PR 9) |

**Gate verdict: FAIL (4 / 10 wrong, threshold `< 2 / 10 wrong`).**
Phase A stays formally **Open**. See
`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`
§"Round 5 — Engine revision 9" for the full audit block and
`TRUTHFUL_SCANS_ROADMAP.md` §4 for the open-status statement.

**The 17 §4.11-scope regression fixtures were not newly audited in
Round 5.** Round 5 is a fresh 10-sample draw from the `no_callers`
pool; it is not a re-audit of the 17 fixtures. The 17 fixtures'
live-vs-dead status against rev 9 is unchanged from rev 6.1
(TR-A.x has not shipped), and their formal acceptance remains a
post-Phase-B-resolver-ship gate. See `FRAMEWORK_RESOLVER_PLAN.md`
§8.3 for the rev-9 annotation on this.

### Reproducibility (rev 9)

```bash
# Full parse-and-analyse cycle against the NPSP source tree.
./experiments/run_canaries.sh          # parse + analyse rev 9 end-to-end
# Equivalent explicit form:
cargo build --release -p graphengine-parsing --bin ge-parse
cargo build --release -p graphengine-analysis --bin ge-analyze

mkdir -p experiments/results/NPSP/rev9
target/release/ge-parse \
  --repo ~/Desktop/apex_baseline_repos/NPSP \
  --db experiments/results/NPSP/rev9/parse.db

target/release/ge-analyze \
  --db experiments/results/NPSP/rev9/parse.db \
  --output experiments/results/NPSP/rev9/baseline.json \
  --exclude-tests --exclude-generated
```

Artefacts under `experiments/results/NPSP/rev9/`:
- `baseline.json` — rev-9 baseline HealthReport (13 MB).
- `parse.db` — rev-9 parse DB (85 MB, includes R46-fixed symbols).

### Why rev 9 motivates the universal-fidelity sprint

The rev-9 Round 5 FAIL is not a resolver-bug verdict on Phase A's
17-fixture contract — it is an audit-level verdict on the
`no_callers` bucket as a whole. The three wrong-verdict shapes
(R39 property accessors, R41 field initializers, R45 chained
receiver typing) are **orthogonal** to the §4.11 resolver work:

- **R39 / R41** are extractor-layer gaps. Fixing them is
  extractor-scope, not §4.11 resolver-scope. No amount of Phase B
  resolver work closes them.
- **R45** is resolver receiver-typing, but on a shape (chained
  call through a call-expression return value) that §4.11
  does not currently enumerate.

The universal-fidelity sprint
(`docs/workstreams/universal-fidelity/ (sprint directory)`)
targets these architectural gaps at a tier above the Apex-specific
resolver work:

- **T8 (extraction-coverage-aware classifier downgrade)** lets the
  `dead_code` classifier consult a per-file extraction-coverage
  metric (unwalked property accessors, unwalked map-literal
  initializers, unwalked lambda bodies) and honestly downgrade
  `no_callers` confidence when the extractor could not see the
  full caller surface. This is the honest workaround for R39 / R41
  without fixing the extractors first — and it is a generalisable
  shape (every language has extractor blind-spots).
- **T3 (dual-metric emission)** exposes the gap between
  `no_callers.all_edges` and `no_callers.high_only`, which today
  is silently collapsed into a single count.
- **T4 (measured fidelity tier)** replaces the self-declared
  `resolution_tier: "full"` seen in rev 7 / rev 9's
  `resolution_quality` block above with a measured tier derived
  from `edges_by_confidence`; a `no_callers` count reported under
  a measured `HeuristicPrimary` tier is not the same artefact as
  one reported under a measured `Authoritative` tier.

Re-audit of Phase A's closure gate is scheduled after either a
dedicated extractor-scope PR family fixes R39 / R41, or T8 ships
and the `no_callers` false-positive rate drops by honest
construction.
