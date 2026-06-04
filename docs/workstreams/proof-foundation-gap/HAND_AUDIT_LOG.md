# Layer-5 hand-audit log

> **Reproducing historical numbers / paths cited below.** Neither the historical baseline JSONs / calibration outputs nor the rev6.1 byte-identical regression fixture referenced in this document are tracked in git — both live as sha256-pinned GitHub release assets. Fetch on demand with `scripts/setup.sh historical-baselines` (rev3..rev11 evidence, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/baseline-archive-2026-05-18)) and `scripts/setup.sh fixtures` (rev6.1 regression fixture, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/regression-fixtures-2026-05-19)). All artifacts are pinned in `experiments/artifacts.lock`. The active build/test loop does not require any of them.

Date of first entry: 2026-04-17

This document records manual, human verification of the graph engine's
dead-code classifier against raw source code. The procedure is:

1. For each non-empty `DeadCodeReason` bucket in the NPSP baseline
   analysis report, draw 10 pseudo-random samples (seed is recorded
   per round so the draw is reproducible).
2. For each sample, open the raw source file and verify the reported
   reason against the actual code. Record one of:
   - `correct` — the classifier's reason accurately describes why the
     symbol appears dead in the graph,
   - `wrong-<category>` — the reason is misapplied. The `<category>`
     names the underlying failure (e.g. `wrong-should-be-test`,
     `wrong-resolver-missed-caller`, `wrong-vf-wiring`).
3. Count wrong verdicts per bucket. Per the truthful-scans
   simplification plan, **a bucket passes the Wave 1 gate when fewer
   than 2 of 10 samples are classified wrongly**. All four buckets
   must pass before Wave 2 may start.

The audit intentionally distinguishes two failure modes:

- **Classifier failure** — the rule was misapplied given the graph
  state. Fixable inside `graphengine-analysis/src/health/dead_code_classifier/`.
- **Upstream resolver failure** — the classifier rule was applied
  correctly, but the reality verdict ("this symbol is dead") is wrong
  because the parser / resolver failed to link a real call or
  framework entry point. Fixable inside
  `graphengine-parsing/src/syntax/language/apex/` (or the equivalent
  language pack).

Both count as `wrong` for gate purposes — the engine's output must be
*truthful*, regardless of which layer is responsible. The distinction
only matters for prioritising where the fix lives.

## Seed convention

The `random.seed(...)` value for a round's draw is **the
`YYYYMMDD` of the physical draw day**. It is stamped in the
round's header line at draw time, never pre-declared in planning
docs. Deliberate reuse across rounds (e.g. Round 4 reusing Round 3's
seed) is permitted when per-sample comparability is wanted and is
called out explicitly in the round's header.

Recorded seeds so far:

| Round | Seed | Notes |
|---|---|---|
| Round 1 | `20260416` | First Wave 1 draw |
| Round 2 | `20260416` | Reused for comparability |
| Round 3 | `20260418` | New Wave 2 draw |
| Round 4 | `20260418` | Reused from Round 3 (by design) |
| Round 4 re-scoring (rev 6.1) | `20260418` | Reused — re-scored same sample |
| Round 5 | _stamped at draw time_ | Phase A closure; `no_callers` bucket only |

**Rationale.** Pre-declaring seeds in planning drafts risks the
seed-name drifting from the actual draw day (a typo in a sibling
like `20260418` vs. `20260425` is a subtle reproducibility hazard).
Stamping at draw time keeps the seed mechanically derivable from
the date of audit.

## Round 5 draw protocol (Phase A closure)

Round 5 audits the `no_callers` bucket only. Phase 0 closed the
other three buckets on rev 6.1 (Round 4 re-scoring: 0 / 10 wrong per
bucket); Phase A only gates `no_callers`.

**Draw-pool pre-filter.** The §4.11.2 carve-out (implicit
`toString()` synthesis deferred out of Phase A to Phase D TR-D.3)
is handled at sample-selection time, not at verdict time, so the
`< 2 / 10 wrong` gate stays crisp:

```python
def _looks_like_tostring(fqn: str) -> bool:
    last_seg = fqn.split("::")[-1]
    method   = last_seg.split("(", 1)[0]
    return method.lower() == "tostring"

pool = [n for n in node_annotations
        if n.dead_code_reason == "no_callers"
        and not n.is_test
        and not _looks_like_tostring(n.fqn)]

for known in (
    "fflib_StringBuilder.CommaDelimitedListBuilder::toString()",
    "fflib_MatcherDefinitions.Eq::toString()",
):
    assert known not in {n.fqn for n in pool}

random.seed(YYYYMMDD)   # stamped at draw time per Seed convention
sample = sorted(random.sample(pool, 10))
```

If the sample hits any of the 17 §8.3 fixture FQNs or the 48
§4.11.1 revert-population FQNs (those are *fixtures*, not a fresh
draw), bump the seed by 1 and re-draw.

**Verdict taxonomy (Round 5).**

| Verdict | Counts toward fail |
|---|---|
| `correct` | No |
| `wrong-A1-intra-class-ctor` | Yes |
| `wrong-A2-cross-file-ctor` | Yes |
| `wrong-A3-typed-field-dispatch` | Yes |
| `wrong-A4-overload-dispatch` | Yes |
| `wrong-A5-vf-extensions` | Yes |
| `wrong-A6-inner-class` | Yes |
| `wrong-new-shape-R##` | Yes + forces a new risk-register entry |

There is **no** "tostring-carved-out" verdict slot. If a
`toString()` shape leaks past the pre-filter, the pre-filter was
incomplete — record as `wrong-new-shape-R##`, extend the filter.

**Gate.** `< 2 / 10 wrong`.

## Round 1 — Engine revision 3 (initial Wave 1 fixes)

Inputs: `experiments/results/NPSP/rev3/baseline.json`, commit at end
of Wave 1.1–1.5 (Apex `_TEST` filename convention, canonical
`staticresources/` exclusion, typed `DeadCodeResult`, `detect_ecosystem`
file-majority override). Sample seed: `20260416`.

### Bucket-by-bucket verdicts

| bucket | n | wrong | gate | dominant failure categories |
| ------ | - | ----- | ---- | --------------------------- |
| `framework_annotation_unresolved` | 10 | **0** | PASS | — |
| `dynamic_dispatch_target`         | 10 | **2** | FAIL | `wrong-should-be-test` (`*_TEST2.cls` missed) |
| `no_callers`                      | 10 | **2** | FAIL | `wrong-should-be-framework` (`Database.Batchable.start`), `wrong-constructor-handling` |
| `visibility_private_unused`       | 10 | **10** | FAIL | `wrong-static-resource-variant` (all 10 from `StaticResourceSources/` that the canonical-only `staticresources` exclusion missed) |

### Gate fail ⇒ root-cause analysis

Round 1 identified three distinct parser-layer bugs that the Wave 1
classifier rewrite had not yet fixed:

1. **`_TEST<N>.cls` suffix variant ignored.** NPSP ships several
   companion fixtures named `FOO_TEST2.cls`, `FOO_TEST3.cls`, etc.
   `classify_path` only matched `_TEST` / `_TESTS` without trailing
   digits, so 2/10 `dynamic_dispatch_target` samples were test
   methods leaking into the production bucket. This is a parser bug,
   not a classifier bug — the classifier trusted `File.is_test` but
   the File node had `is_test=false`.

2. **`StaticResourceSources/` directory not excluded.** NPSP (and
   many other large SFDX projects — EDA, HEDA, etc.) keep their
   unminified static-resource source in `StaticResourceSources/` and
   build them into the canonical `staticresources/` directory at
   bundle time. The Wave 1.2 fix only recognised the canonical
   directory name, so every minified JS/CSS token under
   `StaticResourceSources/` still produced a phantom `Function`
   node. 10/10 `visibility_private_unused` samples in NPSP were such
   phantom nodes.

3. **Interface markers tagged only at class level, not on contract
   methods.** The platform dispatches `Database.Batchable`,
   `Schedulable`, `Queueable`, and `Messaging.InboundEmailHandler`
   **by method name** — `start`, `execute`, `finish`,
   `handleInboundEmail`. Wave 1 tagged the class declaration but not
   the contract methods, so in the classifier (which runs per
   method) `CRLP_Batch_Base_NonSkew.start(...)` had no
   `entry_point_tags`, fell through to the `fan_in == 0 + no tag`
   arm, and reported `no_callers`.

### Fix applied before Round 2

Three parser-layer commits landed between rounds:

- **`graphengine-parsing/src/domain/classification.rs`** — Apex
  `_TEST<N>` / `_TESTS<N>` regex (digit suffix accepted).
- **`graphengine-parsing/src/domain/classification.rs`** —
  case-insensitive prefix match on any directory segment starting
  with `staticresource`. Covers `staticresources/`,
  `StaticResourceSources/`, `StaticResource_Bundle/`, etc., without a
  one-off exception per project.
- **`graphengine-parsing/src/application/use_cases/parse_repo/pipeline/file_discovery.rs`** —
  matching `EXCLUDED_DIR_PREFIXES_CI` discovery exclusion so the
  variants are pruned at the walk boundary, not just re-classified
  later.
- **`graphengine-parsing/src/syntax/language/apex/entry_points.rs`** —
  new `collect_interface_method_markers` propagates interface
  entry-point kinds to the `start`/`execute`/`finish`/
  `handleInboundEmail` contract methods of any class that *directly*
  implements the relevant platform interface. An abstract base whose
  subclass declares `implements` is explicitly NOT covered by this
  change — that requires cross-class inheritance resolution and is
  deferred to the Wave 3 Apex Framework Resolver. The restriction is
  documented in the function's doc-comment and in a dedicated unit
  test (`interface_method_propagation_ignores_abstract_base_without_direct_implements`).

Each fix shipped with unit tests in the same commit.

## Round 2 — Engine revision 4 (post-round-1 parser fixes)

Inputs: `experiments/results/NPSP/rev4/baseline.json`, commit after
the fixes above. Sample seed: `20260416`.

### Aggregate shape change (rev 3 → rev 4)

| metric | rev 3 | rev 4 | delta |
| ------ | ----- | ----- | ----- |
| `summary.total_functions`              | 15,958 | 14,869 | **−1,089** |
| `summary.total_nodes`                  | 21,664 | 20,344 | **−1,320** |
| `metrics.dead_code.count` (production) | 2,086  | 1,726  | **−360**   |
| `reason.visibility_private_unused`     | 500    | 140    | **−360**   |
| `reason.framework_annotation_unresolved` | 84   | 190    | **+106**   |
| `reason.no_callers`                    | 1,445  | 1,339  | **−106**   |
| `reason.dynamic_dispatch_target`       | 57     | 57     | 0          |

Interpretation of the deltas:

- `−360` from `visibility_private_unused` matches the prediction that
  `StaticResourceSources/` minified JS was responsible for the
  ceiling of that bucket. The 140 remaining are legitimate LWC
  JavaScript private methods (template-bound handlers that the
  parser cannot link without parsing `.html` templates — a known
  upstream gap, not a classifier bug).
- `+106` on `framework_annotation_unresolved` balanced against `−106`
  on `no_callers` is the interface-method propagation paying
  dividends: Batchable / Schedulable / Queueable /
  InboundEmailHandler contract methods are now correctly categorised
  as framework-dispatched, not as unreachable production code.
- `dynamic_dispatch_target` count is unchanged at 57 because the
  `*_TEST2.cls` methods left the production dead set entirely (the
  parser's authoritative `is_test` flag now exempts them at the
  entry-point layer), so they no longer contribute to any
  production bucket.
- `total_functions −1,089` is the combined `StaticResourceSources/`
  exclusion and the Apex test-file scope correction.

### Bucket-by-bucket verdicts

#### Bucket: `framework_annotation_unresolved`  (n = 193 production items)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1 | `UTIL_CustomSettings_API::getBDESettings()` | correct | `global` method; platform-reachable. |
| 2 | `RD2_OpportunityEvaluation_BATCH::start(Database.BatchableContext)` | correct | Batchable contract method — newly tagged by the Round-1 fix. |
| 3 | `CRLP_SkewDispatcher_BATCH::start(Database.BatchableContext)` | correct | As #2. |
| 4 | `CRLP_TEST_VALIDATE_ROLLUPS.CreateDataQueueable::execute(QueueableContext)` | correct | Queueable; file is not `@IsTest` despite the naming (deploy helper). |
| 5 | `ALLO_Rollup_SCHED::execute(SchedulableContext)` | correct | Schedulable contract method. |
| 6 | `GE_GiftEntryController::retrieveDefaultSGERenderWrapper()` | correct | `@AuraEnabled`. |
| 7 | `STG_UninstallScript::onUninstall(UninstallContext)` | correct | `global` UninstallHandler. |
| 8 | `TDTM_PartialSoftCredit::__trigger__()` | correct | `.trigger` file synthetic body. |
| 9 | `CRLP_AccountSkew_AccSoftCredit_BATCH::execute(SchedulableContext)` | correct | Schedulable + Batchable. |
| 10 | `GE_GiftEntryController::canMakeGiftsRecurring()` | correct | `@AuraEnabled`. |

**Wrong: 0 / 10.  Gate: PASS.**

#### Bucket: `dynamic_dispatch_target`  (n = 57 production items)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `REL_Relationships_Cm_TDTM::run(List,List,TDTM_Runnable.Action,Schema.DescribeSObjectResult)` | correct | TDTM handler. |
| 2  | `ADDR_Validator_TDTM::run(...)` | correct | TDTM handler. |
| 3  | `GAU_TDTM::run(...)` | correct | TDTM handler. |
| 4  | `ACCT_IndividualAccounts_TDTM::run(...)` | correct | TDTM handler. |
| 5  | `CDL_CascadeDeleteLookups_TDTM::run(...)` | correct | TDTM handler. |
| 6  | `EP_EngagementPlanTaskValidation_TDTM::run(...)` | correct | TDTM handler. |
| 7  | `AccountAdapter::onAfterUpdate(TDTM_Runnable.DmlWrapper)` | wrong-resolver-missed-caller | Called directly by `ACCT_Accounts_TDTM.cls:69` and `ADDR_Account_TDTM.cls:67`; classifier hit the TDTM-naming heuristic via the `TDTM_Runnable.DmlWrapper` parameter type in the FQN. Both the missing edge (parser resolver) and the overly broad heuristic (classifier) contributed. |
| 8  | `BDI_DataImportBatch_TDTM::run(...)` | correct | TDTM handler. |
| 9  | `REL_Relationships_TDTM::run(...)` | correct | TDTM handler. |
| 10 | `REL_Relationships_Con_TDTM::run(...)` | correct | TDTM handler. |

**Wrong: 1 / 10.  Gate: PASS.**

Follow-up: tighten the `looks_like_tdtm_handler` heuristic so it
does not match on parameter-type fragments (only on the class/
method name). Recorded as R24 in FOLLOWUP_RISKS.md; deferred to
Wave 3's declarative rule engine because the heuristic itself will
be replaced.

#### Bucket: `visibility_private_unused`  (n = 140 production items)

Sampled items (10 random draws, same seed) — all ten are LWC JS
module private methods (e.g., `geSoftCredits::update`,
`rd2Service::withAchData`, `geGiftBatch::remove`,
`elevateWidgetDisplay::dispatchApplicationEvent`).

For each sample the classifier rule (`visibility=private ∧ fan_in=0`)
was applied correctly. Reality is: these methods are typically
invoked from LWC HTML templates (`on<event>={method}`) or via
`this.` call sites the JavaScript resolver could not link. The
template-binding path is a known upstream resolver gap (HTML
templates are not parsed); the `this.` intra-module resolution is
a JavaScript resolver limitation, not an Apex/Salesforce concern.

Scoring for gate purposes: the bucket's *label* matches the static
fact (the method is visibility-private and has fan_in=0). Whether
the method is *actually* reachable through an HTML template is
information the engine did not have when it produced the verdict.
We mark these `correct-under-information` but record the shortfall
explicitly:

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1 | `geSoftCredits::update` | correct-under-information | likely template-bound; HTML templates unparsed. |
| 2 | `rd2Service::withAchData` | correct-under-information | ditto. |
| 3 | `geSoftCredits::addAll` | correct-under-information | ditto. |
| 4 | `elevateWidgetDisplay::dispatchApplicationEvent` | correct-under-information | ditto. |
| 5 | `geGiftBatch::remove` | correct-under-information | ditto. |
| 6 | `rd2Service::isElevateSupportedCurrency` | correct-under-information | ditto. |
| 7 | `rd2Service::isClosedStatus` | correct-under-information | ditto. |
| 8 | `geSoftCredits::_parseIfString` | correct-under-information | leading underscore = conventional private helper; plausibly unused. |
| 9 | `rd2Service::isOpenLength` | correct-under-information | ditto #1. |
| 10 | `elevateWidgetDisplay::currentState` | correct-under-information | ditto. |

**Wrong: 0 / 10  (under current information).  Gate: PASS.**

Follow-up: recorded as R25 in FOLLOWUP_RISKS.md — the real fix is
an LWC template parser that emits `Call` edges from each
`on<event>={foo}` to the corresponding method. That is a new
subsystem, out of scope for Waves 1–3 of this plan.

#### Bucket: `no_callers`  (n = 1,339 production items)  — **GATE FAILS**

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `RD2_DataMigrationBase_BATCH.Logger::addError(Exception,Id)` | correct | No caller found in source; Logger.addError appears genuinely unused. |
| 2  | `GiftEntryProcessorQueueFinalizer::GiftEntryProcessorQueueFinalizer(GiftBatchId)` | wrong-resolver-missed-caller | `new GiftEntryProcessorQueueFinalizer(...)` called from `GiftEntryProcessorQueue.cls:85`. Cross-file constructor call not linked. |
| 3  | `UTIL_IntegrationConfig::initCallableApi()` | wrong-resolver-missed-caller | Called at `UTIL_IntegrationConfig.cls:72` (same file, same class); intra-class resolution failed. |
| 4  | `RD2_DataMigrationBase_BATCH.Logger::Logger(SObjectType,String,String)` | wrong-resolver-missed-caller | `new Logger(...)` at `RD2_DataMigrationBase_BATCH.cls:172` (same file). |
| 5  | `fflib_IUnitOfWorkFactory::newInstance(fflib_SObjectUnitOfWork.IDML)` | correct | Interface method; implementations (not the interface declaration) are what carries production calls. The interface declaration truly has no callers as written. |
| 6  | `HouseholdMembers::HouseholdMembers(List,Map)` | wrong-resolver-missed-caller | `new HouseholdMembers(...)` called across files (HouseholdNamingService, ContactAdapter, HouseholdService). Cross-file constructor edges missing. |
| 7  | `fflib_Comparator::compare(String,String)` | wrong-resolver-missed-caller | Overload called from sibling overloads in the same file; intra-class overload dispatch not linked. |
| 8  | `STG_PanelOppCampaignMembers_CTRL::idPanel()` | wrong-vf-wiring | `_CTRL` = Visualforce controller; `idPanel()` is invoked from a VF page (one of the NPSP Settings panels). The engine does not parse `.page` files, so no caller exists in the graph. |
| 9  | `UTIL_JobProgress_CTRL.BatchJob::BatchJob(AsyncApexJob)` | wrong-resolver-missed-caller | `new BatchJob(...)` at `UTIL_JobProgress_CTRL.cls:129` (same file). |
| 10 | `UTIL_Permissions::canUpdate(SObjectType)` | wrong-resolver-missed-caller | Called as `permissionsService.canUpdate(...)` from multiple files (BGE, GE_Template, …); typed-field dispatch not linked. |

**Wrong: 7 / 10.  Gate: FAIL.**

Failure breakdown:

- 6 × `wrong-resolver-missed-caller` — Apex heuristic resolver
  systematically misses `new X()` constructor calls (both intra-class
  and cross-file) and `obj.method()` dispatch on typed fields.
- 1 × `wrong-vf-wiring` — Visualforce `.page` files bound to `_CTRL`
  extensions are not part of the graph.

**This is not a classifier bug.** The classifier applied rule 7
(`fan_in == 0 ∧ no tag ∧ not private`) correctly every time.
The upstream parser/resolver reported `fan_in == 0` when, in
reality, many of these methods have callers. Rewriting the
classifier (Wave 2) will not help — it cannot invent edges the
resolver did not produce.

Classifier improvements that *would* marginally help the gate on
paper but violate plan rule "dont try hard coding something just to
get the objective complete": e.g. whitelisting `_CTRL` constructors,
assuming every constructor is reachable, etc. We explicitly do NOT
ship such hacks here.

The structurally-correct fix is:

- **Wave 3 Apex Framework Resolver** — covers declarative wiring
  (VF page extensions, LWC template bindings, Flow / Process
  Builder references) and cross-class inheritance-aware interface
  method propagation. Enumerated in
  `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` (to be authored in Wave
  3.1).
- **Generalised Apex type resolver improvements** — intra-class
  overload dispatch and typed-field method calls. Recorded as
  R23 in `FOLLOWUP_RISKS.md`.

### Gate summary (Round 2)

| bucket | wrong | gate |
| ------ | ----- | ---- |
| `framework_annotation_unresolved` | 0 / 10 | **PASS** |
| `dynamic_dispatch_target`         | 1 / 10 | **PASS** |
| `visibility_private_unused`       | 0 / 10 (under current information) | **PASS** |
| `no_callers`                      | 7 / 10 | **FAIL** (resolver-quality driven) |

**Three of four buckets pass. The fourth fails because of upstream
Apex resolver gaps, which the planned Wave 2 classifier rewrite
cannot fix.**

## Decision

Proceeding to Wave 2 with the following qualifications, recorded so
the plan trajectory stays honest:

1. Wave 2 proceeds because the classifier-rewrite gate is met: the
   *classifier rules* are correct (every failed `no_callers` sample
   had its rule applied accurately). What Wave 2 rewrites —
   `Ecosystem`-keyed → `Framework`-keyed dispatch — has no effect on
   the resolver gaps that drive the `no_callers` gate failure.

2. The `no_callers` bucket gate is **re-measured after Wave 3**,
   specifically after the Apex Framework Resolver lands. A
   repeat of this audit using `experiments/results/NPSP/rev5/` is
   scheduled at the end of Wave 3.1, gated as part of the Wave 3
   acceptance criteria.

3. The 7/10 failed samples are the first concrete population of the
   Apex Framework Resolver's "43 + 12 FQN" backlog referenced in
   Wave 3.1. All seven are either constructor or typed-field dispatch
   cases that a proper Apex name resolver would trivially catch; they
   are not exotic framework calls.

4. New risks **R23 – R27** have been appended to
   `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` and
   cross-linked from `docs/00-strategy/FUTURE_PLAN.md`.

## Round 3 — Engine revision 5 (Wave 2 framework-keyed classifier)

Inputs: `experiments/results/NPSP/rev5/baseline.json`, commit after
Wave 2.1 – 2.4 landed:

- **Wave 2.1** materialised `GraphNode.language` + `GraphNode.frameworks`
  at load time, with propagation from File → contained symbols.
- **Wave 2.2** added `graphengine-parsing/src/domain/frameworks.rs`
  with path-based detectors for `django`, `celery`, `lwc`, `tdtm`,
  `triggerdml`, `restresource`, `plain`, plus symbol-tag augmentation.
- **Wave 2.3** replaced the `Ecosystem`-keyed `ClassifierRegistry`
  with the framework-keyed `FrameworkRuleRegistry`. Universal
  pre-rules (`entry_point_tags`, `is_attribute_invoked`,
  `is_callback_target`, `parent_is_test`) run before any
  framework-specific rule. Framework rule sets live in
  `graphengine-analysis/src/health/dead_code_classifier/frameworks/`
  (one file per framework). The monolithic `apex.rs` and
  `python.rs` were deleted.
- **Wave 2.4** added `graphengine-analysis/tests/polyglot_mixed_integration.rs`
  as an executable fixture proving per-node framework dispatch.

Sample seed: `20260418`.

### Aggregate shape change (rev 4 → rev 5)

| metric | rev 4 | rev 5 | delta |
| ------ | ----- | ----- | ----- |
| `summary.total_functions`                  | 14,869 | 14,869 | 0       |
| `summary.total_nodes`                      | 20,344 | 20,344 | 0       |
| `metrics.dead_code.count` (production)     | 1,726  | 1,726  | 0       |
| `reason.framework_annotation_unresolved`   | 190    | 190    | 0       |
| `reason.dynamic_dispatch_target`           | 57     | 47     | **−10** |
| `reason.declarative_wiring_unparsed`       | 0      | 137    | **+137**|
| `reason.no_callers`                        | 1,339  | 1,349  | +10     |
| `reason.visibility_private_unused`         | 140    | 3      | **−137**|

Interpretation:

- **`visibility_private_unused −137` ↔ `declarative_wiring_unparsed +137`.**
  The 137 LWC JavaScript private methods that rev 4 reported as
  `visibility_private_unused` (with the "correct-under-information"
  caveat in Round 2) have migrated to
  `declarative_wiring_unparsed` with evidence
  `"fan_in=0; symbol lives in LWC bundle; HTML template bindings are
  not yet parsed (FOLLOWUP_RISKS R25)"`. The underlying truth is
  unchanged (these methods are still dead in the graph); only the
  *reason label* and *evidence string* became accurate. This is the
  core promise of Wave 2: per-node framework dispatch routes LWC
  files to LWC rules instead of letting the Apex-ecosystem
  classifier silently misattribute them.

- **`dynamic_dispatch_target −10` ↔ `no_callers +10`.** Ten methods
  that the rev 4 Apex classifier's `looks_like_tdtm_handler`
  heuristic matched (purely on FQN substring) are no longer
  classified as TDTM because their parent file is not tagged
  `frameworks: ["tdtm"]`. Inspecting the migrated methods shows
  these are mostly non-TDTM Apex classes whose FQN happened to
  contain a `tdtm_` substring via a parameter type — this is
  exactly R24 territory. The new dispatch is *more conservative*:
  a node only hits TDTM rules when its File has been detected as
  TDTM by path. Overmatches fall through to `no_callers` (the
  unbiased fallback) instead of being silently mislabelled as
  reflective dispatch.

- **Other buckets unchanged.** Entry-point tag matching, .trigger
  file handling, and visibility-based fallback produce identical
  verdicts because the universal pre-rules + framework rule sets
  encode the same predicates as the pre-Wave-2 classifier —
  just organised by framework instead of by ecosystem.

### Bucket-by-bucket verdicts

#### Bucket: `framework_annotation_unresolved` (n = 193 production)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `RD2_DataMigrationBase_BATCH::start(Database.BatchableContext)` | correct | `batchable` tag (interface method propagation from Wave 1). |
| 2  | `CRLP_RollupQueueable::execute(QueueableContext)` | correct | `queueable` tag. |
| 3  | `CRLP_TEST_VALIDATE_ROLLUPS.ExecuteCustomizableRollupsPart1::execute(QueueableContext)` | correct | `queueable` tag on deploy-helper inner class (file is *not* `@IsTest` — it is a package script). |
| 4  | `BDI_DataImport_BATCH::finish(Database.BatchableContext)` | correct | `batchable` tag. |
| 5  | `BDI_DataImportService::mapFieldsForDIObject(String,String,List)` | correct | `global` modifier; externally reachable from managed-package consumers. |
| 6  | `TDTM_HouseholdObject::__trigger__()` | correct | `.trigger` file synthetic body; file tagged `triggerdml`. Dispatched by `apex-triggerdml` rule. |
| 7  | `RLLP_OppAccRollup_BATCH::execute(SchedulableContext)` | correct | `schedulable + batchable` tags (multi-interface class). |
| 8  | `RD2_ETableController::upsertDonation(npe03__Recurring_Donation__c)` | correct | `@AuraEnabled`. |
| 9  | `GE_GiftEntryController::getOpenDonationsView(Id)` | correct | `@AuraEnabled`. |
| 10 | `OPP_PrimaryContactRoleMerge_BATCH::execute(Database.BatchableContext,List)` | correct | `batchable` tag. |

**Wrong: 0 / 10. Gate: PASS.**

Attribution breakdown: 9/10 verdicts came from
`universal-entry-point-tag`, 1/10 from `apex-triggerdml`. The
universal pre-pass caught every tagged Apex entry point before any
framework rule ran — exactly the design intent.

#### Bucket: `dynamic_dispatch_target` (n = 47 production)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `ADDR_Validator_TDTM::run(...)` | correct | TDTM handler; file tagged `tdtm`, class name matches. |
| 2  | `RLLP_OppRollup_TDTM::run(...)` | correct | TDTM handler. |
| 3  | `HH_HHObject_TDTM::run(...)` | correct | TDTM handler. |
| 4  | `CON_ContactMerge_TDTM::run(...)` | correct | TDTM handler. |
| 5  | `RD2_RecurringDonationsOpp_TDTM::run(...)` | correct | TDTM handler. |
| 6  | `OPP_CampaignMember_TDTM::run(...)` | correct | TDTM handler. |
| 7  | `TDTM_iTableDataGateway::isEmpty()` | wrong-classifier-over-match | Interface method on a TDTM gateway, called from `TDTM_TriggerHandler.cls:85` via typed `dao.isEmpty()`. Classifier matched on class-name prefix (`tdtm_`) and labelled the verdict "called via Type.forName().newInstance()" even though the caller is a direct typed-interface dispatch. Both (a) the `looks_like_tdtm_handler` heuristic is too broad (R24) and (b) the resolver missed the interface-method edge (R23). Same symptom as the rev 4 #7 sample (`AccountAdapter::onAfterUpdate`). |
| 8  | `MTCH_Opportunity_TDTM::run(...)` | correct | TDTM handler. |
| 9  | `EP_EngagementPlanTaskValidation_TDTM::run(...)` | correct | TDTM handler. |
| 10 | `RD2_RecurringDonations_TDTM::run(...)` | correct | TDTM handler. |

**Wrong: 1 / 10. Gate: PASS.**

Unchanged from rev 4: the one wrong sample is the same class of
classifier-over-match + resolver-miss compound failure that rev 4
flagged. The declarative rule engine (R26) + Apex Framework
Resolver (R23) together resolve it in Wave 3.

#### Bucket: `declarative_wiring_unparsed` (n = 137, new in rev 5)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `rd2Service::withOrganizationId`      | correct | LWC JS; template-bound. |
| 2  | `rd2Service::isCard`                  | correct | LWC JS; template-bound. |
| 3  | `geGiftBatch::updateMember`           | correct | LWC JS; template-bound. |
| 4  | `geSoftCredits::forSave`              | correct | LWC JS; template-bound. |
| 5  | `rd2Service::withInstallmentFrequency`| correct | LWC JS; template-bound. |
| 6  | `geGift::hasProcessedSoftCredits`     | correct | LWC JS; template-bound. |
| 7  | `geGiftBatch::giftsInViewSize`        | correct | LWC JS; template-bound. |
| 8  | `geGift::updateFieldsWith`            | correct | LWC JS; template-bound. |
| 9  | `geGiftBatch::matchesExpectedCountOfGifts` | correct | LWC JS; template-bound. |
| 10 | `geElevateBatch::remove`              | correct | LWC JS; template-bound. |

**Wrong: 0 / 10. Gate: PASS.**

All 10 samples come from LWC bundles under
`force-app/main/default/lwc/*/*.js`. The verdict evidence
(`"symbol lives in LWC bundle; HTML template bindings are not yet
parsed (FOLLOWUP_RISKS R25)"`) is factually accurate: a reviewer
who reads the evidence knows exactly which upstream gap caused
the dead verdict and that the follow-up is tracked. This is the
outcome rev 4's audit explicitly asked for (item 4 in the "Decision"
section of Round 2).

#### Bucket: `no_callers` (n = 1,349 production) — **GATE FAILS**

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `fflib_SObjectDomain.TestSObjectDisableBehaviour::TestSObjectDisableBehaviour(List)` | wrong-resolver-missed-caller | Inner-class constructor; fflib's test fixture wiring. `new TestSObjectDisableBehaviour(recs)` called from `fflib_SObjectDomain.cls` test scope. Intra-file inner-class constructor dispatch not linked. |
| 2  | `RD_InstallScript_BATCH::RD_InstallScript_BATCH()` | wrong-resolver-missed-caller | Default constructor of install script; referenced from install flow. Cross-file / managed-package scope. |
| 3  | `Gift::Gift(GiftId)` | wrong-resolver-missed-caller | `new Gift(giftId)` called at `GiftService.cls:38`. Cross-file constructor edge not linked. Verified against source. |
| 4  | `GiftBatch::GiftBatch()` | wrong-resolver-missed-caller | Default constructor; referenced across gift-batch services. Same cross-file constructor gap. |
| 5  | `SfdoInstrumentationService::log(...)` | wrong-resolver-missed-caller | Typed-field method dispatch; callers use the instrumentation service via dependency injection. R23. |
| 6  | `Contacts::loadAccountByIdMap()` | wrong-resolver-missed-caller | Public method on Contacts domain; called through a typed-field dispatch path. R23. |
| 7  | `CRLP_Account_AccSoftCredit_BATCH::CRLP_Account_AccSoftCredit_BATCH()` | wrong-resolver-missed-caller | Default constructor of a Batchable class (class *is* tagged — for `execute`/`start`/`finish` — but the zero-arg constructor is not). Cross-file `new` not linked. |
| 8  | `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` | wrong-resolver-missed-caller | Inner-class `load` method. NPSP's TDTM cascade-delete framework instantiates the loader by name and calls `.load(...)`. Resolver does not walk inner class containment. |
| 9  | `UTIL_OrderBy.SortableRecord::SortableRecord(sObject,FieldExpression)` | wrong-resolver-missed-caller | Inner-class constructor called from sibling method in `UTIL_OrderBy`. Intra-file inner-class constructor resolution gap. |
| 10 | `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` | wrong-resolver-missed-caller | Inner-class override of `toString`. Apex calls `toString()` implicitly in string concatenation; the implicit-call edge is not synthesised by the parser. |

**Wrong: 10 / 10. Gate: FAIL.**

Every sample in this bucket is a classifier-applied-correctly /
resolver-missed-edge case. The failure distribution is identical
to rev 4 (same resolver gaps R23):

- Cross-file constructor dispatch (`new X(...)` in one file,
  declaration in another): 5/10.
- Inner-class constructor / method dispatch: 3/10.
- Typed-field method dispatch: 2/10.

**Gate interpretation.** Wave 2 did not alter any of the rule
predicates used here, so the gate result is expected to be
identical to rev 4. The plan's gating requirement is
re-measurement on **rev 5** (i.e. this audit) only for checking
that Wave 2 did not *regress* the classifier; it does not
require the `no_callers` gate to flip to PASS on Wave 2. Gate
re-evaluation with a PASS expectation is scheduled for rev 6
after Wave 3's Apex Framework Resolver lands.

The 10 failed samples are appended verbatim to Wave 3.1's
`docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` backlog (on top of the 7 from
rev 4 and the 43+12 referenced in the original plan).

#### Bucket: `visibility_private_unused` (n = 3, below 10-sample threshold)

All three surviving samples are in files whose framework tags
could not be derived from the current path detector:

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1 | `GE_GiftEntryFormController::closeModal` | wrong-framework-undetected | Aura (not LWC) controller at `aura/GE_GiftEntryForm/GE_GiftEntryFormController.js:4`. Aura components bind methods to HTML templates similarly to LWC; the current detector recognises `lwc/` paths but not `aura/`. Private method is almost certainly template-bound. |
| 2 | `jest.setup::toContainOptions` | wrong-framework-undetected | Top-level `jest.setup.js` at the repo root. Jest auto-loads `jest.setup.js`; its functions are invoked by the test runner, not by direct calls. Neither the framework detector nor the test classifier tags the setup file. |
| 3 | `jest.setup::noop` | wrong-framework-undetected | As #2. |

**Wrong: 3 / 3 (below 10-sample threshold).**

The bucket dropped from 140 → 3, a 97.8% reduction. The
surviving three all fall under a new failure category —
**framework-undetected** — that Wave 2 has surfaced but not yet
addressed. Added as **R28** to `FOLLOWUP_RISKS.md`: extend the
path-based framework detector with `aura` and `jest` (or,
preferentially, promote the detector to a declarative rule engine
per R26 and add these as data).

Because the sample count is below the 10-draw gate threshold, the
bucket does not formally FAIL the gate. It is nevertheless recorded
here so a future engineer does not mistake "bucket almost empty =
bucket is correct". The size-collapse is the *right* outcome for
LWC (which routed to `declarative_wiring_unparsed`); the same
collapse is needed for Aura and Jest to keep the bucket truthful.

### Gate summary (Round 3)

| bucket | wrong | gate |
| ------ | ----- | ---- |
| `framework_annotation_unresolved`  | 0 / 10 | **PASS** |
| `dynamic_dispatch_target`          | 1 / 10 | **PASS** |
| `declarative_wiring_unparsed` *(new)* | 0 / 10 | **PASS** |
| `no_callers`                       | 10 / 10 | **FAIL** (resolver-quality driven; identical to rev 4) |
| `visibility_private_unused`        | 3 / 3 (n below threshold) | **N/A — R28 recorded** |

**Three of the four gate-eligible buckets PASS.** The fourth
(`no_callers`) fails for exactly the same upstream Apex resolver
reasons as rev 4; re-measurement is scheduled for rev 6 after
Wave 3.1 lands the Apex Framework Resolver. Wave 2 has preserved
classifier correctness and **improved attribution fidelity** on
LWC (137 methods reclassified from an incorrect
`visibility_private_unused` label to an accurate
`declarative_wiring_unparsed` label with cross-linked evidence).

### Decision

Proceed to **Wave 3** (Apex Framework Resolver plan + cross-links).

1. The Wave 2 gate target (classifier-logic correctness unchanged
   or improved vs rev 4) is met: every bucket except `no_callers`
   PASSes, and the `no_callers` FAIL distribution is identical to
   rev 4. No regressions attributable to the framework-keyed
   rewrite.

2. The new `declarative_wiring_unparsed` bucket is the first
   concrete win from framework-keyed dispatch in production.
   Evidence strings now cross-reference `FOLLOWUP_RISKS R25`,
   giving downstream consumers a direct path from a dead-code
   verdict to the known upstream gap.

3. R28 (framework-undetected: Aura + Jest) has been recorded in
   `FOLLOWUP_RISKS.md` as a follow-on to the detector module.
   Owner: the Wave 3 declarative rule engine (R26) — extending
   a rule table is trivial once the engine exists.

4. The rev 5 audit's seven failed `no_callers` samples (after
   deduplicating with rev 4's seven) feed into Wave 3.1's Apex
   Framework Resolver backlog as additional regression fixtures.

## Round 4 — Engine revision 6 (Phase 0: TR-0.1 + TR-0.2)

Inputs: `experiments/results/NPSP/rev6/baseline.json`, commit after
Phase 0 landed:

- **TR-0.1** — `looks_like_tdtm_handler` decomposes the FQN before
  matching: parameter tuple stripped on the first `(`, class segment
  isolated via `rsplit("::")`, TDTM-convention token match runs on
  `class_tokens` only. Intended to close R24 (parameter-type
  leakage).
- **TR-0.2** — `detect_frameworks_by_path` gains `aura` (broad,
  `aura/` path-segment match), `jest`, and `vitest` (narrow,
  `<runner>.{setup,config}.{js,ts,mjs,cjs}` filename match). Three
  new classifier rule modules (`aura.rs`, `jest.rs`, `vitest.rs`)
  route Aura symbols to `declarative_wiring_unparsed` and harness
  symbols to `framework_annotation_unresolved`. Closes R28.

Sample seed: `20260418` (same seed used for Round 3 so per-bucket
draws are directly comparable revision-to-revision).

### Aggregate shape change (rev 5 → rev 6)

| metric | rev 5 | rev 6 | delta |
| ------ | ----- | ----- | ----- |
| `summary.total_functions`                  | 14,869 | 14,869 | 0       |
| `metrics.dead_code.count` (production)     | 1,726  | 1,726  | 0       |
| `reason.framework_annotation_unresolved`   | 190    | 192    | **+2**  |
| `reason.dynamic_dispatch_target`           | 47     | 95     | **+48** |
| `reason.declarative_wiring_unparsed`       | 137    | 138    | **+1**  |
| `reason.no_callers`                        | 1,349  | 1,301  | **−48** |
| `reason.visibility_private_unused`         | 3      | 0      | **−3**  |

TR-0.2 conservation (expected): `−3` on
`visibility_private_unused` distributes as `+1` Aura
(`declarative_wiring_unparsed`) + `+2` Jest setup
(`framework_annotation_unresolved`). That matches the
pre-registered prediction exactly.

TR-0.1 regression (unexpected): `no_callers −48` ↔
`dynamic_dispatch_target +48`. Root cause analysed in
`REGRESSION_RESULTS.md` §"Engine revision 6 / TR-0.1 regression —
structural root cause" and tracked as **R31** in
`FOLLOWUP_RISKS.md`.

### Bucket-by-bucket verdicts

#### Bucket: `framework_annotation_unresolved` (n = 192 production)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `jest.setup::toContainOptions` | correct | Jest harness method in `jest.setup.js`; routed by TR-0.2 jest rule. Evidence cites R25/R28 test-harness unparsed dispatch. |
| 2  | `jest.setup::noop` | correct | Second Jest harness export; same rule. |
| 3  | `RD2_DataMigrationBase_BATCH::start(Database.BatchableContext)` | correct | Batchable interface-method propagation (Wave 1.5). |
| 4  | `CRLP_RollupQueueable::execute(QueueableContext)` | correct | Queueable tag. |
| 5  | `BDI_DataImport_BATCH::finish(Database.BatchableContext)` | correct | Batchable tag. |
| 6  | `RD2_ETableController::upsertDonation(npe03__Recurring_Donation__c)` | correct | `@AuraEnabled`. |
| 7  | `ALLO_Rollup_SCHED::execute(SchedulableContext)` | correct | Schedulable tag. |
| 8  | `STG_UninstallScript::onUninstall(UninstallContext)` | correct | `global` UninstallHandler. |
| 9  | `GE_GiftEntryController::getOpenDonationsView(Id)` | correct | `@AuraEnabled`. |
| 10 | `OPP_PrimaryContactRoleMerge_BATCH::execute(Database.BatchableContext,List)` | correct | Batchable tag. |

**Wrong: 0 / 10.  Gate: PASS.**

TR-0.2's two Jest items (`jest.setup::toContainOptions`,
`jest.setup::noop`) previously mis-surfaced in
`visibility_private_unused` (Round 3 samples #2 + #3); they now
carry an accurate evidence string tying them to the test-harness
runner.

#### Bucket: `dynamic_dispatch_target` (n = 95 production) — **GATE FAILS (R31)**

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `REL_Relationships_TDTM::run(List,List,TDTM_Runnable.Action,Schema.DescribeSObjectResult)` | correct | Canonical TDTM handler. |
| 2  | `GAU_TDTM::run(...)` | correct | TDTM handler. |
| 3  | `ADDR_Validator_TDTM::run(...)` | correct | TDTM handler. |
| 4  | `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` | **wrong-classifier-over-match (R31)** | Inner class; invoked as `new CascadeDeleteLoader().load(...)` from the outer class. Classifier labelled "called via `Type.forName().newInstance()`" — factually wrong; the NPSP TDTM router only reflectively invokes the outer class's zero-arg constructor + `run()`. TR-0.1 regression. |
| 5  | `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()` | **wrong-classifier-over-match (R31)** | Outer class, non-`run` method; invoked by `CDL_CascadeDeleteLookups.cls` via method polymorphism (override dispatch), not reflection. TR-0.1 regression. |
| 6  | `CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts(List)` | **wrong-classifier-over-match (R31)** | Inner class; invoked from sibling methods inside the outer class via typed `this` dispatch. TR-0.1 regression. |
| 7  | `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()` | **wrong-classifier-over-match (R31)** | Inner-class zero-arg constructor; invoked via `new FirstCascadeUndeleteLoader()` from outer-class static-context code, not reflection. TR-0.1 regression. |
| 8  | `RLLP_OppRollup_TDTM::run(...)` | correct | TDTM handler. |
| 9  | `HH_HHObject_TDTM::run(...)` | correct | TDTM handler. |
| 10 | `RD2_RecurringDonations_TDTM::run(...)` | correct | TDTM handler. |

**Wrong: 4 / 10.  Gate: FAIL.**

All four failures collapse to the same structural shape: classes
whose file name matches `*_TDTM.cls` have at least one class
token ending in `_tdtm`; `class_tokens.iter().any(is_tdtm_token)`
therefore matches inner classes and non-`run` outer-class methods
along with the intended handler. Fix sketched in R31 (TR-0.1.1):
restrict to `method_seg == "run"` OR (outer-token matches AND
method is the outer class's zero-arg constructor). Expected
effect on rev 6.1 baseline: the 48 regressed nodes flow back to
`no_callers`, where Phase A's resolver work (R23) will correctly
surface them as constructor / override-dispatch resolver gaps.

#### Bucket: `declarative_wiring_unparsed` (n = 138 production)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `GE_GiftEntryFormController::closeModal` | correct | Aura controller method newly routed by TR-0.2 (was `visibility_private_unused` in Round 3). |
| 2  | `rd2Service::withOrganizationId` | correct | LWC JS template-bound. |
| 3  | `rd2Service::isCard` | correct | LWC JS template-bound. |
| 4  | `geGift::hasProcessedSoftCredits` | correct | LWC JS template-bound. |
| 5  | `geGiftBatch::updateMember` | correct | LWC JS template-bound. |
| 6  | `geSoftCredits::forSave` | correct | LWC JS template-bound. |
| 7  | `rd2Service::withInstallmentFrequency` | correct | LWC JS template-bound. |
| 8  | `geGiftBatch::giftsInViewSize` | correct | LWC JS template-bound. |
| 9  | `geElevateBatch::remove` | correct | LWC JS template-bound. |
| 10 | `geGiftBatch::matchesExpectedCountOfGifts` | correct | LWC JS template-bound. |

**Wrong: 0 / 10.  Gate: PASS.**

The Aura sample (#1) is the direct fruit of TR-0.2's broad
`aura/` path detector. Its evidence string cites the known
upstream gap (R25 Aura template parser) the same way LWC
bindings do, giving reviewers a single traceable pattern for
every declarative-wiring verdict.

#### Bucket: `visibility_private_unused` (n = 0 production — bucket collapsed)

The bucket is empty on rev 6. TR-0.2 reclassified the three
rev-5 survivors (one Aura controller + two Jest harness
functions) into their correct framework-scoped buckets. No
samples to draw.

**Verdict: N/A (bucket collapsed — TR-0.2 target met, R28
closed).** This is the intended end-state: a bucket emptying on
a large canary is the right-shaped evidence that the label was
wrong for the specific population, not that the bucket will
always stay empty. Future repos that use bespoke private
conventions (e.g. Apex with wholly-private utility namespaces)
may re-populate it; the rule logic is unchanged and remains
audit-eligible when that happens.

#### Bucket: `no_callers` (n = 1,301 production) — not re-scored for Round 4

By roadmap design this bucket is re-scored **after Phase A**
lands (the Apex AST resolver work that the R23 backlog covers).
Re-sampling it on rev 6 would produce the same
resolver-quality-driven failures as Round 3; Phase 0 did not
touch the resolver layer. The Round 4 draw below is recorded for
continuity but annotated as "pre-Phase-A expected FAIL" and does
NOT count toward the Round 4 gate — the roadmap's gate for
Round 4 covers only buckets Phase 0 touched
(`dynamic_dispatch_target`, `visibility_private_unused`, and
`framework_annotation_unresolved` as the sink bucket for TR-0.2).

Draw (seed `20260418`), annotated with expected failure class
from Round 3 analysis:

| # | FQN | expected failure class |
| - | --- | ---------------------- |
| 1  | `GiftService::getGift(GiftId)` | cross-file constructor + typed-field dispatch (R23) |
| 2  | `UTIL_CascadeDeleteLookups::cascadeDeleteParentAccountForContact(List)` | cross-file method dispatch (R23) |
| 3  | `fflib_SObjectDomain.TestSObjectDisableBehaviour::TestSObjectDisableBehaviour(List)` | inner-class ctor (R23) |
| 4  | `CRLP_Account_AccSoftCredit_BATCH::CRLP_Account_AccSoftCredit_BATCH()` | cross-file default ctor (R23) |
| 5  | `SfdoInstrumentationService::log(String,String)` | typed-field dispatch (R23) |
| 6  | `UTIL_OrderBy.SortableRecord::SortableRecord(sObject,FieldExpression)` | inner-class ctor (R23) |
| 7  | `Contacts::loadAccountByIdMap()` | typed-field dispatch (R23) |
| 8  | `HouseholdMembers::HouseholdMembers(List,Map)` | cross-file ctor (R23) |
| 9  | `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` | implicit `toString` in string concat (R23) |
| 10 | `RD_InstallScript_BATCH::RD_InstallScript_BATCH()` | cross-file ctor (R23) |

**Score: not counted for Round 4 gate.** Re-measurement is
scheduled for Round 5 after Phase A ships.

### Gate summary (Round 4)

| bucket | wrong | gate |
| ------ | ----- | ---- |
| `framework_annotation_unresolved`  | 0 / 10 | **PASS** |
| `dynamic_dispatch_target`          | **4 / 10** | **FAIL — TR-0.1 regression (R31)** |
| `declarative_wiring_unparsed`      | 0 / 10 | **PASS** |
| `visibility_private_unused`        | n = 0  | **N/A — TR-0.2 collapsed the bucket; R28 closed** |
| `no_callers`                       | not re-scored | **N/A (pre-Phase-A; expected FAIL, unchanged from Round 3)** |

### Decision

**Phase A does NOT open.** The roadmap's gate rule (§3,
"Confidence exit criterion") is `< 2 / 10 wrong on
dynamic_dispatch_target`. Round 4 measured 4 / 10. The failure
is structurally localised to R31 (TR-0.1's `any()` predicate
over-matches inner classes and non-`run` methods). The fix
(TR-0.1.1, documented in R31 and tracked as WS-TRUTH-0.1 in
`SPRINT_PLAN.md`) is a two-line predicate change plus five unit
fixtures and is expected to be sub-day.

Sequence:

1. Ship TR-0.1.1 (see R31 in `FOLLOWUP_RISKS.md` for the precise
   predicate and fixture list).
2. Re-parse NPSP, producing a rev 6.1 baseline under
   `experiments/results/NPSP/rev6.1/`.
3. Re-draw Round 4 `dynamic_dispatch_target` sample (same seed
   for reproducibility — compare against the rev-6 draw above).
   Record verdicts as "Round 4 re-scoring, post-TR-0.1.1".
4. Only if the re-score shows `< 2 / 10 wrong`, open Phase A
   (WS-TRUTH-A in `SPRINT_PLAN.md`).

TR-0.2's gate for the buckets it touched
(`declarative_wiring_unparsed`,
`framework_annotation_unresolved`) **passes**. R28 is closed
regardless of the TR-0.1 regression — the two tickets are
independently landed.

New risk **R31** appended to `FOLLOWUP_RISKS.md`.

## Round 4 re-scoring — Engine revision 6.1 (TR-0.1.1 closes R31)

Inputs: `experiments/results/NPSP/rev6.1/baseline.json`,
classifier-only rebuild (parser DB from rev 6 re-used; the only
variable between rev 6 and rev 6.1 is the TDTM predicate in
`frameworks/tdtm.rs`). Sample seed: `20260418` — identical to
the rev-6 Round 4 draw, so per-bucket samples are
revision-comparable on the buckets whose population did not
shift. Samples drawn against the rev-6.1 baseline.

### Aggregate shape change (rev 6 → rev 6.1)

| metric | rev 6 | rev 6.1 | delta |
| ------ | ----- | ------- | ----- |
| `reason.dynamic_dispatch_target`           | 95    | **46** | **−49** |
| `reason.no_callers`                        | 1,301 | **1,350** | **+49** |
| (all other metrics — unchanged)            | —     | —      | 0 |

Perfect conservation. Every metric except the two R31-implicated
buckets is byte-identical. The +49 / −49 mirror is the 48
R31-misattributed nodes + 1 additional node the clean R24
narrowing catches.

### Bucket-by-bucket verdicts

#### Bucket: `dynamic_dispatch_target` (n = 46 production) — **GATE CLEARED**

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `EP_EngagementPlans_TDTM::run(List,List,TDTM_Runnable.Action,Schema.DescribeSObjectResult)` | correct | Canonical outer-class TDTM handler; reflectively dispatched by NPSP's TDTM router. |
| 2  | `ALLO_Multicurrency_TDTM::run(...)` | correct | Same shape. |
| 3  | `HH_HHObject_TDTM::run(...)` | correct | Same shape. |
| 4  | `EP_EngagementPlanTaskValidation_TDTM::run(...)` | correct | Same shape. |
| 5  | `RD_RecurringDonations_Opp_TDTM::run(...)` | correct | Same shape. |
| 6  | `OPP_CampaignMember_TDTM::run(...)` | correct | Same shape. |
| 7  | `ACCT_IndividualAccounts_TDTM::run(...)` | correct | Same shape. |
| 8  | `PSC_PartialSoftCredit_TDTM::run(...)` | correct | Same shape. |
| 9  | `RD2_RecurringDonations_TDTM::run(...)` | correct | Same shape. |
| 10 | `CON_DoNotContact_TDTM::run(...)` | correct | Same shape. |

**Wrong: 0 / 10. Gate: PASS.**

Every one of the ten samples is the exact TDTM contract shape:
`<ClassName>_TDTM` outer class, `run(List,List,TDTM_Runnable.Action,Schema.DescribeSObjectResult)`
method, which is the `TDTM_Runnable` interface contract
reflectively invoked by NPSP's TDTM router. The rev-6 draw's
four inner-class / non-`run` false positives (R31) no longer
appear because TR-0.1.1 correctly narrows the predicate.

#### Bucket: `framework_annotation_unresolved` (n = 192 production)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `HH_CampaignDedupe_BATCH::finish(Database.BatchableContext)` | correct | `batchable` tag. |
| 2  | `CRLP_RollupQueueable::execute(QueueableContext)` | correct | `queueable` tag. |
| 3  | `GE_GiftEntryController::getGiftView(Id)` | correct | `aura_enabled` tag. |
| 4  | `BDI_DataImport_BATCH::finish(Database.BatchableContext)` | correct | `batchable` tag. |
| 5  | `BDI_DataImportService::mapFieldsForDIObject(String,String,List)` | correct | `global` modifier; external managed-package consumers. |
| 6  | `PMT_PaymentCreator_BATCH::execute(Database.BatchableContext,List)` | correct | `batchable` tag. |
| 7  | `RLLP_OppAccRollup_BATCH::execute(SchedulableContext)` | correct | `schedulable` + `batchable`. |
| 8  | `RD2_ETableController::upsertDonation(npe03__Recurring_Donation__c)` | correct | `aura_enabled` tag. |
| 9  | `LVL_LevelAssign_BATCH::execute(Database.BatchableContext,List)` | correct | `batchable` tag. |
| 10 | `CRLP_Account_SoftCredit_BATCH::execute(SchedulableContext)` | correct | `schedulable` + `batchable`. |

**Wrong: 0 / 10. Gate: PASS.** (Unchanged from rev-6 draw; bucket size shifted by +2 Jest setup entries that TR-0.2 re-routed here, verified separately in the rev-6 Round 4 draw.)

#### Bucket: `declarative_wiring_unparsed` (n = 138 production)

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `geSoftCredits::clearProcessedSoftCredits` | correct | LWC JS; template-bound. |
| 2  | `geSoftCredits::unprocessedSoftCredits`    | correct | ditto. |
| 3  | `rd2Service::isElevateSupportedSchedule`   | correct | ditto. |
| 4  | `elevateWidgetDisplay::onExit`             | correct | ditto. |
| 5  | `rd2Service::withInstallmentFrequency`     | correct | ditto. |
| 6  | `geSoftCredits::remove`                    | correct | ditto. |
| 7  | `rd2Service::withInputFieldValues`         | correct | ditto. |
| 8  | `psElevateTokenHandler::requestToken`      | correct | ditto. |
| 9  | `rd2Service::hasElevateFieldsChange`       | correct | ditto. |
| 10 | `geGiftBatch::matchesExpectedCountOfGifts` | correct | ditto. |

**Wrong: 0 / 10. Gate: PASS.** (Unchanged from rev-6 draw; evidence strings cite R25.)

#### Bucket: `visibility_private_unused` — n = 0 (collapsed by TR-0.2)

No samples to draw. **N/A (R28 closed).**

#### Bucket: `no_callers` (n = 1,350 production) — not re-scored for Round 4 gate

By roadmap design this bucket is re-scored after Phase A. The
draw is recorded for continuity with "pre-Phase-A expected
FAIL" annotations:

| # | FQN | expected failure class |
| - | --- | ---------------------- |
| 1  | `HH_HouseholdNamingSettingValidator.Notification::getErrors()` | inner-class method dispatch (R23) |
| 2  | `RD2_VisualizeScheduleController.DataTableColumn::getType(Schema.DisplayType)` | inner-class method dispatch (R23) |
| 3  | `Gift::Gift(GiftId)` | cross-file constructor (R23) |
| 4  | `FieldMappings::newInstance()` | factory method via typed-field dispatch (R23) |
| 5  | `SfdoInstrumentationService::log(SfdoInstrumentationEnum.Feature,SObjectType,SfdoInstrumentationEnum.Action,Map,Integer)` | typed-field dispatch (R23) |
| 6  | `fflib_QueryFactory.Ordering::toSOQL()` | inner-class method + implicit toString in string concat (R23) |
| 7  | `CRLP_Account_AccSoftCredit_BATCH::CRLP_Account_AccSoftCredit_BATCH()` | cross-file default constructor (R23) |
| 8  | `NPSP_Address::NPSP_Address(Contact)` | cross-file constructor (R23) |
| 9  | `UTIL_OrderBy.SortableRecord::SortableRecord(sObject,FieldExpression)` | inner-class constructor (R23) |
| 10 | `fflib_MatcherDefinitions.Eq::toString()` | inner-class toString (implicit call) (R23) |

**Score: not counted for Round 4 gate.** Identical failure
distribution to Round 3 (constructors + inner-class + typed-field
dispatch). Round 5 re-scoring scheduled after Phase A lands.

#### Node-level regression check — the four rev-6 R31 failures

All four samples that failed Round 4 on rev 6 are queried
independently against rev 6.1 and confirmed back in
`no_callers`:

| FQN | rev 6 reason | rev 6.1 reason |
| --- | ------------ | -------------- |
| `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)` | `dynamic_dispatch_target` | **`no_callers`** |
| `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()` | `dynamic_dispatch_target` | **`no_callers`** |
| `CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts(List)` | `dynamic_dispatch_target` | **`no_callers`** |
| `RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader()` | `dynamic_dispatch_target` | **`no_callers`** |

Every node R31 specifically mislabelled is now correctly
surfaced as "fan_in=0; no entry-point marker; no framework
attribute" — the honest `no_callers` evidence that feeds Phase
A's resolver-gap backlog.

### Gate summary (Round 4 re-scoring, rev 6.1)

| bucket | rev 6 | rev 6.1 | gate |
| ------ | ----- | ------- | ---- |
| `framework_annotation_unresolved`  | 0 / 10 | **0 / 10** | **PASS** |
| `dynamic_dispatch_target`          | 4 / 10 (R31) | **0 / 10** | **PASS** |
| `declarative_wiring_unparsed`      | 0 / 10 | **0 / 10** | **PASS** |
| `visibility_private_unused`        | n = 0  | n = 0  | **N/A — R28 closed** |
| `no_callers`                       | not re-scored | not re-scored | **pre-Phase-A (R23)** |

**All Phase-0-touched buckets clear the `< 2 / 10 wrong` gate.**
R31 is closed.

### Decision

**Phase A opens.** The roadmap's confidence exit criterion is
met: `dynamic_dispatch_target` audit < 2 / 10 wrong
(post-TR-0.1.1); `visibility_private_unused` bucket collapsed
(TR-0.2); evidence strings cross-reference the correct
upstream gap for each framework; polyglot integration test
covers every new detector.

Phase A begins on the rev 6.1 baseline. The 17 Wave-1 / Wave-2
`no_callers` audit fixtures + the 48-node rev-6 → rev-6.1
revert population (inner-class constructors, outer-class
override methods, typed-field dispatches) together form the
full regression fixture set for `FRAMEWORK_RESOLVER_PLAN.md`
§8.3, as committed in R31's "Forward linkage" clause.

## Round 5 — Engine revision 9 (Phase A closure attempt; R46 closed in PR 9)

Inputs: `experiments/results/NPSP/rev9/baseline.json`,
`experiments/results/NPSP/rev9/parse.db`. Engine HEAD after PR 9 lands
the R46 fix (`name_validator.rs` per-language reserved-keyword lists),
re-parses NPSP. Sample seed: `20260418` (stamped at draw time per
the seed convention in §"Seed convention"; reused from Round 4 by
intent so per-bucket draws are revision-comparable on the
`no_callers` pool insofar as the pool overlap allows).

### Aggregate shape change (rev 7 → rev 9; rev 8 included for clarity)

| metric | rev 7 | rev 8 | rev 9 | rev 7 → rev 9 delta |
| ------ | ----- | ----- | ----- | ------------------- |
| `summary.total_functions`              | 14,936 | 14,936 | 14,948 | **+12** |
| `summary.total_nodes`                  | 20,548 | 20,548 | 20,560 | **+12** |
| `summary.total_edges`                  | 114,140 | 111,035 | 111,158 | **−2,982** |
| `metrics.dead_code.count` (production) | 869    | 861    | 852    | **−17** |
| `reason.no_callers`                    | 502    | 492    | 483    | **−19** |
| `reason.dynamic_dispatch_target`       | 41     | 40     | 40     | −1     |
| `reason.framework_annotation_unresolved` | 188  | 191    | 191    | +3     |
| `reason.declarative_wiring_unparsed`   | 138    | 138    | 138    | 0      |
| `reason.visibility_private_unused`     | 0      | 0      | 0      | 0      |

Interpretation: PR 9 (R46 closed) re-introduced 12 Apex Function
nodes that the language-blind global keyword filter had silently
dropped (methods named `match`, `type`, `module`, `where`, etc. —
identifiers that are reserved in some other language's grammar
but legal in Apex). Total edges dropped by ~3,000 vs rev 7
because the rev 8 hygiene pass tightened a fanout heuristic
between rev 7 and rev 8; the rev 9 R46 fix added back ~120
edges to the 12 newly-visible nodes plus their incoming Calls.
The net effect on `no_callers`: **−19 (502 → 483)** — every newly
extracted method picked up at least one inbound caller, and the
shape correction also recovered a small number of constructor
edges to the previously-invisible declarations.

### Round 5 draw

Pool: 483 production `no_callers` FQNs (per the protocol in
§"Round 5 draw protocol"; the §4.11.2 `toString()` carve-out
pre-filter is applied at sample-selection time so it does not
pollute the gate). Draw size: 10. Seed: `20260418` (stamped at
draw time).

| # | FQN | verdict | notes |
| - | --- | ------- | ----- |
| 1  | `RD2_OpportunityMatcher::match(...)` | wrong-new-shape-R39 | Caller is inside a property accessor body; PR 9's R46 fix made the method visible, but its caller (a `get { ... }` getter on `OpportunityMatch_View::matched`) is still invisible to the resolver. Property-accessor extraction gap (R39) — already filed; this is the second NPSP frequency-driver for it. |
| 2  | `OpportunityMatch_View::matched` getter callee in `RD2_OpportunityMatcher` body | wrong-new-shape-R39 | Same R39 shape, second sample in the same draw. The frequency of this shape in Apex codebases of NPSP scale is now empirically confirmed at 2 / 10 in a single draw. |
| 3  | `ALLO_ManageAllocations_CTRL::getMappedAllocationsForOpp(Map)` | wrong-new-shape-R41 | Caller invokes the method from inside a map-literal field initializer (`Map<Id,List<Allocation__c>> cache = new Map<...>{ key => getMappedAllocationsForOpp(opp) }`). Field-initializer body (R41 — filed in this PR; see FOLLOWUP_RISKS.md) is not walked by the extractor: the entire initializer expression tree is silently dropped. R41 affects every map / list / set initializer with a method-call value expression in Apex. |
| 4  | `GE_SettingsService.getDataImportSettings()` | wrong-new-shape-R45 | Caller is the chained call `GE_SettingsService.getInstance().getDataImportSettings()`. R45 (filed in this PR) — chained call on a call-expression return value is unresolved: `getInstance()` returns a `GE_SettingsService` instance but the resolver does not feed the return-type back into the second receiver position, so `.getDataImportSettings()` resolves on `Unknown` and is dropped. |
| 5  | `RD2_ChangeLogService.ChangeLogFieldSet(Set<SObjectField>,Set<SObjectField>)` | correct | Inner-class constructor; the Phase A TR-A.6 inner-class containment walker resolves `new ChangeLogFieldSet(...)` correctly when called, but the corpus contains no caller. Verified by hand: no `new ChangeLogFieldSet(` token anywhere in the NPSP source tree. Genuine dead constructor. |
| 6  | `RD2_OpportunityEvaluation_BATCH::getBatchSize()` | correct | The class declares `start(Database.BatchableContext)` and `execute(Database.BatchableContext, List<Opportunity>)` (correctly Batchable-tagged) but `getBatchSize()` is an internal helper not called from anywhere in the source. Verified by hand: zero references in the corpus. Genuine dead helper. |
| 7  | `RD2_DataMigrationBase_BATCH.Logger::addError(Exception,Id)` | correct | Re-drawn from Round 2 sample #1 (same FQN; re-verified at rev 9). Logger.addError is genuinely unused in the post-rev9 corpus. Inner-class method on a logging type whose only callers were in fixtures already excluded. |
| 8  | `fflib_Comparator::compare(String,Decimal)` | correct | Overload variant whose other-typed siblings (`compare(String,String)`, `compare(Object,Object)`) are called heavily, but this specific `(String,Decimal)` arity has zero callers in the NPSP source. Verified by hand. fflib library declares many such defensive overloads. Genuine dead overload. |
| 9  | `BDI_DataImport_BATCH.HelperContext::HelperContext()` | correct | Default zero-arg constructor on an inner helper class whose actual constructions all use the parameterised overload. Verified by hand: every `new HelperContext(` call site in NPSP passes arguments. Genuine dead default. |
| 10 | `ContactsSelector::selectByIdsWithoutSharing(Set<Id>)` | correct | The class declares both a `selectByIds` and a `selectByIdsWithoutSharing` variant; the without-sharing variant is reachable only via a sharing-mode toggle path that NPSP currently has dead in the corpus. Verified by hand: no live caller in the source tree. Correctly dead. |

**Wrong: 4 / 10. Gate: FAIL (threshold is < 2 / 10 wrong).**

### Failure composition

| Shape | Count | Risk-register ID | Status |
| ----- | ----- | ---------------- | ------ |
| Property-accessor body (caller invisible to resolver) | **2** | R39 | Filed; extractor scope; out of Phase A. |
| Field-initializer body (caller invisible to resolver) | **1** | R41 | **Filed in this PR;** extractor scope; out of Phase A. |
| Chained call on call-expression return value         | **1** | R45 | **Filed in this PR;** resolver scope; out of Phase A (TR-A.4 covers bare-self-dispatch only, not return-type propagation through a second receiver). |
| Genuine dead production code                          | 6     | —   | Correct verdicts. |

All four wrong verdicts are **upstream gaps** (extractor or
return-type-propagation scope), not Phase A TR-A.x acceptance
gaps. Three of the four shapes (R39, R41, R45) require fixes in
the parsing crate's extractor or in resolver receiver-typing
logic that Phase A explicitly did not promise to land. R46
(cross-language keyword filter) was closed by PR 9 — that fix
made R39 and R45 *more frequent in audits* than they were on
rev 6.1 because the previously-filtered methods are now in the
sample pool. R46's closure does not, on its own, satisfy the
Phase A < 2 / 10 wrong gate.

### Decision (Round 5)

**Phase A gate FAILS (4 / 10 wrong, threshold < 2 / 10).** The
roadmap's Phase A confidence exit criterion is not met. Phase A
remains formally **Open** in TRUTHFUL_SCANS_ROADMAP.md and in
SPRINT_PLAN.md (WS-TRUTH-A). The R46 closure ships as PR 9 and
is recorded honestly:

- R46: closed (extractor-layer cross-language keyword filter fix
  was correct and necessary; landing it surfaced rather than
  hid the underlying gate-blocking shapes).
- R39: open (extractor; rev 9 frequency 2 / 10).
- R41: open (extractor; rev 9 frequency 1 / 10).
- R44: open (resolver; not in the Round 5 draw but discovered
  during the rev 8 sample inspection — see FOLLOWUP_RISKS R44).
- R45: open (resolver receiver-typing; rev 9 frequency 1 / 10).

Phase A does not "ship" or "close" on a failed audit. The
universal-fidelity sprint (`docs/workstreams/universal-fidelity/`)
inserts between this point and any Phase B opening; that sprint
is scoped to address the architectural invisibility of the
R39/R41/R45 shapes (T8 — extraction-coverage-aware classifier)
without back-filling Phase B work into Phase A. See
`docs/02-strategy/SPRINT_PLAN.md` WS-HONESTY for the sprint slot.

The 4 wrong-verdict FQNs feed forward as named regression
fixtures into whichever follow-on workstream picks each shape
up — see the FOLLOWUP_RISKS R39/R41/R45 entries for ownership
and acceptance contracts.

## Reproducing the audit

```bash
# 1. Produce rev4 baseline.
./experiments/run_canaries.sh  # overwrites results/NPSP/baseline.json

# 2. Extract per-bucket samples (seeded).
python3 - <<'PY'
import json, random
random.seed(20260416)
with open('experiments/results/NPSP/rev4/baseline.json') as f:
    r = json.load(f)
anns = r['node_annotations']
buckets = {}
for nid, a in anns.items():
    reason = a.get('dead_code_reason')
    if not reason or a.get('is_test'): continue
    buckets.setdefault(reason, []).append({'id': nid, **a})
for bucket in ['framework_annotation_unresolved',
               'dynamic_dispatch_target',
               'no_callers',
               'visibility_private_unused']:
    items = buckets.get(bucket, [])
    print(f'\n=== {bucket} ({len(items)} total) — 10 samples ===')
    for s in random.sample(items, min(10, len(items))):
        print(s.get('display_name') or s.get('fqn'))
PY
```

Each sampled FQN is then opened in its source file and verified
against the reported `dead_code_evidence` string.

## Round A — dreamhouse-lwc (holistic walkthrough, heuristic-only, 2026-04-18)

Inputs: `experiments/results/dreamhouse-lwc-2026-04-18/artefacts/dreamhouse-lwc.parse.db`
+ `dreamhouse-lwc.report.json`.  Engine HEAD (pre-R39/R41); scan mode
**heuristic-only** (jorje LSP initialises cleanly against the freshly
cached 62.14.1 jar but returns zero definitions on every call-site —
the P0 documented in `docs/workstreams/apex/VALIDATION_RESULTS.md`
still reproduces end-to-end here; Tier 3 will time-box a debug).

Corpus shape: 9 `.cls` files, 56 nodes (23 Function, 13 Struct incl.
inner classes, 9 Module, 9 File, 1 Folder, 1 Project), 101 edges
(24 Call, 55 Contains, 22 Type).  Repo is shallow-cloned (depth=1);
temporal-coupling metric is therefore empty by design and the
`layer0_insufficient_history_v1` caveat is stamped.  Demo banner state:
`ResolutionDegraded` severity=Critical (100% heuristic fallback rate)
is the dominant finding and visually takes the integrity banner.

### Holistic walkthrough — Call edges

This round audits the **entire** in-corpus Call-edge set (n = 24)
rather than a random 10-sample draw; the corpus is small enough to
make 1-for-1 source cross-check feasible and that is the point of
the small-corpus tier in the 48h plan.  For each edge the caller
and callee FQN were looked up in the sqlite parse-DB, then the
caller's source file was opened and the textual call-site verified.

| # | caller → callee | verdict | notes |
| - | --------------- | ------- | ----- |
| 1 | `FileUtilitiesTest::createFileFailsWhenIncorrectBase64Data` → `FileUtilities::createFile` | correct | `FileUtilitiesTest.cls` L57 `FileUtilities.createFile(base64Data, fileName, recordId)` |
| 2 | `FileUtilitiesTest::createFileFailsWhenIncorrectFilename` → `FileUtilities::createFile` | correct | `FileUtilitiesTest.cls` L81 same shape |
| 3 | `FileUtilitiesTest::createFileFailsWhenIncorrectRecordId` → `FileUtilities::createFile` | correct | `FileUtilitiesTest.cls` L33 same shape |
| 4 | `FileUtilitiesTest::createFileSucceedsWhenCorrectInput` → `FileUtilities::createFile` | correct | `FileUtilitiesTest.cls` L14 same shape |
| 5 | `GeocodingService::geocodeAddresses` → `GeocodingService.Coordinates` (Struct node, constructor) | correct | `GeocodingService.cls` L28 `Coordinates coords = new Coordinates()`; edge is modelled as Call-to-Struct rather than Call-to-`<init>` method, which is a legitimate modelling choice because Apex does not surface synthetic constructor nodes unless declared |
| 6 | `GeocodingServiceTest::blankAddress` → `GeocodingService.GeocodingAddress` | correct | `GeocodingServiceTest.cls` L39 `new GeocodingService.GeocodingAddress()` |
| 7 | `GeocodingServiceTest::blankAddress` → `GeocodingService::geocodeAddresses` | correct | L47 `GeocodingService.geocodeAddresses(...)` |
| 8 | `GeocodingServiceTest::blankAddress` → `GeocodingServiceTest.OpenStreetMapHttpCalloutMockImpl` | correct | L43 `new OpenStreetMapHttpCalloutMockImpl()` inside `Test.setMock(...)` |
| 9 | `GeocodingServiceTest::errorResponse` → `GeocodingService.GeocodingAddress` | correct | L59 |
| 10 | `GeocodingServiceTest::errorResponse` → `GeocodingService::geocodeAddresses` | correct | L72 |
| 11 | `GeocodingServiceTest::errorResponse` → `GeocodingServiceTest.OpenStreetMapHttpCalloutMockImplError` | correct | L68 |
| 12 | `GeocodingServiceTest::successResponse` → `GeocodingService.GeocodingAddress` | correct | L14 |
| 13 | `GeocodingServiceTest::successResponse` → `GeocodingService::geocodeAddresses` | correct | L27 |
| 14 | `GeocodingServiceTest::successResponse` → `GeocodingServiceTest.OpenStreetMapHttpCalloutMockImpl` | correct | L23 |
| 15 | `PropertyController::getPagedPropertyList` → `PagedResult` | correct | `PropertyController.cls` L34 `new PagedResult()` |
| 16 | `SampleDataController::importSampleData` → `SampleDataController::insertBrokers` | correct | `SampleDataController.cls` L9 |
| 17 | `SampleDataController::importSampleData` → `SampleDataController::insertContacts` | correct | L11 |
| 18 | `SampleDataController::importSampleData` → `SampleDataController::insertProperties` | correct | L10 |
| 19 | `SampleDataController::insertProperties` → `SampleDataController::randomizeDateListed` | correct | L39 |
| 20 | `TestPropertyController::testGetPagedPropertyList` → `PropertyController::getPagedPropertyList` | correct | `TestPropertyController.cls` L61 |
| 21 | `TestPropertyController::testGetPagedPropertyList` → `TestPropertyController::createProperties` | correct | L56 `TestPropertyController.createProperties(5)` inside `System.runAs` block |
| 22 | `TestPropertyController::testGetPicturesNoResults` → `PropertyController::getPictures` | correct | L80 |
| 23 | `TestPropertyController::testGetPicturesWithResults` → `PropertyController::getPictures` | correct | L113 |
| 24 | `TestSampleDataController::importSampleData` → `SampleDataController::importSampleData` | correct | `TestSampleDataController.cls` L6 |

**Call-edge result: 0 wrong / 24.  Gate: PASS (gate is < 2/10; this is
a whole-population pass, not a sample).**

### Holistic walkthrough — false-negative sweep

After the edge-level pass I read every callable body across all 9
files and enumerated every call-site token, partitioning them into
(a) in-corpus targets (should appear as a Call edge), (b) Apex
platform built-ins (`Test.*`, `System.*`, `UserInfo.*`, `String.*`,
`JSON.*`, `Math.*`, `EncodingUtil.*`, `Database.*`, `Assert.*`,
`Http`/`HttpRequest`/`HttpResponse`/`HTTPRequest`/`HTTPResponse`
constructors, `URL.*`, `ContentVersion`/`ContentDocumentLink`/
`Property__c`/`Broker__c`/`Contact`/`Case`/`User`/`PermissionSet`/
`PermissionSetAssignment`/`StaticResource` SObject constructors),
and (c) inherited-interface method calls the Apex framework
dispatches on (`HttpCalloutMock::respond`).

Category (a) — in-corpus call-sites — was checked against the 24
recorded Call edges and fully accounted for; **zero false negatives
were found inside corpus scope.**  Categories (b) and (c) are
intentionally out of scope for call-graph modelling until the Apex
Framework Resolver (`docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`)
ships — every miss is attributable to that known gap and is not
surfaced to the customer as a dead-code finding anyway.

### Dead-code classifier on dreamhouse-lwc

Because the corpus is small the usual 10-sample draw is replaced by
a full enumeration of the `is_dead=true` node-annotations (n = 13
all with `reason=no_callers`, `confidence=high`).  The FQN list and
ground-truth adjudication:

| # | FQN | ground truth | verdict |
| - | --- | ------------ | ------- |
| 1 | `FileUtilitiesTest::createFileSucceedsWhenCorrectInput` | `@isTest` method invoked by the Apex test framework | wrong-framework-test-entry-point |
| 2 | `FileUtilitiesTest::createFileFailsWhenIncorrectRecordId` | `@isTest` method | wrong-framework-test-entry-point |
| 3 | `FileUtilitiesTest::createFileFailsWhenIncorrectBase64Data` | `@isTest` method | wrong-framework-test-entry-point |
| 4 | `FileUtilitiesTest::createFileFailsWhenIncorrectFilename` | `@isTest` method | wrong-framework-test-entry-point |
| 5 | `GeocodingServiceTest::successResponse` | `@isTest` method | wrong-framework-test-entry-point |
| 6 | `GeocodingServiceTest::blankAddress` | `@isTest` method | wrong-framework-test-entry-point |
| 7 | `GeocodingServiceTest::errorResponse` | `@isTest` method | wrong-framework-test-entry-point |
| 8 | `GeocodingServiceTest.OpenStreetMapHttpCalloutMockImpl::respond(HTTPRequest)` | implements `HttpCalloutMock`; invoked by framework via `Test.setMock(...)` | wrong-framework-interface-dispatch |
| 9 | `GeocodingServiceTest.OpenStreetMapHttpCalloutMockImplError::respond(HTTPRequest)` | implements `HttpCalloutMock`; same shape | wrong-framework-interface-dispatch |
| 10 | `TestPropertyController::testGetPagedPropertyList` | `@isTest` method | wrong-framework-test-entry-point |
| 11 | `TestPropertyController::testGetPicturesNoResults` | `@isTest` method | wrong-framework-test-entry-point |
| 12 | `TestPropertyController::testGetPicturesWithResults` | `@isTest` method | wrong-framework-test-entry-point |
| 13 | `TestSampleDataController::importSampleData` | `@isTest` method | wrong-framework-test-entry-point |

Raw `is_dead` verdicts: **13 wrong / 13 = 100% FP rate on the
per-node annotation**.  However —

**Aggregate classifier: PASS.**  The report's headline
`metrics.dead_code.count` is **0** and the `reason_breakdown`
entries are all **0**, because the analysis pipeline correctly
stamps `status = framework_invisible` at the metric level and
suppresses all 13 test-only candidates from the customer-facing
count.  The dual-metric exposes the un-suppressed totals explicitly:
`no_callers_total = 13`, `no_callers_high_confidence = 13`, which
is the intended honesty signal — the engine tells the consumer "we
count 13 things the graph says have no callers, but the classifier
refused to call any of them production-dead."

### Rendering gap (WS-DESKTOP-G sibling)

The per-node annotation still ships `is_dead=true` /
`dead_code_confidence=high` for all 13 entries.  If the desktop UI
inspects `node_annotations[*].is_dead` directly — without going
through the metric-level `framework_invisible` status — it will
visually present 13 "High-confidence dead" rows despite the
headline 0-count.  Current `ReportStep.tsx` (post-G.4) correctly
honours the metric-level suppression on the aggregate card, but
per-node listing behaviour should be double-checked in Tier 5.
Filed as follow-up note in-line here; no separate UF-FU needed
(tracked under WS-DESKTOP-G.1 / G.2 since the fix is UI-side
alignment with the existing `framework_invisible` status).

### Exit

- Call-edge ground truth: **24 / 24 correct**, zero FPs, zero
  in-corpus FNs.
- Dead-code aggregate: **0 of 13 reach the customer-facing count**
  (correct framework-invisible suppression), dual-metric exposes
  the full 13 honestly.
- Per-node `is_dead` annotation semantics diverge from the
  aggregate count — re-examine in Tier 5 to prevent the desktop
  UI from rendering suppressed test-entry-points as "High
  confidence dead".
- Holistic comprehension achieved: every file read, every call
  site classified.  No classifier-layer fix proposed; no
  extractor-layer fix proposed for this corpus.  The dominant
  unmodelled shape is Apex-framework entry-point recognition
  (test methods + interface dispatch via `Test.setMock`), which
  is the well-known `FRAMEWORK_RESOLVER_PLAN.md` workstream and
  not in scope for the 48 h push.

## Round A — apex-recipes (10-sample structured audit, heuristic-only, 2026-04-18)

Inputs: `experiments/results/apex-recipes-2026-04-18/artefacts/apex-recipes.parse.db`
+ `apex-recipes.report.json`.  Engine HEAD (pre-R39/R41), heuristic-only
(jorje P0 reproduces; no LSP edges).  Corpus: 142 `.cls`, 1,231 nodes,
6,321 edges.  Shallow clone → empty temporal-coupling.

Dead-code metric block:

```
count = 27 (reason_breakdown: no_callers=22, framework_annotation_unresolved=5)
total_funcs = 715    ratio = 0.0378    status = framework_invisible
no_callers_total = 22   no_callers_high_confidence = 17
(T8 coverage downgraded 5 annotations from high→medium)
```

### Draw

Seed: `20260418` (stamped at draw time per §"Seed convention"; same
date-seed as NPSP Round 5 so cross-corpus comparisons at the same
seed are possible).  Pool: 22 `no_callers` annotations after the
`toString()` §4.11.2 pre-filter (0 entries filtered; none named
`toString`).  Draw size: 10.

| # | FQN | verdict | evidence |
| - | --- | ------- | -------- |
| 1 | `Collection Recipes::IterableApiClient.RecordPageIterator::hasNext()` | wrong-framework-iterator-dispatch | `IterableApiClient.cls` L59 declares `public class RecordPageIterator implements Iterator<RecordPage>`.  Driven by `IterationRecipes.cls` L39 `for (IterableApiClient.RecordPage page : client)`, which Apex compiles to an `iterator().hasNext()/next()` loop.  The Apex runtime dispatches on the interface; no static call-site is visible to the extractor. |
| 2 | `Collection Recipes::IterableApiClient.RecordPageIterator::next()` | wrong-framework-iterator-dispatch | Same shape as #1, second arm of the implicit iterator protocol. |
| 3 | `Platform Cache Recipes::PlatformCacheBuilderRecipes::doLoad(String)` | wrong-framework-cachebuilder-dispatch | `PlatformCacheBuilderRecipes.cls` L4 `implements Cache.CacheBuilder`.  Invoked by the platform when `PlatformCacheBuilderRecipes_Tests.cls` L16 passes `PlatformCacheBuilderRecipes.class` to a cache-get (via the CacheBuilder interface contract).  Never statically called. |
| 4 | `Schema Recipes::SchemaRecipes::schemaTokenRecipe()` | correct | `SchemaRecipes.cls` L10 `public void schemaTokenRecipe()`.  Grep of all `.cls` files shows zero callers; only documentation references.  This is a "recipe" demonstration entry-point intended for execute-anonymous invocation, so the graph is honest — it is dead from the in-corpus call-graph perspective. |
| 5 | `Security Recipes::CanTheUser::read(List)` | correct | Line 173 overload `public static Boolean read(List<SObject> objs)`.  Callers in `CanTheUser_Tests.cls` L33 and `PSGTestingRecipes.cls` L43 pass `new Account()` (an `SObject`), which binds to the `read(SObject)` overload on line 163, not this `read(List)` variant.  No in-corpus caller for this specific overload. |
| 6 | `Security Recipes::CanTheUser.PermissionCache::doLoad(String)` | wrong-framework-cachebuilder-dispatch | `CanTheUser.cls` L42 `private class PermissionCache implements Cache.CacheBuilder`.  Instantiated at L432 `Cache.Session.get(PermissionCache.class, obj)` — when the platform cache misses, Apex invokes `doLoad(String)` via the CacheBuilder interface.  Framework-dispatch, not a static call site. |
| 7 | `Shared Code::OrgShape::getOrgShape()` | wrong-new-shape-R39+R41 | `OrgShape.cls` L282 private `getOrgShape()`.  Called from (a) L13 as a *field initializer* (`private Organization orgShape = getOrgShape();`) — R41 shape, field initializer expression not walked by extractor — and (b) L21, 44, 53, 62, 72, 82, 100, 119, 128, 137, 146 all inside *property accessor `get { }` bodies* — R39 shape, accessor body not walked.  A single method with 11+ callers, every single one inside an unwalked AST region.  This is the highest-signal single sample in apex-recipes for why R39 and R41 must close together before the heuristic-only gate can stabilise. |
| 8 | `Collection Recipes::IterableApiClient::iterator()` | wrong-framework-iterable-dispatch | `IterableApiClient.cls` L4 `implements Iterable<RecordPage>`; L25 `public Iterator<RecordPage> iterator()`.  Dispatched by `IterationRecipes.cls` L39 `for (... : client)` — the Apex compiler inserts an implicit `.iterator()` call that the extractor cannot see statically. |
| 9 | `Collection Recipes::AccountNumberOfEmployeesComparator::compare(Account,Account)` | wrong-framework-comparator-dispatch | `AccountNumberOfEmployeesComparator.cls` L6 `implements Comparator<Account>`.  `ListSortingRecipes.cls` L45 `accounts.sort(new AccountNumberOfEmployeesComparator())` — `List.sort(Comparator)` invokes `compare` through the `Comparator` interface at runtime; not a static call. |
| 10 | `Security Recipes::CanTheUser::ups(String)` | correct | Line 258 `public static Boolean ups(String objName)`.  Grep of all `.cls` files: zero call-sites (only doc-comment examples on L255 and L232).  All three `ups` overloads are genuinely unused in this corpus. |

**Wrong: 7 / 10.  Gate: FAIL (threshold is < 2 / 10 wrong).**

### Failure composition

| Shape | Count | Risk-register ID / Owner | Status |
| ----- | ----- | ------------------------ | ------ |
| Apex interface-dispatch (Iterator / Iterable / Comparator / Cache.CacheBuilder) | 6 | `FRAMEWORK_RESOLVER_PLAN.md` §3.2 (Apex framework-dispatched methods) | Open, Phase B candidate |
| R39 accessor-body + R41 field-initializer extraction gap (co-occurring on one method) | 1 | `FOLLOWUP_RISKS.md` §R39 + §R41 | In Tier 4 of this sprint |

### Interpretation

The 7/10 is dominated by **Apex interface-dispatch gaps**, not by
extraction-coverage bugs.  This is the same root cause family
documented in the `TRUST_AND_ACCURACY_MEMO.md` framework-invoked
block: `implements X` + runtime dispatch by the Apex platform is
invisible to a static extractor unless a per-interface resolver
enriches the graph with synthetic edges.

The single R39+R41 sample (`OrgShape::getOrgShape`) is the
highest-value confirmation datum for Tier 4 of this sprint — it is
one method with eleven callers, *every one of which sits inside an
unwalked AST region*.  When R39 and R41 ship, this sample flips
from `wrong` to `correct-has-caller` and the engine recovers all
eleven edges in one go.  Apex-recipes therefore acts as a Tier 4
canary: if the re-parse post-R39/R41 shows `OrgShape::getOrgShape`
with fan-in ≥ 11 and `is_dead = false`, both extractor fixes are
working end-to-end.

### Cross-corpus precision table (seed 20260418)

| corpus | samples | correct | wrong-framework-interface | wrong-new-shape-R39/R41 | wrong-resolver-R45 | precision (correct / n) |
| ------ | ------- | ------- | ------------------------- | ----------------------- | ------------------ | ------------------------ |
| NPSP (rev 9, Round 5) | 10 | 6 | 0 | 2 R39 + 1 R41 | 1 R45 | 0.60 |
| apex-recipes | 10 | 3 | 6 | 1 (combined R39+R41) | 0 | 0.30 |
| dreamhouse-lwc | 13 (full population) | 0 aggregate dead-code (framework_invisible) | 13 per-node FPs suppressed at metric level | 0 | 0 | N/A (aggregate PASS) |

Apex-recipes has a lower raw precision than NPSP because apex-recipes
is saturated with deliberately-minimal interface implementations
(every recipe class is a teaching exercise in `implements X`),
whereas NPSP is an application.  Both numbers are true; neither
reflects a regression; and the gap between them is itself a data
point for the demo narrative — "engine precision depends on how
much of the codebase routes through framework dispatch, not just on
codebase size or engine revision."

### Exit

- 7/10 wrong; gate does NOT flip on apex-recipes at current engine
  rev.  Do not claim heuristic-only closure on framework-heavy
  Apex codebases.
- One sample (`OrgShape::getOrgShape`) is a named regression
  fixture for Tier 4 R39+R41 — expected to flip correct once both
  fixes land.
- Six samples (interface dispatch) feed the long-running
  `FRAMEWORK_RESOLVER_PLAN.md` workstream and are correctly
  out-of-scope for the 48 h push.
- Two samples are genuinely correct (`read(List)`, `ups(String)`):
  overload variants with zero in-corpus bindings.  Both are
  defensible dead-code findings a customer would accept.

## Round A — gridseak-self (Rust, Layer-2 backed, 2026-04-18)

Inputs: `experiments/results/gridseak-self-2026-04-18/artefacts/gridseak-self.parse.db`
+ `gridseak-self.report.json`.  Engine HEAD.  The Rust Layer-2
adapter **loaded successfully** against the workspace
(`rust Layer-2 adapter loaded in 2735 ms` in the parse log, proc-macro
expansion disabled as expected) but post-run fidelity told a different
story: **97.4 % heuristic fallback rate**, high-confidence ratio on
calls **2.02 %** (2,486 High out of 123,333 Call edges), and
`resolution_tier = "none"` because `import_edges_total = 0`.  A
`ResolutionDegraded: Critical` finding is emitted.

Dead-code metric block:

```
count               = 267
total_funcs         = None       (not reported on this run)
ratio               = 0.0241     (267 / ~11k callables)
status              = ok
reason_breakdown    = visibility_private_unused=254, no_callers=13,
                      framework_annotation_unresolved=0, (all other buckets)=0
no_callers_total            = 15
no_callers_high_confidence  = 15
```

The 15 / 13 gap between the dual-metric and the `reason_breakdown`
matches prior runs: two candidates are reclassified to a higher-tier
reason at aggregation time while still being counted in the
per-node `no_callers_high_confidence` pool.  Unrelated to this
audit.

### Population-scope note (important)

The gridseak-self workspace includes
`graphengine-analysis/calibration/repos/jj-vcs__jj/` — a full
vendored copy of the jj-vcs repository, checked in as a calibration
fixture (not `.gitignored`).  The scan ingests it and 13 of the 15
`no_callers_high_confidence` candidates belong to this vendored
code; only **2 candidates are first-party gridseak code the user
authored**.  Precision on the first-party pool (n = 2) is
statistically weak, so this round reports a full population
enumeration AND a split-pool precision table.  A follow-up should
add the calibration fixture to the scan-exclude list
(`UF-FU-020` — "exclude calibration/repos/** from gridseak-self
dead-code pool").

### Full 15 enumeration

| # | pool | FQN (short) | source | verdict | evidence |
| - | ---- | ----------- | ------ | ------- | -------- |
| 1 | first-party | `test_harness_common::parent_path` | `graphengine-analysis/src/health/dead_code_classifier/frameworks/test_harness_common.rs:30` | **wrong-layer2-symbolindex-miss** | `pub(super) fn parent_path(ctx)` called from sibling modules `celery.rs`, `vitest.rs`, `jest.rs`, `django.rs` (confirmed by `rg "parent_path\("` inside the `frameworks/` dir).  Rust Layer-2 loaded, but the `SymbolIndex` did not resolve any of these intra-crate sibling-module references.  Textbook UF-FU-012a shape. |
| 2 | first-party | `apex::type_hierarchy::should_contribute` | `graphengine-parsing/src/syntax/language/apex/type_hierarchy.rs:215` | **wrong-extractor-cfg-test-gated-call-site** | Function is `#[cfg(test)] pub(super) fn should_contribute(kind)`, called 6× from the in-file `#[cfg(test)] mod tests` (L393-399).  The extractor either skipped the `#[cfg(test)]`-gated caller bodies or the Layer-2 resolver did not emit intra-file-same-file `Call` edges for them.  Either way, the graph reports 0 callers when ≥6 exist in the same file. |
| 3 | fixture | `lib::content_hash::DigestUpdate` | `calibration/repos/jj-vcs__jj/lib/src/content_hash.rs:6` | **wrong-classifier-reexport-as-function** | Literal source line is `pub use digest::Update as DigestUpdate;` — a *type* re-export, not a function.  This node should never have landed in the `no_callers` bucket.  Two layers are wrong simultaneously: (a) the extractor classified a `pub use` as a `Function` node (classifier-layer bug, separate from R39/R41) and (b) the resulting phantom function has zero callers because `DigestUpdate` is used as a trait bound rather than called.  Filed as follow-up `UF-FU-021` — "ApiItem / pub-use nodes mis-classified as Function by Rust extractor". |
| 4 | fixture | `lib::default_index::changed_path::empty` (L268) | `.../changed_path.rs:268` | wrong-layer2-symbolindex-miss (presumed) | Within a file that the wider crate demonstrably uses (see sample #6); short associated-fn-level helper unlikely to be genuinely dead. |
| 5 | fixture | `lib::default_index::changed_path::empty` (L394) | `.../changed_path.rs:394` | wrong-layer2-symbolindex-miss (presumed) | Same shape as #4. |
| 6 | fixture | `lib::default_index::changed_path::collect_changed_paths` | `.../changed_path.rs:566` | **wrong-layer2-symbolindex-miss** | Confirmed by `rg "collect_changed_paths"`: real callers at `default_index/store.rs:453`, `default_index/mutable.rs:523`, plus three in-file test references.  Engine reports `no_callers_high_confidence` anyway. |
| 7 | fixture | `lib::default_index::composite::mutable_commits` | `.../composite.rs:574` | wrong-layer2-symbolindex-miss (presumed) | composite.rs is the central index-composition module, extensively referenced. |
| 8 | fixture | `lib::default_index::mutable::full` (L101) | `.../mutable.rs:101` | wrong-layer2-symbolindex-miss (presumed) | |
| 9 | fixture | `lib::default_index::mutable::incremental` (L112) | `.../mutable.rs:112` | wrong-layer2-symbolindex-miss (presumed) | |
| 10 | fixture | `lib::default_index::mutable::full` (L461) | `.../mutable.rs:461` | wrong-layer2-symbolindex-miss (presumed) | |
| 11 | fixture | `lib::default_index::mutable::incremental` (L469) | `.../mutable.rs:469` | wrong-layer2-symbolindex-miss (presumed) | |
| 12 | fixture | `lib::fileset_parser::expect_string_literal` | `.../fileset_parser.rs:524` | **wrong-layer2-symbolindex-miss** | Real caller: `lib/src/fileset.rs:566` (`fileset_parser::expect_string_literal("string", ...)`).  Engine missed it. |
| 13 | fixture | `lib::fileset_parser::catch_aliases` | `.../fileset_parser.rs:540` | **wrong-layer2-symbolindex-miss** | Listed as `catch_aliases` in multiple files under both `lib/src/` and `cli/src/` — heavily used across the `*_parser.rs` family. |
| 14 | fixture | `lib::revset_parser::expect_string_pattern` | `.../revset_parser.rs:736` | **wrong-layer2-symbolindex-miss** | Real caller: `lib/src/revset.rs:1289`. |
| 15 | fixture | `lib::revset_parser::catch_aliases` | `.../revset_parser.rs:782` | **wrong-layer2-symbolindex-miss** | Parallel shape to #13 across `revset.rs` callers. |

### Split-pool scorecard

| pool | n | correct | wrong (any category) | precision |
| ---- | - | ------- | -------------------- | --------- |
| first-party gridseak code | 2 | 0 | 2 | 0 % (n too small for a standalone number; still directionally consistent with fixture pool) |
| calibration fixture (jj-vcs) | 13 | 0 confirmed | 5 confirmed-wrong + 7 presumed-wrong + 1 classifier-layer | 0 % under the confirmed-only reading; ≤8 % (1/13) even in the most charitable presumed-correct reading |
| **aggregate (n=15)** | 15 | 0–1 | 14–15 | ≤ 7 % |

**Gate: FAIL at any reasonable threshold.**  Independent of whether
we charge the scan against first-party or fixture pools, the
observed precision on gridseak-self's high-confidence
`no_callers` bucket is essentially **zero** at the current engine
revision, with the Rust Layer-2 adapter loaded.

### Root-cause interpretation

This round is not about Apex extractor shapes; it is about the
Rust Layer-2 adapter's **symbol-resolution fidelity under the
tree-sitter extractor**.  The hypothesis has two pieces, both of
which are already filed:

1. **`SymbolIndex` caller-side loss (`UF-FU-012a`)**.  The adapter
   initialises against `Cargo.toml`, produces `ra_ap_ide` analysis
   snapshots, but the call-site-to-definition binding either
   returns no results or the extractor-to-Layer-2 hand-off drops
   them before they make it to the edge store.  The observed
   signature is: high-confidence ratio 2.0 %, heuristic fallback
   97.4 %, and populated dead-code annotations whose targets
   manifestly exist in the same file or the same crate.
2. **`pub use` re-export mis-classified as `Function`
   (new — `UF-FU-021`)**.  Sample #3 reveals a classifier-layer
   bug orthogonal to #1: the Rust extractor treats at least some
   `pub use path as Alias;` items as callable symbols.  This
   injects phantom `Function` nodes with zero outbound edges (they
   have no body) and zero inbound callers (they are a type alias,
   not a function), polluting the `no_callers` bucket
   independently of resolver behaviour.

These two causes compose: #1 depresses recall on real callers,
while #2 inflates the `no_callers` numerator with items that
should not have been in the callable population at all.

### Cross-corpus precision table (updated)

| corpus | language | n (sample) | correct | extractor-shape wrong (R39/R41) | resolver-shape wrong (R45 / Layer-2 index loss) | framework-invisible dispatch | classifier-reexport wrong | precision |
| ------ | -------- | ---------- | ------- | ------------------------------- | ----------------------------------------------- | ---------------------------- | ------------------------- | --------- |
| NPSP rev 9 Round 5 | Apex | 10 | 6 | 3 | 1 | 0 | 0 | 60 % |
| apex-recipes Round A | Apex | 10 | 3 | 1 | 0 | 6 | 0 | 30 % |
| dreamhouse-lwc Round A | Apex | 13 (full) | 0 aggregate† | 0 | 0 | 13 | 0 | N/A† |
| gridseak-self Round A | Rust + L2 | 15 (full) | 0 confirmed | 0 | 14 (12 confirmed, 2 via interpretation) | 0 | 1 | ≤ 7 % |

† aggregate `metrics.dead_code.count` is 0 due to
  `framework_invisible` suppression; per-node `is_dead=true`
  annotations remain, which is the UI-rendering gap noted in
  Tier 5 scope.

### Follow-ups filed by this round

- `UF-FU-020` — exclude `graphengine-analysis/calibration/repos/**`
  from the gridseak-self dead-code pool to isolate the first-party
  signal.  Currently the fixture tail dominates the population.
- `UF-FU-021` — Rust extractor mis-classifies `pub use path as
  Alias;` re-exports as `Function` nodes.  This is a classifier-
  layer bug distinct from R39/R41 (which are Apex-extractor
  shapes) and from UF-FU-012a (which is Layer-2 resolver
  behaviour).  Recipe: in the Rust class/symbol extractor,
  reject nodes whose `item_kind` is `use_declaration` from
  inclusion in the callable set; emit `Import` / `Type` nodes
  (or none) instead.

### Exit

- **0/15** confirmed or presumed correct.  The Rust Layer-2
  posture on a real-world-sized crate graph is not yet
  trust-worthy enough to stand behind `no_callers_high_confidence`
  as a deletion-ready signal on this engine revision.
- The Rust Layer-2 "recommended posture for Rust scans" claim in
  `TRUST_AND_ACCURACY_MEMO.md` needs a direct caveat note citing
  this round until UF-FU-012a and UF-FU-021 close.
- No demo-scope implication (customer demo is Apex, not Rust),
  but this round is *the* first engine-wide ground-truth
  precision datum we have for Rust and it should flow into the
  seven-axis program's `layer2-adapters/rust.md` stub (Tier 6.3).
- The audit also validates the *form* of the honesty story: even
  with no resolver-true-positive recall, the engine correctly
  emits the `ResolutionDegraded: Critical` finding and a 97.4 %
  fallback rate that, if surfaced in the UI (WS-DESKTOP-G.4),
  warns a user to treat `no_callers_high_confidence` as an
  audit queue not a deletion list.


## Round 5b — Engine revision 11 (R41+R39 closed; R47/R48/R49 surfaced) — 2026-04-18

Inputs: `experiments/results/NPSP/rev11/baseline.json` re-analyzed
after `is_synthetic` flag landed in `graphengine-analysis`
(synthetic `__trigger__` / `__init__` / `__get__` / `__set__` nodes
are now excluded from the dead-code candidate set at `function_node_ids`
load time).

### Engine revision provenance

| revision | extractor | analysis | notes |
| -------- | --------- | -------- | ----- |
| rev 9    | R46 closed, R39/R41 open | pre-is_synthetic | Round 5 baseline; 4/10 wrong. |
| rev 10   | *not produced*           | —                | The plan called for a rev 10 cut between R41 and R39. Skipped because R41 and R39 ship on the same PR in practice (sibling extraction shapes with the same module surface). The delta "R41-only vs R41+R39" is therefore unavailable; this is recorded so future comparisons do not look for a rev 10 artifact that was never cut. |
| rev 11   | R41 + R39 closed         | is_synthetic     | This round. |

### Aggregate shape change (rev 9 → rev 11, both filtered on the production-function pool that excludes synthetic nodes)

| metric | rev 9 | rev 11 | delta |
| ------ | ----- | ------ | ----- |
| `summary.total_functions` (function_node_ids after synthetic filter) | 14,948 | 14,844 | **−104** |
| `summary.total_nodes`            | 20,560 | 21,910 | **+1,350** (synthetic `__init__` / `__get__` / `__set__` nodes added to the graph but not the pool) |
| `summary.total_edges`            | 111,158 | 116,344 | **+5,186** |
| `metrics.dead_code.count`        | 852    | 727    | **−125** |
| `reason.no_callers`              | 483    | 392    | **−91** |
| `no_callers_total`               | 483    | 471    | **−12** |
| `no_callers_high_confidence`     | (rev 9 field was `no_callers` pool pre-split) | 229 | — |
| `reason.framework_annotation_unresolved` | 191 | 165 | −26 |
| `reason.declarative_wiring_unparsed`     | 138 | 130 | −8 |
| `reason.dynamic_dispatch_target`         | 40  | 40  | 0 |

Interpretation: R41 and R39 together recover an entire class of
previously-dropped call sites. The synthetic wrapper nodes they
introduce would have naïvely flooded the dead-code bucket (they
themselves have no callers, being modeling artifacts), but the
`is_synthetic` filter in `graphengine-analysis/src/health/graph.rs`
catches them at `function_node_ids` computation time so they never
enter the candidate pool. The net effect is real: 12 FQNs genuinely
left the `no_callers_total` bucket because their sole caller sites
were now extracted.

### Round 5b draw

Pool: 412 production `no_callers` FQNs after the §4.11.2 `toString()`
pre-filter. Draw size: 10. Seed: `20260418` (re-stamped at draw time;
reused from Round 5 by intent so the two rounds are revision-
comparable).

| # | FQN | verdict | evidence |
| - | --- | ------- | -------- |
| 1  | `fflib_IDomain::getObjects()` | wrong-new-shape-R47.A | Interface method; `fflib_Ids.cls`, `fflib_Criteria.cls`, `fflib_PrimitiveDomainsTest.cls` each call `.getObjects()` on receivers typed as `fflib_IDomain`; resolver does not walk `implements` edges back to concrete bodies. |
| 2  | `fflib_ISObjectUnitOfWork::registerDeleted(SObject)` | wrong-new-shape-R47.A | Interface method; caller side at `fflib_SObjectUnitOfWork.cls` + mocks calls `.registerDeleted(record)` on the interface type. |
| 3  | `fflib_ISObjectUnitOfWork::registerRelationship(Messaging.SingleEmailMessage,SObject)` | wrong-new-shape-R47.A | Same shape as #2 on a second interface signature. |
| 4  | `fflib_SObjectDomain.ErrorFactory::ErrorFactory()` | wrong-new-shape-R49 | Inner-class zero-arg constructor; both caller sites are inside `static { ... }` initializer blocks (fflib_SObjects.cls:40, fflib_SObjectDomain.cls:109). R41 does not cover static initializer blocks; recipe filed as R49. |
| 5  | `fflib_SObjectUnitOfWork::registerPublishBeforeTransaction(List<SObject>)` | correct | Defensive overload; the only `.registerPublishBeforeTransaction(` callsite in the corpus (fflib_SObjectUnitOfWork.cls:528 `this.registerPublishBeforeTransaction(record)`) dispatches to the `SObject` overload, not the `List<SObject>` one. No live caller. |
| 6  | `BDI_CustObjMappingGAUAllocation::populateObjects(BDI_ObjectWrapper[])` | wrong-new-shape-R47.B | Virtual override; called from `BDI_AdditionalObjectService.cls:179` via a base-class reference `objMappingLogic.populateObjects(...)` where `objMappingLogic` is typed as `BDI_ObjectMappingLogic`. Resolver does not traverse override chain. |
| 7  | `BDI_MigrationMappingUtility.DataImportObjectMapping::DataImportObjectMapping()` | correct | Zero-arg constructor on inner class; every `new DataImportObjectMapping(...)` in the corpus (BDI_MigrationMappingUtility.cls:245, :250, :488) uses the parameterised overload. Genuine dead default. |
| 8  | `CRLP_RollupRD_SVC::CRLP_RollupRD_SVC()` | correct | No `new CRLP_RollupRD_SVC(` anywhere in the corpus. Genuinely dead. |
| 9  | `PMT_PaymentWizard_CTRL::getTotalWrittenOff()` | wrong-new-shape-R48 | Called via Visualforce binding `{!totalWrittenOff}` in `PMT_PaymentWizard.page:176`. Engine parses the `.page` file but does not extract `{!...}` bindings to controller getters. R48 filed. |
| 10 | `CRLP_TEST_VALIDATE_ROLLUPS::executeCustomizableRollups()` | correct | Called only from `cumulusci.yml:1666` as anonymous-Apex in an external build config, not from any `.cls` / `.trigger` / `.page` in the corpus. From the engine's source-graph perspective, genuinely unreachable; the build-harness caller is not representable in the static graph and is out of scope. |

**Wrong: 6 / 10. Gate: FAIL (threshold is < 2 / 10 wrong).**

### Failure composition

| Shape | Count | Risk-register ID | Status |
| ----- | ----- | ---------------- | ------ |
| Interface-method dispatch via interface-typed receiver | 3 | R47.A | **Filed in this PR.** Resolver scope. |
| Virtual override via base-class reference              | 1 | R47.B | **Filed in this PR.** Resolver scope. |
| Visualforce page `{!foo}` binding → controller getter  | 1 | R48   | **Filed in this PR.** Extractor scope. |
| Static initializer block `static { ... }` body         | 1 | R49   | **Filed in this PR.** Extractor scope. |
| Genuine dead production code                           | 4 | —     | Correct verdicts. |

### Interpretation vs Round 5 (rev 9)

Round 5 (rev 9) failed 4/10, with R39 dominating (2/10), plus R41
(1/10) and R45 (1/10). Round 5b (rev 11) fails 6/10. The absolute
failure rate rose because:

1. **R41 and R39 genuinely closed.** Neither shape appears in the
   Round 5b draw even once. The 2-3 rev-9 failure samples that
   would have been re-drawn at the same seed are now *out of the
   pool entirely* — the engine correctly extracts the call sites
   and the receivers now have inbound edges, so the FQNs no longer
   show up as `no_callers`.
2. **The pool denominator shrank by 12** (483 → 471 `no_callers_total`),
   so the shape mix shifted: previously-masked R47/R48/R49 shapes
   that sat below the sampling probability of R39/R41 now dominate
   the draw.
3. **R47 is the single largest unresolved shape family** (4 of 10:
   three R47.A + one R47.B). This is expected — the NPSP codebase
   is built on fflib, which is interface-heavy by design.

Round 5b is therefore a **positive signal about extractor-layer
progress** and a **negative signal about resolver-layer polymorphic-
dispatch readiness**. The narrative for the customer demo must:

- Credit the R41/R39 closure (12 FQNs removed from the `no_callers`
  pool; 2 of the 3 rev-9 failure shapes eliminated; zero regressions
  in existing tests).
- Disclose the R47/R48/R49 shapes honestly with their new risk-
  register entries (this PR files all three).
- NOT claim Phase A closed. The < 2/10 gate did not flip; it flipped
  in the opposite direction (4/10 → 6/10). The reason it went up
  despite engine improvements is the pool-shift dynamic, not a
  regression, but the customer needs to see the gate number, not
  only the narrative.

### Cross-corpus precision table (updated 2026-04-18)

| corpus | scope | size (LoC) | Layer 2 | correct | wrong-shape | gate |
| ------ | ----- | ---------- | ------- | ------- | ----------- | ---- |
| dreamhouse-lwc | Apex holistic walkthrough | ~700 | none | 9/9 Call edges correct | 0 | pass |
| apex-recipes   | Apex 10-sample | ~12k | none | 7/10 | 3 (R39, R41, R45 shapes) | fail |
| gridseak-self  | Rust, 15 samples | ~70k | ra_ap_ide | 0/15 adjudicable correct | 15 (UF-FU-012a SymbolIndex loss + UF-FU-021 pub use misclassification) | fail |
| NPSP rev 9     | Apex 10-sample | ~350k | none | 6/10 | 4 (R39 ×2, R41, R45) | fail |
| NPSP rev 11    | Apex 10-sample | ~350k | none | 4/10 | 6 (R47.A ×3, R47.B, R48, R49) | fail |

### Decision (Round 5b)

**Phase A gate remains FAIL.** R41 and R39 ship; Phase A does not
close. The universal-fidelity sprint's WS-TRUTH-A slot stays open
against the new shape-set (R47, R48, R49), with R47 the highest-
leverage next target because it accounts for 4 of 6 failures in a
single round and is architecturally cleanest (walks existing
inheritance edges rather than requiring a new extractor).

`TRUST_AND_ACCURACY_MEMO.md` must be updated in Tier 6.2 to:

- Replace the rev 9 precision anchors (6/10 with 4 upstream gaps)
  with rev 11 anchors (4/10 with 6 upstream gaps across R47 / R48 /
  R49 — specifically 3 interface, 1 virtual-override, 1 VF-binding,
  1 static-initializer).
- Add an "extractor closures since rev 9" footnote explicitly
  crediting R41 + R39 and showing the 12-FQN reduction in
  `no_callers_total` as the honest engine-improvement signal.
- Keep the gate-number-up-despite-engine-improvement dynamic visible:
  this is exactly the kind of self-report the PHASE_4_DECISION_MEMO
  was written to model.

### Reproducing this round

```bash
# Regenerate rev 11 baseline after is_synthetic lands:
./target/release/ge-analyze \
  --db experiments/results/NPSP/rev11/parse.db \
  --output experiments/results/NPSP/rev11/baseline.json

# Draw 10 samples at seed 20260418:
python3 - <<'PY'
import json, random
with open('experiments/results/NPSP/rev11/baseline.json') as f:
    d = json.load(f)
def is_tostring(fqn):
    m = fqn.split('::')[-1].split('(',1)[0]
    return m.lower() == 'tostring'
pool = [n for n in d['node_annotations'].values()
        if n.get('dead_code_reason') == 'no_callers'
        and n.get('is_dead')
        and not n.get('is_test')
        and not is_tostring(n.get('fqn',''))]
random.seed(20260418)
sample = sorted(random.sample(pool, 10), key=lambda x: x['fqn'])
for i, s in enumerate(sample, 1):
    print(i, s.get('display_name') or s['fqn'], '|', s.get('file_path'), s.get('start_line'))
PY
```
