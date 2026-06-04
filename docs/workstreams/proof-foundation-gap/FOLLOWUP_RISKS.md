# Proof Follow-up Risks & Integrity Concerns

> **Reproducing historical numbers / paths cited below.** Neither the historical baseline JSONs / calibration outputs nor the rev6.1 byte-identical regression fixture referenced in this document are tracked in git — both live as sha256-pinned GitHub release assets. Fetch on demand with `scripts/setup.sh historical-baselines` (rev3..rev11 evidence, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/baseline-archive-2026-05-18)) and `scripts/setup.sh fixtures` (rev6.1 regression fixture, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/regression-fixtures-2026-05-19)). All artifacts are pinned in `experiments/artifacts.lock`. The active build/test loop does not require any of them.

> Companion to `docs/workstreams/proof-foundation-gap/FINDINGS.md`. Records concerns flagged during the
> Proof-of-Foundation-Gap experiment that do not neatly fit the two
> workstreams (A: orchestration bug fix, B: foundation/framework resolvers)
> but must be addressed to avoid regressions of trust.

Source: the proof experiment surfaced issues beyond the headline bug. These
are architectural / product-integrity risks. They are distinct from the bug
itself and should not be bundled into the same PR.

## R1. Desktop UI reports metrics without expressed confidence

**Observation.** The desktop surface renders raw `HealthReport` metric values
(cycles, tangle, dead-code, depth, coupling) as plain numbers. No confidence
band, no "data sufficiency" marker. This is how `cycles_found = 0` passed
visual review on multiple scans across multiple projects without anyone
noticing.

**Risk.** A customer scanning their own repo sees a definitive-looking "0
cycles" and forms false trust. When they later discover cycles via any other
tool (or via their own intuition of their codebase), the entire product's
credibility is damaged — in exactly the audience that will try to reverse-
engineer or disprove the engine (engineering pilot users).

**Recommendation.** The fix for the orchestration bug and the introduction
of confidence bands should ship together, not in separate releases. A
`cycles_found = 0` result is never meaningful without a companion
"edge-graph sufficiency" signal.

## R2. `tangle_index = 0.000` on sparse graphs is architecturally misleading

**Observation.** Even after the orchestration bug is fixed, `nextjs-commerce`
(403 production edges) and `django-site` (231 production edges — artificially
low because of the `urls.py` gap) will still report `tangle_index = 0.000`.
That's numerically correct but tells the user the opposite of the truth: the
codebase isn't untangled, it's unobserved.

**Risk.** Same credibility risk as R1, but harder to fix with a mechanical
confidence band because the metric itself is ambiguous.

**Recommendation.** Every metric should carry an explicit status:

```
{ "status": "ok" | "insufficient_edges" | "framework_invisible" | "not_applicable",
  "value": 0.000,
  "description": "..." }
```

When `status != "ok"`, the desktop UI must refuse to render the raw number
and instead render the status reason. This prevents false precision from
propagating.

## R3. `resolution_quality` is not prominent in the current UI

**Observation.** `djangoproject.com` reports `100 % heuristic fallback` in
the baseline scan — meaning name-matching only, no LSP. Users scanning a
Python project without the Python LSP running are getting name-match
results that look identical to LSP-backed results in the UI.

**Risk.** Silent degradation in resolution quality will cause customer
churn when they compare our metrics against a colleague's LSP-backed run.

**Recommendation.** Elevate `resolution_quality` to a first-class header
indicator in the desktop — not buried in a details panel. If heuristic
fallback exceeds a threshold (e.g., 30%), block the "scan complete" UI and
show an actionable message: "Install Python LSP for accurate results."

## R4. Historical telemetry is corrupt for cycles/tangle

**Observation.** If any `HealthReport` has been persisted to cloud, CI, or
customer dashboards prior to the orchestration-bug fix, every one of those
records will have `cycles_found = 0` and `tangle_index = 0.0` regardless of
the underlying codebase.

**Risk.** If we ever compute trend-lines, historical comparisons, or "your
code got better over time" visualizations, the data before the fix is
systematically wrong. Silently overwriting with corrected values after the
fix loses the provenance.

**Recommendation.**
- Stamp every `HealthReport` with the `graphengine-analysis` crate version
  AND a `schema_caveats` list (e.g., `["cycles_pre_orderfix_v0_1_x"]`).
- When displaying trends, visually demarcate the boundary where the engine
  changed versions / caveats.
- Never silently backfill; always re-scan explicitly.

### Resolution (shipped in the bug-fix-and-integrity release)

Every `HealthReport` now carries an `IntegrityStatus` block with a locked
set of caveat string constants. The authoritative definitions live in
[`graphengine-analysis/src/health/report.rs`](../../../graphengine-analysis/src/health/report.rs):

| constant                             | literal value                    | meaning                                                                                                                                                     |
| ------------------------------------ | -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CAVEAT_CYCLES_ORDERFIX_APPLIED`     | `"cycles_orderfix_applied"`      | Stamped on every report produced by an engine that includes the cycle-detection ordering fix. Absence (in any persisted historical record) indicates the report was produced by a buggy pipeline. |
| `CAVEAT_METRIC_STATUS_CONTRACT`      | `"metric_status_contract_v1"`    | Stamped on every report emitted by an engine that populates the `MetricStatus` contract. Reports without this caveat are "legacy shape" and cannot be trend-compared safely. |
| `CAVEAT_LEGACY_PRE_ORDERFIX`         | `"legacy_pre_orderfix"`          | **Not emitted by this engine.** Defined for downstream use: when a persistence layer encounters a report with no `integrity_status` field at all (deserialized from pre-contract JSON), it must synthesize this caveat for internal bookkeeping. |

**Invariants these constants commit to, forever:**
- The literal strings are frozen. They MUST NOT be renamed. They are the
  contract between any engine version and any historical record.
- The list is append-only. Adding a new caveat is a minor additive
  schema change; removing or renaming one is not.
- Matching code in downstream tools (desktop UI, cloud dashboards, CI
  gates) MUST match on the constants, not on the literal strings, so a
  future breaking rename gets caught at compile time.

**Downstream convention for legacy data (no server-side schema changes required):**
1. When reading a persisted report that has no `integrity_status` field,
   treat it as if `schema_caveats = [CAVEAT_LEGACY_PRE_ORDERFIX]` and
   `engine_version = "unknown"`.
2. In any trend view, reports tagged (implicitly or explicitly) with
   `CAVEAT_LEGACY_PRE_ORDERFIX` must be visually demarcated and excluded
   from regression statistics. The desktop UI's `IntegrityBanner`
   component already honors this by rendering the "Legacy report" warning
   when the caveat list is empty or absent.
3. Migration path is explicit re-scan (user re-runs analysis on the same
   codebase). No silent backfill of historical records.

## R5. `HealthReport` has no self-check invariants

**Observation.** Nothing in the analysis pipeline asserts basic invariants
like "if `clean_structural_edge_indices.len() > 0`, then
`production_structural_edge_indices.len() > 0`." The orchestration bug
could have been caught on day one by an assertion this simple.

**Risk.** Future ordering bugs, refactors, or feature toggles can
reintroduce zero-edge-in-production states silently. Every metric
downstream of `production_structural_edge_indices` (cycles, depth, dead
code, tangle, blast-radius, fan-in/out in production-only mode) depends on
it.

**Recommendation.** Add a `validate_invariants()` method on `AnalysisGraph`
called at the boundary of every phase of `run_analysis`. Failures are
`debug_assert!` in dev and are **emitted as `AnalysisError` entries** in
release — so the desktop can surface them.

## R6. Unit-test coverage did not catch the orchestration bug

**Observation.** `cycles.rs` has tests for `detect_cycles` in isolation that
pass (they call `graph.finalize_production_edges()` manually in `make_graph`
test helpers). The production pipeline ordering was never end-to-end
tested. That is how the bug escaped.

**Risk.** Any bug that lives in the *composition* of components, not inside
any single component, is invisible to the current test strategy. This is a
systemic test-architecture gap, not a one-off miss.

**Recommendation.** Add a new test layer: end-to-end tests over the whole
`run_analysis` entry point, using small hand-crafted parse DBs with known
expected metric values (cycle count, depth, dead-code count). Minimum three:
known 2-cycle, known deep chain, known dead function. Run in CI.

## R7. "Static-only" metrics mask themselves on declarative-heavy frameworks

**Observation.** A Django or Rails app can genuinely have a high tangle
index, deep call depth, and significant coupling in reality, but the engine
will emit plausible-looking near-zero values because the parsed call graph
is missing ~95% of the dispatch edges (see gap table in `FINDINGS.md`).

**Risk.** Selling the engine on a pilot involving a Django or Rails
customer will show them a misleadingly "healthy-looking" report. They will
trust it briefly, then distrust it permanently when the product exposes
production complexity the engine denied.

**Recommendation.** Until Workstream B's declarative-wiring resolvers ship,
the desktop should display an explicit warning banner on Django/Rails/Spring
scans: "Framework-aware resolution unavailable. Metrics underestimate
complexity for this codebase." This is uncomfortable but honest — and it's
the right way to avoid Risk R1+R2+R3 compounding.

## R8. Foundation-gap proof relied on manual edge injection, not a repeatable benchmark

**Observation.** `experiments/ab_inject/inject.py` was a one-off for NPSP.
There's no comparable benchmark for Django view dispatch, Rails routes, or
Spring beans. When Workstream B ships a resolver, we need a way to measure
coverage gain per framework.

**Risk.** Without a benchmark, Workstream B becomes impossible to evaluate
objectively. Different engineers will ship different resolvers without a
common measuring stick.

**Recommendation.** Elevate `experiments/audit_tool/` and `gap.py` to
first-class benchmark status — not throwaway. Run them in CI on a small set
of canary repos (NPSP snapshot, `djangoproject.com` snapshot, `rails-app`
snapshot) every build, track the missed-edge count over time, and block
merges that regress it.

<!--
  R9–R17 were surfaced during the dead-code categorization
  investigation (see `.cursor/plans/dead-code-reason-classifier`).
  They are the honest aftermath of the Workstream B regression
  failure: the ≥25% dead-code drop prediction missed because our
  aggregate metric conflated "truly unused" with "framework-invisible".
  Every item below is either resolved in that plan, or explicitly
  deferred to a downstream workstream with the scope named.
-->

## R9. Dead-code aggregate conflates "unused" with "framework-invisible"

**Observation.** `MetricsReport.dead_code.count` sums every function
with zero parsed fan-in that survives the entry-point filter. A
function exempt from static analysis (Apex `@AuraEnabled`, Django
URLconf-referenced view, Celery `@task`) contributes the same 1 to
that total as a genuinely orphaned helper. The Workstream B
regression prediction ("≥25% dead-code drop after TDTM injection")
failed because most of NPSP's static-dead set is framework-invisible,
not reflection-dispatched; aggregate movement of 5.1% buried the
fact that the framework-invisible bucket moved a lot more than that.

**Risk.** Every customer trusting the dead-code total as "delete
candidates" is exposed to a silent accuracy collapse the moment
their ecosystem's framework dispatch isn't resolved.

**Resolution (shipped in the `dead-code-reason-classifier` plan).**
Per-function `DeadCodeReason` + evidence string, aggregate
`reason_breakdown` histogram, and `CAVEAT_DEAD_CODE_REASONS_V1`
stamp on every report. See
`graphengine-analysis/src/health/dead_code_classifier/mod.rs` and
`graphengine-analysis/src/health/report.rs:DeadCodeReason`.

**Next step.** Retire the `framework_annotation_unresolved` and
`dynamic_dispatch_target` buckets by emitting `FrameworkEntry`
edges in the resolver so these symbols become `live`. Enumerated
plan: `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` (Wave 3.1).

## R10. `is_attribute_invoked` is a bare bool — discards which attribute matched

**Observation.** The parsing crate emits `properties.entry_points` as a
list of strings (`["aura_enabled", "invocable_method"]`, …), but the
analysis crate collapses that down to a single
`GraphNode.is_attribute_invoked: bool`. A user asking "why is this
function exempt from dead-code?" gets nothing to read.

**Risk.** Debuggability: when an Apex customer disagrees with the
classifier ("this `@Schedulable` was flagged dead, but it's running
in production!") we cannot show them the signal path the engine took.

**Resolution.** `GraphNode` now carries `entry_point_tags: Vec<String>`
alongside the bare bool, populated from the `entry_points` property.
The Apex classifier consumes the tag list and includes it in the
evidence string. See
`graphengine-analysis/src/health/graph.rs:GraphNode` and
`graphengine-analysis/src/health/dead_code_classifier/apex.rs`.

## R11. `is_entry_point` short-circuits without recording which rule fired

**Observation.** `entry_points::is_entry_point` returns `bool`. When
a function is exempted, the exempting rule (barrel file? framework
handler suffix? extra pattern?) is lost.

**Risk.** Identical to R10 in spirit — opacity. The classifier needs
to know *which* entry-point rule matched to produce an honest
evidence string; re-implementing the decision tree would double the
surface area that has to stay in sync.

**Resolution.** New `EntryPointReason` enum + `classify_entry_point`
function in `graphengine-analysis/src/health/entry_points.rs`.
`is_entry_point` is now a thin `.is_some()` shim.

## R12. `FRAMEWORK_HANDLER_SUFFIXES` is suffix-string matching with no ecosystem scoping

**Observation.** `entry_points.rs:FRAMEWORK_HANDLER_SUFFIXES` (e.g.
`handler`, `controller`, `middleware`, `listener`) is applied
identically to every ecosystem. In Go, `Handler` is an interface-
method convention (`ServeHTTP` is in lifecycle methods for this
reason, but arbitrary HTTP handler functions slip through). In C#,
`Controller` is a naming convention enforced by ASP.NET MVC that
should trip a stronger signal than "name ends with controller".

**Risk.** False-exempt: a Go helper method named `cleanupHandler`
that does nothing but internal bookkeeping looks like an entry point
and never becomes a dead-code candidate, regardless of fan-in.

**Resolution.** The dead-code classifier's ecosystem-specific
modules (Apex, Python) now own the framework-convention rules for
their language. As Go, Java, and C# classifiers are added
(`dead_code_classifier/<lang>.rs`), their rules supersede the
generic suffix list. Generic fallback retains the suffix match as a
last resort when no specific classifier is registered.

## R13. NPSP has ≥9 Apex-specific dispatch idioms beyond TDTM

**Observation.** Workstream B's resolver plan focused on TDTM
reflection. NPSP additionally depends on `@AuraEnabled`,
`@InvocableMethod`, `@RestResource`, `@HttpGet`/`@HttpPost`,
`@Schedulable`, `@Batchable`, `@Queueable`, `@RemoteAction`,
`.trigger` files, `@isTest` classes, `global`/`webservice`
modifiers, and `implements Messaging.InboundEmailHandler`. Shipping
a TDTM-only resolver would leave the "framework-invisible" bucket
almost unchanged.

**Risk.** Declaring Workstream B "done" after TDTM under-delivers
on the bug-fix-and-integrity release.

**Resolution.** The Apex classifier — after Wave 2's split into
per-framework rule sets
(`graphengine-analysis/src/health/dead_code_classifier/frameworks/`) —
covers all of the above as `FrameworkAnnotationUnresolved`, with
TDTM specifically as `DynamicDispatchTarget`. Workstream B's
edge-emission resolver remains a separate deliverable, but it is
no longer required in order to give users an accurate breakdown.

**Next step.** The 11-idiom dispatch matrix has been promoted
into `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` §3 as the authoritative
edge-emission contract for Wave 3.

## R14. `DeadCodeResult` loses positive-classification information

**Observation.** `dead_code::detect_dead_code` knows *why* it spared
a function (the entry-point rule that fired), but throws that
information away. The classifier would like to reuse it rather than
re-walk the decision tree per node.

**Risk.** Decision duplication — the dead-code filter and the
reason classifier can drift out of sync under future edits.

**Resolution.** Not yet acted on beyond R11's refactor. The
classifier consumes `entry_points::classify_entry_point` directly
instead of re-implementing any decision arm. A future optimisation
(not blocking) is to have `detect_dead_code` emit per-node
(`dead` | `entry-point-reason`) tuples so classification doesn't
require a second classification pass.

## R15. Benchmark has no dead-code reason distribution gate

**Observation.** Before this plan, `tools/benchmark/diff_against_baseline.py`
only gated totals (cycles, dead-code aggregate, tangle). A future
bug that relabelled framework-invoked functions as `no_callers`
would leave aggregate totals untouched and slip through CI.

**Risk.** The exact class of regression the bug-fix-and-integrity
release exists to prevent, but on a different axis.

**Resolution.** `diff_against_baseline.py` now asserts:
1. `no_callers` cannot jump by more than max(3, 25%) between
   baseline and current — a sharp rise is suspicious.
2. A fixture with `framework_annotation_unresolved > 10` going to 0
   surfaces a NOTE (not a failure — a resolver may legitimately
   land) so a human verifies the shift.
3. Per-fixture ground-truth files
   (`tools/benchmark/fixtures/<fixture>/reason_ground_truth.json`)
   are validated with an absolute tolerance.

## R16. Python Django dispatch is invisible — risk of false dead-code claims

**Observation.** Django URLconf (`urls.py`) resolves routes to view
functions via string paths. The parser never emits a `Call` edge
from `urls.py` to `views.py:MyView.get`; the dispatch happens at
request time. Before this plan, NPSP-style "framework-invisible"
reporting did not exist for Python, so Django views were silently
classified as `no_callers`.

**Risk.** A Django customer scanning their site sees plausible
dead-code numbers that are almost entirely views.

**Resolution.** The Python classifier
(`dead_code_classifier/python.rs`) flags views.py / management
commands / tasks.py functions as `DeclarativeWiringUnparsed` with
an explicit evidence string ("caller is likely a urls.py path()
binding that the parser did not resolve"). A full URLconf resolver
remains Workstream B; the classifier is the honest interim.

<!--
  R18–R22 were surfaced while re-running the NPSP regression with
  the dead-code reason classifier shipped. They are pre-existing
  defects the classifier made visible — not caused by it.
-->

## R18. File `is_test` flag is missing on NPSP `*_TEST.cls` classes

**Observation.** When the Apex classifier relied only on
`File.is_test` to pick out test classes, 3,088 methods inside
`*_TEST.cls` NPSP test classes were classified as `no_callers`
rather than `test_only_reference`. The File-classification layer
is either not parsing the class-level `@IsTest` annotation on
those files, or the parser's file-level `is_test` decision is
deferred elsewhere.

**Risk.** Every NPSP-style Apex codebase (Salesforce DX projects,
which follow this convention universally) would see its test
methods rolled into the "dead production functions" bucket. The
headline number is silently inflated by thousands.

**Interim resolution.** The Apex classifier now also recognises
the `*_TEST` filename convention as a test signal (see
`dead_code_classifier/apex.rs::parent_is_test`). This is a
classifier-level safety net. The authoritative fix is for the
parser to set `File.is_test = true` when it sees `@IsTest` at the
class level *or* when the file stem matches the universal Apex
convention. File a ticket in the parsing crate.

## R19. Parser emits Function nodes for static-resource minifier tokens

**Observation.** NPSP's `StaticResourceSources::moment::moment.min::ja`,
`jquery_3.5.0.min::U`, `typeahead.bundle.min::d`, etc. are emitted
as Function nodes by the parser. They are minified-JavaScript
identifier tokens inside static resource bundles, not code
functions. 924 of them survive the load, enter the dead set, and
would have swelled `visibility_private_unused` had the non-
production filter not caught them first.

**Risk.** Static-resource tokens are counted as functions in
`graph.total_functions()`, inflating the denominator of every
ratio metric (dead ratio, fan-in average, fan-out average). The
non-production filter keeps them out of the headline aggregates,
but the underlying graph model is contaminated.

**Recommendation.** The parsing crate should stop emitting
Function nodes for content inside `.min.js`, `.bundle.min.js`,
and similar minified-resource patterns. Files under
`staticresources/` or `force-app/*/staticresources/` are
resource bundles by SF DX convention and should be classified
`is_vendor` or `is_build_output` at the File-node layer so the
extractors skip them entirely.

## R20. `MetricsReport.dead_code.count` and `reason_breakdown` scope must match

**Observation.** An initial wiring of the classifier registry
iterated `DeadCodeResult.dead_node_ids` (all dead nodes, including
test classes and static-resource tokens) and summed them into
`reason_breakdown`. The headline `dead_code.count` however is
`prod_dead_count`, which filters via `is_non_production_node`.
These two numbers must be identical or the UI chart overflows
the headline it decorates.

**Risk.** Integrity: if a user sees "2,452 dead functions" and a
chart labelled "dead-code reason distribution" that sums to
5,972, they reasonably lose trust in both numbers.

**Resolution.** `run_analysis` now classifies the full dead set
(so per-node annotations still cover everything for drill-down),
then filters to the production subset before summing the
`reason_breakdown`. Both numbers now match. See the "Scope
contract" comment in `graphengine-analysis/src/health/mod.rs`
at the classifier dispatch site.

## R21. `production_structural_edge_indices` depends on correct ecosystem

**Observation.** The cycle-detection path builds
`production_structural_edge_indices` by filtering on each node's
production classification. When ecosystem detection is wrong
(see R22), downstream depth / cycles / tangle calculations
operate on the wrong subset. On NPSP this manifested as the
engine running full pipelines labelled "javascript" even though
74% of the code was Apex.

**Risk.** Every config-per-ecosystem threshold (coupling,
cohesion, depth) applies the wrong knob values. Integrity
statuses become misleading because their "ok" bounds are
calibrated against a different ecosystem.

**Resolution.** File-majority ecosystem detection (R22) fixed
the specific NPSP mis-tag. The broader defence — per-node
ecosystem routing so polyglot repos get correct treatment per
file — remains a separate workstream. Tracked as a future
enhancement to `ClassifierRegistry::classify_batch` to consult
each node's owning-file language rather than a single global
ecosystem.

## R22. `detect_ecosystem` trusted the Project node's `language` unconditionally

**Observation.** `graph::detect_ecosystem` short-circuited on
the Project node's language property before consulting File-
majority. NPSP's Project node wrote `language = "javascript"`
(LWC components are the only JS the project has, but the parser
picked JS for the Project label). 1,071 of 1,441 File nodes
reported `language = "apex"` — 74% of the codebase — yet the
engine behaved as if it were a JavaScript project. The Apex
classifier never dispatched.

**Risk.** Polyglot projects (any repo with multiple languages)
are one-bit away from a completely wrong ecosystem tag. The
Project node's language is written by one of the ingestion
paths (whichever runs last wins) and is not authoritative.

**Resolution.** `detect_ecosystem` now prefers the File-majority
language (≥60% threshold), then the Project node, then plurality
with no threshold. A mismatch between Project and File-majority
emits a `[ge-analyze] WARNING:` line so the operator can verify.
See `graphengine-analysis/src/health/graph.rs::detect_ecosystem`.

## R17. Pre-classifier historical reports have no reason breakdown

**Observation.** Every report produced before the classifier ships
has `dead_code.count` but no `dead_code.reason_breakdown`. Trend
tooling that wants to show a time-series of `no_callers` vs
`framework_annotation_unresolved` can't run across the boundary.

**Risk.** Trend / regression products built on top of the
classifier will either show gaps or implicitly assume
`{Unclassified: count}`.

**Resolution.** `report.rs:CAVEAT_DEAD_CODE_REASONS_V1` is stamped
on every post-ship report. Downstream tooling treats its absence as
"the entire dead_code total is Unclassified" and surfaces a warning
instead of misattributing mass to `no_callers`. The Desktop UI
renders an explicit "this report predates the classifier" banner
when the caveat is absent — see
`desktop/ui/src/components/ReportStep.tsx:DeadCodeReasonBreakdown`.

## R23. Apex heuristic resolver misses basic constructor and typed-field dispatch

**Observation.** Wave 1 Layer-5 hand-audit round 2 sampled 10
FQNs from the `no_callers` bucket (NPSP, rev 4 baseline,
`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`). 7 of 10 had real callers in the source
tree that `graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs`
failed to link. Patterns in the miss set:

- `new X(...)` calls where `X` is declared in the same file
  (`new Logger(...)` @ `RD2_DataMigrationBase_BATCH.cls:172`,
  `new BatchJob(...)` @ `UTIL_JobProgress_CTRL.cls:129`).
- `new X(...)` calls where `X` is declared in a different file
  (`new HouseholdMembers(...)` @ `HouseholdNamingService.cls:257`).
- `instance.method(...)` dispatch where `instance` is a field
  with a declared type in the same class
  (`permissionsService.canUpdate(...)` across multiple files).
- Intra-class overload dispatch
  (`fflib_Comparator.compare(String,String)` called from the
  `compare(Object,Object)` overload in the same class).

**Risk.** Any Apex repo's `no_callers` production bucket is
inflated by a mass of "dead" functions that are in fact
normally-called but parser-invisible. The engine reports them as
unused code. Customers who ship an Apex codebase would see false
dead-code findings on basic patterns (constructors, service
calls) — the exact class of finding that destroys product trust.
The Wave 1 hand-audit gate cannot be met on this bucket without a
resolver fix; Wave 2 (classifier rewrite) does not touch the
resolver.

**Recommendation.** Owned by Wave 3 of the
truthful-scans-simplification plan: the Apex Framework Resolver
explicitly takes over from the heuristic resolver for Apex, with
a name-resolution pass that covers at minimum:

1. Intra-class constructor calls and sibling overloads.
2. Field-type-aware method dispatch (parse field declaration,
   carry the declared type through `instance.method()`).
3. Cross-file type resolution via the pre-computed class registry
   (`graphengine-parsing/src/syntax/language/apex/class_registry.rs`
   already builds this — the resolver just doesn't consult it
   consistently).

**Phase A of the truthful-scans roadmap (TR-A.0 foundation + TR-A.1 – TR-A.6, see
`docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`
§4).** Phase A gates Phase B (the full Apex Framework Resolver
enumerated at `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`)
because framework-entry edges must resolve into a correct class
registry. The 7 Wave-1 Round-2 + 10 Wave-2 Round-3 audited FQNs
are Phase A's concrete regression fixtures.

## R24. `looks_like_tdtm_handler` matches on parameter-type fragments

**Status.** Resolved by TR-0.1 (Engine rev 6, pending NPSP re-parse
+ Layer-5 Round 4). See the TR-0.1 commit touching
`graphengine-analysis/src/health/dead_code_classifier/frameworks/tdtm.rs`.

**Observation.** `graphengine-analysis/src/health/dead_code_classifier/frameworks/tdtm.rs::looks_like_tdtm_handler`
used to test `fqn_lc.contains("tdtm_") || fqn_lc.ends_with("_tdtm")`
against the full FQN. Because Apex FQNs include parameter-type
signatures (`onAfterUpdate(TDTM_Runnable.DmlWrapper)` contains
`tdtm_`), methods that merely *accept* a TDTM type matched the
rule. The Wave 1 Round 2 audit caught
`AccountAdapter::onAfterUpdate(TDTM_Runnable.DmlWrapper)`
classified as `dynamic_dispatch_target` purely on the parameter
type.

**Risk (pre-fix).** The TDTM dynamic-dispatch bucket was falsely
inflated whenever a non-handler method accepted TDTM types. For
NPSP the miss rate was small (1/10 in Round 2) but the pattern was
systematic.

**Resolution (TR-0.1).** The heuristic now decomposes the FQN
before matching: the parameter tuple is stripped on the first `(`,
the class and method segments are isolated via `rsplit("::")`, and
the TDTM-convention match runs only against class-segment tokens
(`tdtm_` prefix, `_tdtm` suffix, or exact `tdtm`). The
run-on-handler fallback is likewise scoped to the class segment.
See the module docstring in `tdtm.rs` and the positive/negative
tests added alongside the fix. R26 (full declarative rule engine)
still supersedes this heuristic in Phase D; the TR-0.1 fix lives
on the same path and will be absorbed by that migration.

**Discovery task (Layer-5 Round 4).** Confirm the three
documented class-name shapes (`TDTM_<X>`, `<X>_TDTM`, bare `TDTM`)
cover every TDTM handler in NPSP rev 6. If the audit surfaces an
outlier without an underscore separator (hypothetically
`CON_ContactMergeTDTM`), record it as a new R# with the sampled
FQN attached. Do not pre-add detection rules for shapes we cannot
evidence in real data — that re-opens the R24 false-positive
class. Current token rule is intentionally conservative.

## R25. LWC template bindings are not edges in the call graph

**Observation.** Wave 1 Round 2 hand-audit sampled 10 FQNs from
the `visibility_private_unused` bucket in NPSP (rev 4); all were
private methods on LWC JavaScript modules (e.g.
`geSoftCredits::update`, `rd2Service::withAchData`). Inspection
of the paired `.html` templates shows every sampled method is
bound via `on<event>={method}` attributes. The JavaScript parser
emits no edge from the template to the method, so the methods
appear unused.

**Risk.** Every LWC-heavy repo will have this bucket inflated by
the count of template-bound handlers. LWC is the modern Salesforce
UI framework — the bias is systematic, not marginal.

**Recommendation.** Add an LWC template parser pass that, for each
`<lwc-module>/<name>.html`, emits `Call` edges to the module's
method(s) named in `on<event>={…}`, `{getter}`, and `{method(…)}`
expressions. **Phase C of the truthful-scans roadmap (TR-C.1, see
`docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`
§6).** Phase C also covers Aura `.cmp`, Visualforce outside
`extensions=`, Flow XML, and Platform-Event handler wiring — all
of which share this root cause. Until Phase C ships, the interim
mitigation is the classifier routing these nodes to the honest
`declarative_wiring_unparsed` bucket (shipped in Wave 2) rather
than the misleading `visibility_private_unused` bucket; verdicts
recorded in `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`.

## R26. Apex classifier rules live in imperative Rust; declarative rule engine is the long-term home

**Observation.** `graphengine-analysis/src/health/dead_code_classifier/apex.rs`
is a 7-arm `if / else` chain with hand-coded fallthrough order.
The arms encode Apex platform knowledge (interface names,
annotation-to-kind mappings, TDTM handler heuristics, etc.) in
Rust. Each new Salesforce feature (platform event handlers,
Data Cloud, CRM Analytics, etc.) requires another arm and a
re-ordering decision.

**Risk.** (a) Contributors without Apex domain knowledge cannot
safely add rules. (b) The fallthrough order silently encodes
priority — one wrong order and a whole class of methods hits the
wrong verdict. (c) Cross-language rule reuse is impossible —
every ecosystem re-implements the same dispatch chain.

**Recommendation.** Once Wave 2 ships the framework-keyed
dispatch and Phase B ships the Apex Framework Resolver, migrate
the `if` chain to a declarative rule engine: each rule a data
structure (condition + reason + evidence template), engine
selects the highest-priority match. Declarative rules let
non-Rust contributors add Salesforce-specific rules without
touching `classify()`. **Phase D of the truthful-scans roadmap
(TR-D.3, see `docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`
§7).** Gated behind Phase C so the full set of rules needing
migration is known.

**Concrete worked example — R31 (rev 6 → rev 6.1).** The TDTM
handler predicate in `frameworks/tdtm.rs` was an imperative Rust
function whose unit-test fixture set enumerated four
parameter-type shapes (the R24 regression population) but did
not cover the inner-class × class-token-prefix cross-product. It
shipped on rev 6 with all Layer 1 – 3 gates green, then
regressed 48 NPSP nodes on the Layer 4 canary and failed Round 4
hand-audit 4 / 10 wrong. The fix (TR-0.1.1) was a two-line
predicate correction. In a declarative rule-engine world that
same rule would carry its sampled positive and negative
populations as data rows next to the predicate; a Layer-2
invariant check refusing to promote a rule whose sampled
negative set is empty on the NPSP canary would have caught R31
before merge. R31 is the structural-lesson case for R26; see
the Meta-observation paragraph on R31 for the full narrative,
and the acceptance-gate note on TR-D.3 in the roadmap for the
concrete Layer-2 invariant this rule engine must ship with.

## R27. `DeadCodeReason` enum bakes classifier taxonomy into schema

**Observation.** `graphengine-analysis/src/health/report.rs::DeadCodeReason`
is a closed `enum` persisted into `HealthReport.metrics.dead_code.reason_breakdown`.
Adding a new reason is a schema-breaking change for every
downstream consumer (Desktop UI, benchmarking CLI, saved reports
archive). Wave 3's Apex Framework Resolver will want at least
two new reasons (`vf_wiring_unparsed`, `lwc_template_binding_unparsed`);
the current schema can't accept them without a caveat bump.

**Risk.** Reason-breakdown evolution is permanently gated on a
coordinated cross-repo release. Single-module fixes that could
safely introduce a new reason get stalled.

**Recommendation.** Either (a) open `DeadCodeReason` so new
reasons can be added additively behind a `report_schema_version`
bump — downstream tooling treats unrecognised reasons as
`Unclassified` — or (b) remove the enum in favour of a free-form
`reason_id: String` + a registry. Option (a) is the lower-risk
path. **Phase D of the truthful-scans roadmap (TR-D.2 + TR-D.4,
see `docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`
§7).** Gating dependency: Desktop UI must honour the
`report_schema_version` stamp before Phase D ships.

## R28. Framework-undetected: Aura and Jest setup files

**Observation.** Wave 2 Round 3 hand-audit (rev 5; see
`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`) showed `visibility_private_unused`
collapsed from 140 production items → 3 once LWC files began
routing to `declarative_wiring_unparsed`. The three survivors are:

- 1 × Aura controller method (`aura/GE_GiftEntryForm/GE_GiftEntryFormController.js`)
  — template-bound like LWC, but the path detector in
  `graphengine-parsing/src/domain/frameworks.rs` only recognises
  the `lwc/` segment.
- 2 × top-level `jest.setup.js` functions invoked by the Jest
  runner during test-suite bootstrap. Neither the framework
  detector nor the `classify_path` test-file heuristic tags the
  setup file.

**Risk.** Any Salesforce repo mixing Aura with LWC (the norm for
multi-year orgs) will see Aura controller methods misclassified
as `visibility_private_unused`. The same shape applies to any
JavaScript monorepo using `jest.setup.js`, `jest.config.js`,
`vitest.setup.ts`, etc., as configuration entry points. The
absolute numbers in NPSP are small (3) but the pattern scales
with the size of the Aura footprint and the test-bootstrapping
surface.

**Recommendation.** Extend `detect_frameworks_by_path` to
recognise:

1. `aura/<component>/<Name>Controller.js` and
   `aura/<component>/<Name>Helper.js` → framework `aura`,
   with evidence `declarative_wiring_unparsed` (same upstream
   gap class as R25 — the `.cmp` file is not parsed so
   `on<event>` bindings do not produce edges).
2. Root-level `jest.setup.{js,ts}`, `jest.config.{js,ts}`,
   `vitest.setup.{js,ts}`, `vitest.config.{js,ts}` → framework
   `jest` (or `vitest`) with evidence `test_harness_entry_point`
   (functions invoked by the runner, not by user code — treat
   as entry points).

The fix is a data edit to the detector table. **Phase 0 of the
truthful-scans roadmap (TR-0.2, see
`docs/workstreams/proof-foundation-gap/TRUTHFUL_SCANS_ROADMAP.md`
§3).** Part-1 (Aura controller routing) is a near-duplicate of
R25's declarative-wiring fix and is listed as a feeder in Phase C
(`TR-C.2`); Phase 0 does the path-detector portion so the
bucket-level miscount stops now, Phase C does the actual edge
emission so methods become live.

**Status.** Closed by TR-0.2 (engine rev 6). Sizing verdict per
`TRUTHFUL_SCANS_ROADMAP.md` §13: Aura tagged **broadly** via
`aura/` path-segment match (mirrors LWC — folder convention IS
the contract, and the broad rule survives non-canonical bundle
helpers like `FormUtils.js` that a narrow `*Controller.js` rule
would miss); Jest and Vitest tagged **narrowly** as two distinct
tags sharing `frameworks/test_harness_common.rs` (runner identity
IS the contract, and keeping tags distinct preserves
`reason_breakdown` attribution honesty). Implementation:
`graphengine-parsing/src/domain/frameworks.rs::{is_aura_path,
is_jest_harness_file, is_vitest_harness_file}` and
`graphengine-analysis/src/health/dead_code_classifier/frameworks/
{aura,jest,vitest,test_harness_common}.rs`. Verified by the
extended `tests/polyglot_mixed_integration.rs` (includes a
non-canonical Aura helper to lock down the broad-match
contract). The markup-edge emission half (TR-C.2) stays open
under Phase C.

## R29. `GraphNode::extract_name` leaks parameter signature into `node.name`

**Status.** Open. Surfaced during TR-0.1 test alignment
(`graphengine-analysis/src/health/dead_code_classifier/mod.rs`).

**Observation.** `GraphNode::extract_name` in
`graphengine-analysis/src/health/graph.rs` derives the short name
as `fqn.rsplit("::").next()`. For real Apex method FQNs the shape
is `<path>::<Outer>.<Inner>::<method>(<params>)`, so the returned
value is `method(<params>)` — the parameter tuple is not stripped.
Other languages emit short-form FQNs without a `::<sig>()` tail,
so this leak is Apex-specific in practice but the helper is
language-agnostic and silently does the wrong thing for any
language whose FQN carries a parameter signature.

**Risk.** Any downstream consumer comparing `node.name` against a
bare identifier silently misses. Concrete example today:
`looks_like_tdtm_handler`'s `name_lc == "run"` branch is dead on
real data (caught because the sibling `method_seg == "run"` branch,
computed from the header before `(`, preserves coverage). Any
*new* rule using `node.name == "<identifier>"` will be silently
broken on Apex until R29 lands. Report rendering and the Desktop
UI also display the parenthesised form where a bare identifier is
semantically expected.

**Pre-fix hygiene (already in TR-0.1).** The test helper
`derive_simple_name` in
`dead_code_classifier/mod.rs` was intentionally aligned to the
same (leaky) behavior as `extract_name`, so test fixtures now
faithfully reflect production shape. This prevents future work
from being written against a fiction.

**Discovery task (pre-R29-fix).** Before changing `extract_name`,
enumerate every call site of `GraphNode::name` (and any downstream
field that copies it) across the analysis and reporting crates.
Record, for each site, whether the semantics expect a bare
identifier or tolerate a parenthesised form. Expected outcomes:

1. Classifier rule sets — expect bare identifier.
2. Report renderers / Desktop UI — the parenthesised form is
   arguably *desirable* for Apex (readability); changing may
   regress UX. Needs a product decision.
3. Fan-in / fan-out metrics — name-agnostic; no impact.
4. Entry-point detection — TBD.

Only once the audit is complete should `extract_name` be changed.
The one-line fix (`header.rsplit("::").next()` after
`split_once('(')`) is trivial; the blast-radius audit is the real
work. Ticket target: WS-TRUTH Phase 0 follow-up, post-TR-0.2 once
the rev-6 re-parse has a stable baseline to A/B against.

**Recommendation.** Single ticket with two sub-deliverables:
(a) call-site audit artifact (markdown table in this
workstream), (b) `extract_name` fix + one integration test that
asserts `node.name == "run"` on a real Apex parse. Do not bundle
with any other change — the callsite audit is the gate.

## R30. `parent_path` helper duplicated across framework rule sets

**Status.** Open, non-truth-impacting refactor. Surfaced during
TR-0.2 design review when the framework-tag sizing rule
(`TRUTHFUL_SCANS_ROADMAP.md` §13) codified the "distinct tags may
share logic via a helper module" principle. TR-0.2 shipped that
shape for Jest + Vitest via
`frameworks/test_harness_common.rs`, but did *not* retrofit the
pre-existing duplicates in `django.rs` and `celery.rs`.

**Observation.** Three framework rule modules each carry a
verbatim local copy of the same three-line helper that reads a
node's repo-relative path from its classification:

- `graphengine-analysis/src/health/dead_code_classifier/frameworks/django.rs:102`
- `graphengine-analysis/src/health/dead_code_classifier/frameworks/celery.rs:42`
- `graphengine-analysis/src/health/dead_code_classifier/frameworks/test_harness_common.rs:30` (the canonical home added in TR-0.2)

All three bodies are identical:

```rust
let p = ctx.graph.classification_of(ctx.node_id)?;
p.path_repo_rel.clone().or_else(|| p.file_path.clone())
```

The shared version is already `pub(super)`, so sibling modules
can call it without a further visibility change.

**Risk.** Low, and specifically *not* truth-impacting: the three
copies behave identically today. The risk is prospective — a
future change (e.g. honouring a workspace-root override, adding
a canonical-path normaliser, handling symlink targets) applied
to one copy and not the others would introduce a silent
attribution skew between Django, Celery, and the JS test
harnesses. The fix is sized in minutes but the cost of *not*
fixing it is a trap waiting for the next engineer who touches
any of the three.

**Recommendation.** Single refactor PR: delete the local
`parent_path` in `django.rs` and `celery.rs`, replace with
`super::test_harness_common::parent_path(ctx)`. No test changes
(behaviour is identical). Do not bundle with any other change;
this is the kind of clerical cleanup that wants its own review.

**Phase and gating.** Non-blocking. Ship whenever convenient;
natural pair with any Phase-D `EdgeSource` / declarative-
rule-engine work (R26) that will touch every framework module
anyway, since the consolidation lowers the blast radius of that
later refactor. Listed in `docs/02-strategy/SPRINT_PLAN.md`
`WS-TRUTH` section as a `WS-TRUTH-HYGIENE` drive-by.

## R31. TR-0.1 over-matches inner classes and non-`run` methods inside `*_TDTM.cls` files

**Status.** Closed by TR-0.1.1 on rev 6.1 (2026-04-18). Layer-5
Round 4 re-scoring measured `dynamic_dispatch_target` = 0 / 10
wrong against the rev-6.1 baseline (seed `20260418`, identical
to the rev-6 draw). All four specific NPSP FQNs the original
R31 flagged (`CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load`,
`CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader`,
`CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts`,
`RD_CascadeDeleteLookups_TDTM.FirstCascadeUndeleteLoader::FirstCascadeUndeleteLoader`)
are now correctly routed to `no_callers` where Phase A's R23
resolver work picks them up. See
`REGRESSION_RESULTS.md` §"Engine revision 6.1" and
`HAND_AUDIT_LOG.md` §"Round 4 re-scoring — Engine revision 6.1".

The entry below preserves the original analysis for audit
continuity.

### Original observation (as recorded on rev 6)

**Observation.** TR-0.1 replaced the pre-fix substring scan
with a structural check: strip the parameter tuple on the first
`(`, isolate the class segment via `rsplit("::")`, run
`is_tdtm_token` against every dot-split class token. The outer
fix (R24 — parameter types can no longer contribute) is
correct. The simultaneous use of `class_tokens.iter().any(...)`
is too permissive:

- **Inner classes inside a `*_TDTM.cls` file.** Apex FQN
  decomposition presents inner classes as `Outer.Inner`.
  `class_tokens` therefore carries the outer `*_tdtm` token
  plus the inner class name; `.any(is_tdtm_token)` returns
  `true` because the outer token matches, regardless of which
  class's method is being classified. Rev 6 rev-baseline shows
  ≈ 40 inner-class methods / constructors newly labelled
  `dynamic_dispatch_target` with evidence "called via
  `Type.forName().newInstance()`" — factually wrong; NPSP's
  TDTM router reflectively invokes only the *outer* class's
  zero-arg constructor plus `run()`.
- **Non-`run()` methods on the outer TDTM class.** Methods
  like `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()`
  (an override of a parent-class hook) are also now labelled
  `dynamic_dispatch_target`. The method is called by normal
  Apex override polymorphism from
  `CDL_CascadeDeleteLookups`, not by
  `Type.forName().newInstance()`. The classifier cannot tell
  the two apart because the decomposition step does not filter
  by method.

Concrete NPSP FQNs in the regressed set
(`experiments/results/NPSP/rev6/baseline.json`, cross-referenced
against source):

- `CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)`
  — inner class, instantiated via
  `new CascadeDeleteLoader()` at
  `CAM_CascadeDeleteLookups_TDTM.cls:43`.
- `CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()` —
  `protected override` on outer class; dispatched by
  `CDL_CascadeDeleteLookups.cls` via method polymorphism.
- `CON_ContactMerge_TDTM.ContactMergeProcessor::getAccounts(List)`
  — inner-class helper; called from sibling methods inside the
  outer class.
- (≈ 42 additional nodes of the same two shapes — see the rev-5
  → rev-6 delta breakdown in `REGRESSION_RESULTS.md` §rev 6 /
  "TR-0.1 regression — structural root cause".)

**Risk.** (1) The rev-6 `dynamic_dispatch_target` bucket is
inflated by 48 nodes whose evidence string is factually wrong.
Any customer who trusts the reason histogram to size TDTM's
contribution to their dead-code report is reading a lie. (2) The
same 48 nodes are *no longer* in `no_callers`, which robs Phase
A (R23) of the fixture population that should have flagged them
as constructor / override-dispatch resolver gaps. Phase A's
backlog needs them back. (3) The gate rule blocks Phase A from
opening on a rev where Phase 0 introduced a regression —
correctly, but it means the TR-0.1 ship lost us elapsed time we
thought we had banked.

**Recommendation (TR-0.1.1, Phase-0 revisit).** Restrict the
TDTM-convention match to one of the two shapes NPSP actually
reflection-dispatches:

1. The **method** equals `run` — the only reflectively-invoked
   method name on the TDTM_Runnable contract. Applies whether
   the class token is inner or outer.
2. Or: the **outermost class token** (`class_tokens[0]`)
   matches `is_tdtm_token` AND the method is the class's
   zero-arg constructor (method name equals the outermost class
   token after `rsplit('::')`). This catches the
   `Type.forName().newInstance()` call on the constructor.

Concretely in
`graphengine-analysis/src/health/dead_code_classifier/frameworks/tdtm.rs`:

```rust
let outer_class_token = class_tokens.first().copied().unwrap_or("");
let outer_is_tdtm = is_tdtm_token(outer_class_token);
let method_is_run = method_seg == "run" || name_lc == "run";
let is_constructor = method_seg == outer_class_token;
(outer_is_tdtm && (method_is_run || is_constructor))
    || run_on_handler  // keep existing trigger/handler fallback
```

This preserves the R24 fix (parameter types still stripped
first, class-only matching retained) and restores the rev-5
correctness on inner-class / non-`run` nodes (they flow back to
`no_callers`, where Phase A's resolver work can pick them up).

**Test fixtures needed for TR-0.1.1:**

- Positive: `::TDTM_Opportunity::run()` → matches.
- Positive: `::TDTM_Opportunity::TDTM_Opportunity()` → matches
  (zero-arg ctor on an outer TDTM class, reflectively
  instantiated).
- Negative: `::CAM_CascadeDeleteLookups_TDTM.CascadeDeleteLoader::load(Set)`
  → does NOT match (inner class, method != run).
- Negative: `::CAM_CascadeDeleteLookups_TDTM::getCascadeDeleteLoader()`
  → does NOT match (outer class but method != run, method !=
  constructor).
- Negative (R24): `::AccountAdapter::onAfterUpdate(TDTM_Runnable.DmlWrapper)`
  → does NOT match (preserved from TR-0.1).

**Phase A gate is held until Round 4 re-run on a rev 6.1
baseline shows `dynamic_dispatch_target < 2 / 10 wrong`.** The
fix is small (≈ 10 lines + 5 tests). Do not bundle with any
other change — this is a direct correction to a shipped
heuristic and the diff must be auditable in isolation.

**Forward linkage.** When TR-0.1.1 lands, the 48 nodes currently
mislabelled `dynamic_dispatch_target` flow back to `no_callers`.
That revert-population is the exact shape Phase A (R23) must
resolve: inner-class constructors (≈ 40 of 48) and override /
typed-field dispatch on outer classes (≈ 8 of 48). The
`experiments/results/NPSP/rev6.1/` delta between rev 6 and rev 6.1
becomes the authoritative enumeration; it is added to Phase A's
regression fixture set alongside the 17 rev-4/rev-5
`no_callers` audit misses already promoted to
`FRAMEWORK_RESOLVER_PLAN.md` §8.3. Do not ship TR-0.1.1 without
snapshotting that delta into `FRAMEWORK_RESOLVER_PLAN.md` — the
Phase A audit loses that population otherwise.

**Meta-observation (Phase D rationale).** R31 is a case where
a shipped heuristic passed every Layer 1 – 3 gate (unit tests
on the four param-type shapes, polyglot integration, clippy
clean) and still regressed on the Layer 4 / Layer 5 population
canary because the fixture set did not cover the decomposition's
cross-product shape (inner class × class-token prefix).
`EdgeSource` + declarative rule rows (Phase D / TR-D.3) are
specifically the mitigation: a rule row lists the sampled
positive and negative examples next to its predicate, and the
layer-2 invariant gate can assert rule-row coverage against the
NPSP canary before a rule is allowed to flip live. Record this
as supporting evidence for TR-D.3 priority, not as a separate
ticket.

## R32. Parse DB schema migrations are invisible — downstream consumers cannot detect pre-upgrade artefacts

**Status.** Engine-side closed in WS-TRUTH-A PR #1 (TR-A.0). Desktop
caveat-surface UI is still open under WS-DIAG. This risk will flip
to "Closed" once the Desktop surface ships.

**Engine closure evidence (PR #1).**

- `parse_meta` table added with `schema_version INTEGER NOT NULL`
  stamped at `2` on every new DB
  (`graphengine-parsing/src/infrastructure/storage/schema.rs`,
  `sqlite_repository.rs::migrate_schema`).
- `graphengine-analysis::health::mod::is_parse_db_stale` reads the
  stamp before analysis; missing row or stamp `< 2` causes
  `CAVEAT_STALE_PARSE_DB_V1` emission via
  `build_integrity_status(.., stale_parse_db=true)`
  (`graphengine-analysis/src/health/report.rs`,
  `graphengine-analysis/src/health/mod.rs`).
- Three end-to-end cases in
  `graphengine-analysis/tests/integration_test.rs`:
  `legacy_db_without_parse_meta_emits_stale_caveat`,
  `older_schema_version_emits_stale_caveat`,
  `current_schema_version_does_not_emit_stale_caveat` — all green.
- The byte-identical CI gate
  (`experiments/bin/assert_rev6_1_byte_identical.sh`) explicitly
  asserts the caveat fires on a rev-6.1 DB, providing a second
  independent check.

**Still open.** Desktop caveat-surface UI must render
`CAVEAT_STALE_PARSE_DB_V1` as "Re-parse your repository" (not as a
trend-eligible number). Tracked under WS-DIAG.

**Observation.** The SQLite parse DB (`parse.db` /
`parse.ab.sqlite`) carries no schema-version stamp. Every
existing table is read as-if-current by `graphengine-analysis`
and by any consumer of the parse DB. Phase A (TR-A.0) adds a new
`apex_class_symbols` table; existing parse DBs will silently lack
it. `ge-analyze` reading a pre-TR-A.0 DB sees "no Apex class
symbols exist for any class" rather than "this DB predates the
oracle."

**Risk.** Silent data loss — an analyse run on a stale DB
produces a correct-looking HealthReport that is missing every
Phase A resolver improvement. No signal, no caveat, no error.
Users upgrading `graphengine-analysis` without re-parsing would
see regressed-looking numbers and lose confidence; a support
engineer debugging the regression cannot distinguish "stale DB"
from "genuine regression". The same class of failure will recur
on every future schema addition (Phase B framework-edge tables,
Phase C declarative-wiring tables, Phase D `EdgeSource` / per-
edge confidence columns) if we don't solve it once now.

**Recommendation (closed by TR-A.0).** Add a `parse_meta` table
with `schema_version INTEGER NOT NULL` as part of TR-A.0. Ship
at `schema_version = 2` (implicit v1 is the current state).
`ge-analyze` reads `schema_version` from `parse_meta` before any
other table; if below the expected minimum, it emits a top-level
`CAVEAT_STALE_PARSE_DB_V1` stamp on the HealthReport and a
non-fatal log line on stderr. Desktop UI caveat dispatcher
surfaces the caveat visibly and prompts re-parse. Phase D
(TR-D.4) takes ownership of the broader versioning contract
(parse-DB schema version + report schema version under a single
caveat-dispatch infrastructure); TR-A.0 ships its first concrete
use.

**Phase assignment.** TR-A.0 ships the table + stamp + caveat
emission. Desktop caveat-surface UI work is tracked in WS-DIAG.
Phase D (TR-D.4) owns the cross-cutting versioning contract.

## R33. Shared extractor drops every `new X(...)` constructor call site across Apex / Java / C# / JavaScript / TypeScript

**Status.** Open. Surfaced during Phase-A planning when tracing why
Apex constructor resolution (TR-A.1 / TR-A.2) had zero call sites to
resolve. To be closed by TR-A.1 ship (see
`docs/workstreams/proof-foundation-gap/PHASE_A_EXECUTION_PLAN.md`
§8).

**Observation.**
`graphengine-parsing/configs/apex.yaml` L105–109 captures
`@constructor` on the `type` child of `object_creation_expression`:

```
(object_creation_expression
  type: (_) @constructor
  arguments: (argument_list) @args
) @call
```

The same pattern (`@constructor` on the type node, no `@type`) is used
by `java.yaml`, `csharp.yaml`, `javascript.yaml`, and
`typescript.yaml`.

`graphengine-parsing/src/syntax/extractors/call_site_extractor.rs`
L70–L110 recognises these capture names: `call`, `func`,
`method_call`, `receiver`, `method`, `constructor_call`, `type`,
`scope`, `chained_call`, `chained_method`. **`constructor` is not in
the recognised list.** Only `type` (L83–L88) synthesises a
`TypeName::new` function name.

Result: in every Apex, Java, C#, JavaScript, and TypeScript repo the
engine has ever scanned, every `new X(...)` expression is silently
dropped at extractor time with `function_name = None`. The resolver
never sees a single constructor call site. `tests/call_resolution_integration.rs::resolves_constructor_calls_with_type_fallback`
is a synthetic test that manually feeds `"constructor_call:Type::new()"`
into `SyntaxResults`; it never exercises the extractor path, so the
gap has no coverage.

**Adjacent gap (Apex-specific).** `apex.yaml` L112–L114:

```
(explicit_constructor_invocation
  arguments: (argument_list) @args
) @call
```

has *no* function-name capture at all. `this(...)` and `super(...)`
chained-ctor sites are dropped at the same boundary.

**Risk.** Every Phase-A ticket that consumes "constructor call site
exists" is gated on this fix. More broadly: four non-Apex language
canaries (Java, C#, JavaScript, TypeScript — to the extent customers
run them) have been producing under-counted call graphs for the
lifetime of the extractor. Any `no_callers` metric on those languages
is inflated by the missing-constructor-edges population. The
metric-trust story the product sells has a five-language blast
radius on this one bug.

**Recommendation (closed by TR-A.1).** Single-source-of-truth fix in
`call_site_extractor.rs`: add a `"constructor"` arm mirroring the
existing `"type"` arm that synthesises `TypeName::new`. Five YAML
files stay unchanged. Add the Apex-specific
`explicit_constructor_invocation` capture in `apex.yaml` (keyword
alternation on `this` / `super`) plus a matching extractor arm
synthesising `EnclosingClass::new` / `ParentClass::new`. Integration
test per affected language end-to-end (Apex, Java, C#, JavaScript,
TypeScript) — each parses a real `new Foo()` source string and
asserts `Foo::new` lands in `SyntaxResults.call_sites`.

**PR description requirement (WS-TRUTH-A PR 2).** Call out R33 closure
explicitly. Expect small (double-digit) constructor-edge increases on
Java / C# / JS / TS canaries; re-run relevant non-Apex canaries
before merge; pre-register the delta if any exists.

**Pre-merge action completed on PR 2 (2026-04-18).** Cross-language A/B
measurement executed. Two new canaries vended (`apache/commons-lang` @
`rel/commons-lang-3.14.0` for Java, `serilog/serilog` @ `v4.0.0` for C#);
added to `experiments/run_canaries.sh`. Consecutive release builds at PR 2
merge-base, PR-2-OFF (stashed) → PR-2-ON (popped), captured heuristic call
edge counts via `Call provenance coverage: Heuristic=N`.

| Canary | Language(s) | Δ heuristic call edges | Δ total edges | Δ `no_callers` |
|---|---|---|---|---|
| commons-lang | Java | +1,072 | +1,072 | **−10** |
| serilog | C# | +505 | +505 | 0 |
| nextjs-commerce | TS + JS | +11 | +6 | 0 |

Java delta is the strongest signal: +1,072 new resolved constructor call
edges AND a corresponding −10 `no_callers` drop, confirming the extractor
arm fires end-to-end. All three canaries non-negative, non-zero — §8.5
regression-signal condition is clear. Apex coverage proved by PR 2's in-
tree integration fixtures (`extractor_constructor_fixtures.rs` +
`apex_resolver_r23_ctor_fixtures.rs`). Full methodology + interpretation
in [`R33_CROSS_LANGUAGE_EVIDENCE.md`](R33_CROSS_LANGUAGE_EVIDENCE.md).

**Merge posture.** §8.5 evidence requirement satisfied. PR 2 merge is
unblocked on the R33 evidence axis. R33 flips to Closed once PR 2 merges.

**Phase assignment.** TR-A.1 (WS-TRUTH-A PR 2) ships the fix. No
separate ticket; the five-language blast radius is closed by the
Apex-motivated repair because the extractor is shared.

## R34. `ResolverTier::Merged` is defined but never produced; no telemetry field consumes it

**Status.** Open — **deferred wholesale to WS-PROOF-R3** per
`PHASE_A_EXECUTION_PLAN.md` §9.1. The bundling gate was evaluated at
PR 2 implementation time and failed on condition 1 (≤ 30 LOC combined):

- Because `ResolvedEdges.stats` lives in `application::ports`, not in
  `domain/provenance.rs` as the plan hypothesized, a layer-correct
  placement of `ResolverTier` required moving the enum from
  `syntax::language::apex::resolver_dispatch` into
  `application::ports` (application cannot depend on syntax). That
  relocation alone consumes the gate's budget before any new logic.
- A faithful, well-documented implementation (enum relocation, field,
  per-return stamping helper, gap-fill tracking in
  `merge_with_gap_fill`, one unit test) measured ~48 LOC of non-test
  source additions across the two modules — strictly over the 30-LOC
  ceiling.
- Shrinking under 30 LOC required dropping doc comments, removing the
  `stamp_tier` helper, and inlining the merge stats, which would
  violate project coding standards.

Per §9.1 the partial-fix pathway is explicitly forbidden, so the
entire R34 scope — `actual_tier` on `ResolvedEdges.stats`,
`merge_with_gap_fill` stamping `Merged`, HealthReport surfacing,
Desktop UI wiring — now ships together under WS-PROOF-R3.

**Observation.** `ResolverTier::Merged` exists in
`graphengine-parsing/src/syntax/language/apex/resolver_dispatch.rs`
(L45–L61) with wire string `"merged"` and is referenced by tests.
`selected_tier()` (L148–L160) only ever returns `LspPrimary` or
`Heuristic`. `rg resolver_tier` across `graphengine-analysis` and the
API crate returns zero hits — **no HealthReport / analysis-crate /
API-crate field currently emits the resolver tier.**
`selected_tier()` is a pre-resolution peek used only for startup
logging.

**Risk.** WS-PROOF-R3 (`resolution_quality` as a first-class UI
indicator) presupposes a post-resolution "actual tier that ran"
signal. Pre-resolution `selected_tier()` cannot serve that role
because gap-fill participation is determined during `resolve()`,
not at peek time. If WS-PROOF-R3 were implemented against the
current shape, it would display tier = `LspPrimary` on scans where
heuristic gap-fill contributed a meaningful fraction of edges.

**Fix shape.**

1. **(a)** Add `actual_tier: ResolverTier` on `ResolvedEdges.stats`,
   populated from the `resolve()` happy path — `Lsp` when only LSP
   ran, `Heuristic` when only heuristic ran, `Merged` when
   `merge_with_gap_fill` appended any edges from the fallback tier.
   Narrow mechanical change inside `resolver_dispatch.rs`.
2. **(b)** Propagate into `HealthReport` under a new `resolver_tier`
   field, or under the EdgeSource-aware aggregation that TR-D.1
   introduces if Phase D is close. Requires touching
   `graphengine-analysis` and the API crate.

**Phase assignment.** Scope (b) — HealthReport surfacing, Desktop UI
wiring — is owned by WS-PROOF-R3. Scope (a) — narrow
`ResolvedEdges.stats` update — is bundled opportunistically into
WS-TRUTH-A PR 2 (TR-A.1), only if the diff size is trivial (the PR
already touches `resolver_dispatch.rs`-adjacent wiring for R33). If
(a) is bundled, a new unit test asserts `merge_with_gap_fill` stamps
`Merged` on `ResolvedEdges.stats` when both tiers contributed.
Otherwise (a) moves wholesale to WS-PROOF-R3.

## R38. PHASE_A_EXECUTION_PLAN §4.3 NPSP acceptance for TR-A.3 was over-scoped vs. actual NPSP call shapes — only 1 of 3 target FQNs is resolvable within TR-A.3's receiver grammar

**Status.** Open — cross-PR acceptance re-baseline. Not a
correctness regression; a planning-vs-reality correction.

**Observation.** Plan §4.3 (TR-A.3 acceptance) declares three
NPSP FQNs as the "resolve live" bar:

1. `UTIL_Permissions::canUpdate(SObjectType)`
2. `SfdoInstrumentationService::log(...)`
3. `Contacts::loadAccountByIdMap()`

Dry NPSP run post-R37 fix + R36 fix + PR 3 TR-A.3 landing
(heuristic resolver, `GRAPHENGINE_APEX_RESOLVER=heuristic`,
NPSP corpus at the rev-7 working SHA):

- `UTIL_Permissions::canUpdate` — **55 incoming `Call` edges.**
  Resolves live. This is the typed-field-direct-call shape
  (`private UTIL_Permissions permissionsService; … permissionsService.canUpdate(X)`)
  that TR-A.3 targets explicitly.
- `SfdoInstrumentationService::log(...)` — **0 incoming `Call`
  edges.** Every NPSP call site uses the chained shape
  `SfdoInstrumentationService.getInstance().log(...)` — the
  receiver is a **method invocation that returns an instance**,
  not a typed field or local. TR-A.3's receiver grammar
  (`NormalisedReceiver = SelfRef | BareIdent | DottedDefer`)
  declines on dotted receivers that are not `this.<field>`, and
  `X.getInstance().log(...)` normalises to `DottedDefer`.
  Resolving this shape requires **return-type propagation** —
  infer that `SfdoInstrumentationService.getInstance()` returns
  `SfdoInstrumentationService`, then dispatch `log(...)` on that
  type. Return-type propagation is not in Phase A's TR-A.x
  scope; it is framework-resolver / Phase B material.
- `Contacts::loadAccountByIdMap()` — **0 incoming `Call`
  edges.** The NPSP call site is bare (`loadAccountByIdMap();`
  inside the `accountById` getter body of `Contacts.cls`). Bare
  self-calls have `receiver_text = None` in `CallSite`, and
  `resolve_field_type_call` returns early on the `None` case
  (by design — the TR-A.3 scope is receiver-prefixed calls).
  This shape is handled by **TR-A.4 (PR 4)**'s
  `this.method(...)` self-dispatch arm, which also covers the
  Apex-implicit-`this` bare form. PR 4 is the next scheduled
  PR and will close this FQN.

**Why the plan over-scoped.** The plan author paired each
TR-A.x acceptance FQN to an NPSP call site by grepping for the
target method name and picking one representative call. For
`UTIL_Permissions::canUpdate` the picked call was indeed a
typed-field direct call (the TR-A.3 shape). For
`SfdoInstrumentationService::log` and
`Contacts::loadAccountByIdMap` the picked calls were chained
and bare-self respectively — call shapes that TR-A.3's
receiver grammar explicitly defers (per the inline docs on
`NormalisedReceiver`). The plan acceptance bar conflated
"method exists and will be called in NPSP" with "TR-A.3's
receiver grammar can bind the call"; in reality the latter is
strictly narrower than the former for these two FQNs.

**Fixture coverage is intact.** The TR-A.3 fixtures at
`graphengine-parsing/tests/fixtures/apex_resolver/r23_a3_*`
exercise the **shapes TR-A.3 owns**: typed-field direct call,
DI-constructor-injected field, typed-local-var direct call.
All three fixtures pass the single-Medium-edge assertion in
`apex_resolver_r23_a3_field_dispatch_fixtures.rs`. TR-A.3 as a
resolver module is complete; the plan-vs-reality gap is on the
cross-PR acceptance row only.

**Corrected acceptance ownership.**

| Plan §4.3 FQN | Resolves via | PR that closes it |
|---|---|---|
| `UTIL_Permissions::canUpdate(SObjectType)` | Typed-field direct call | **PR 3 (TR-A.3)** — live, 55 edges. |
| `Contacts::loadAccountByIdMap()` | Bare self-call (implicit `this`) | **PR 4 (TR-A.4)** — planned. |
| `SfdoInstrumentationService::log(...)` | Chained-call return-type propagation | **Phase B** (framework resolver / return-type inference). Tracked here until the Phase B entry point exists. |

The PR 7 ship write-up MUST label the TR-A.3 live-acceptance
signal as "1/3 NPSP FQNs resolved by TR-A.3 as-written; 1/3
deferred to TR-A.4 (bare self-call); 1/3 deferred to Phase B
(chained-call return-type propagation)" rather than "TR-A.3
fell short". The shape-vs-scope match is the honest signal.

**No rollback triggered.** PR 3's fixtures pass, the TR-A.3
resolver module is correct for the shapes in its grammar, and
R37 (the real production bug PR 3 surfaced) closed inside the
same PR. The finding here is a planning correction, not an
engineering regression.

**Follow-through after PR 4.**

- Re-run the NPSP dry on PR 4 merge; `Contacts::loadAccountByIdMap()`
  must show >= 1 incoming Call edge (the bare-self caller in
  the `accountById` getter). **Update 2026-04-18:** PR 4 shipped
  the TR-A.4 bare-self-dispatch arm correctly (fixture
  `r23_a4_bare_self_dispatch` passes, 1 Medium edge). On NPSP
  corpus the FQN still shows **0 incoming Call edges** — but the
  cause is **not** a TR-A.4 resolver gap. The bare call site
  lives inside the `accountById` property getter body
  (`get { if (accountById == null) { loadAccountByIdMap(); } ... }`);
  `find_enclosing_function` does not walk into property accessors
  because the Apex extractor does not emit property accessors as
  Function nodes. Filed as **R39** below; owned by a separate
  extractor-scope PR, not by any remaining TR-A.x resolver work.
  `Contacts::loadAccountByIdMap()` is re-routed from R38 → R39
  for NPSP live acceptance.
- `SfdoInstrumentationService::log(...)` will remain at 0 edges
  until return-type propagation ships. The Phase B framework
  resolver work owns this FQN; mark it on the Phase B entry
  ticket when that lands.
- Update PHASE_A_EXECUTION_PLAN §4.3 acceptance text to name
  PR 3 / PR 4 / R39 / Phase B ownership explicitly in the
  doc-sweep pass (PR 7).

---

## R39. Apex property accessor bodies are invisible to the resolver — every call site inside a `get { ... }` / `set { ... }` body is silently dropped

**Status.** Open — extractor scope, separate PR. Not a TR-A.x
resolver gap; TR-A.4's bare-self-dispatch arm is proven correct
on a matched shape (`r23_a4_bare_self_dispatch` fixture, 1 Medium
edge).

**Observation — how PR 4 surfaced this.** R38 ownership table
routed the NPSP FQN `Contacts::loadAccountByIdMap()` to PR 4's
TR-A.4 bare-self-dispatch arm. PR 4 landed the arm and the
matching fixture passes cleanly. On the NPSP dry (rev-7 post
PR 4) the FQN still shows **0 incoming Call edges**:

```
sqlite3 experiments/results/NPSP/rev7_tra4_dry/parse.db "
SELECT COUNT(*) FROM edges e JOIN nodes n ON n.id=e.to_id
WHERE e.kind='Call' AND n.fqn LIKE '%Contacts::loadAccountByIdMap%';
"
0
```

The call site at `force-app/main/adapter/in/sobjects/contact/Contacts.cls:193`
is:

```apex
public Map<Id, Account> accountById {
    get {
        if (accountById == null) {
            loadAccountByIdMap();        // <-- line 193
        }
        return accountById;
    }
    set;
}
```

— i.e. **inside the `accountById` property accessor body**, not
inside a `method_declaration` or `constructor_declaration`.

**Root cause.** The Apex extractor (`configs/apex.yaml` +
`class_symbols_extractor.rs`) captures `method_declaration` and
`constructor_declaration` as `Function` nodes. It does **not**
capture property accessor bodies (Apex grammar node
`accessor_declaration` inside a `property_declaration`) as
Function nodes or any other callable kind. Verified in parse DB:

```
sqlite3 experiments/results/NPSP/rev7_tra4_dry/parse.db "
SELECT id, kind, fqn FROM nodes
WHERE json_extract(location,'\$.file') LIKE '%/Contacts.cls'
  AND fqn LIKE '%accountById%';
"
# only the single Function node for loadAccountByIdMap itself
# is returned — NO node representing the accountById property
# accessor.
```

The resolver's `find_enclosing_function` in
`heuristic_resolver.rs:660` walks `functions_by_file` (the
Function nodes) and returns the smallest one containing the
call site's range. For calls inside an accessor body the
property is not a Function, the nearest Function ancestor
outside is the containing class (which is a `Struct`, not a
`Function`) — so `find_enclosing_function` returns `None` and
the call site is silently dropped **before** any TR-A.x
dispatch arm runs. This affects **every** call inside **every**
property accessor in the corpus, not just `loadAccountByIdMap`.

**Evidence the resolver pipeline itself is not at fault.**

- The TR-A.4 fixture at
  `graphengine-parsing/tests/fixtures/apex_resolver/r23_a4_bare_self_dispatch/ContactsLike.cls`
  places the bare self-call inside an ordinary method body. It
  resolves live with 1 Medium edge (Exact tier, arity-0 unique) —
  proving the bare-self-dispatch arm itself works.
- NPSP shows 91 `Call` edges from other Contacts.cls functions
  resolve fine — the extractor is healthy for `method_declaration`
  bodies; the gap is strictly about accessor bodies.

**Blast radius beyond `loadAccountByIdMap`.** Unknown without
an audit. Every Apex property accessor body containing method
calls, field references, or cross-class invocations is
currently a black hole for the resolver. NPSP alone has dozens
of properties with non-trivial getters (the `accountById`
pattern is canonical Apex). A full audit belongs in the R39
fix PR.

**Why this was not caught earlier.**

- No fixture in the TR-A.1 / TR-A.2 / TR-A.3 / TR-A.4 corpora
  uses an accessor body as the caller site. Every caller lives
  inside a `method_declaration`. This is an honest gap in
  fixture coverage, not a conscious deferral.
- The plan author (PHASE_A_EXECUTION_PLAN §4.3) identified
  `Contacts::loadAccountByIdMap` as the TR-A.3 / TR-A.4 target
  by grepping for the method name; the bare shape matches
  TR-A.4's grammar on paper. That the call lives inside a
  property-getter body — an orthogonal extractor gap — was not
  visible until a real NPSP dry run after PR 4 landed.
- R38 correctly predicted PR 4 would close the shape at the
  resolver layer; it did. The plan-vs-reality miss is one
  layer down, in the extractor.

**Recommendation.** Ship R39 as its own extractor-scope PR
(R39.1) before claiming NPSP acceptance for
`Contacts::loadAccountByIdMap`:

1. Extend `configs/apex.yaml` with patterns that capture
   `accessor_declaration` nodes. Accessor kind is orthogonal
   to `get` vs `set` from the resolver's point of view; both
   are callable bodies.
2. Extend `class_symbols_extractor.rs` (and the parent
   `method_declaration` extractor) to emit each accessor as
   a Function node whose FQN is
   `<class>::<property>::__get__` / `<class>::<property>::__set__`
   (mirroring the existing `__trigger__` synthetic-node
   convention for triggers). Using an `__accessor__` suffix
   rather than a bare property name avoids FQN collisions
   with methods of the same name.
3. Cross-link the synthetic accessor node to the property
   field node via a `ContainedIn` / `Accessor` edge so the
   property remains the single logical target for analysis
   consumers.
4. Fixture: one-file Apex class with a property whose `get`
   body calls a sibling method. Assert 1 Call edge from the
   synthetic accessor node to the sibling method.
5. NPSP dry re-run after R39.1 lands:
   `Contacts::loadAccountByIdMap` must show >= 1 incoming
   Call edge; audit the corpus for new accessor-body edges
   (expect dozens-to-hundreds — record the delta on the
   R39 ticket).

**No rollback.** PR 4's TR-A.4 module is correct for the
shapes in its grammar; all four fixtures pass. The PR 7 ship
write-up MUST label TR-A.4 live-acceptance honestly:

- `fflib_Comparator::compare(String,String)` — **25 incoming
  Call edges** on NPSP (1 Medium + 24 Low), live.
- `Contacts::loadAccountByIdMap()` — **0 incoming Call edges**,
  deferred to R39 (extractor scope).

The ship message must not claim "PR 4 closed R38" — it closed
the TR-A.4 resolver half; the extractor half is R39.

**Owner — suggested PR slot.** R39.1 should land before Phase
B kickoff; property accessors are Phase A / correctness-gap
scope. Candidate slot: between PR 5 (TR-A.6 inner class) and
PR 5.5 (R35 determinism). Exact placement is caller-scheduled.

**Cross-reference.** R38 "Follow-through after PR 4" updated
2026-04-18 to reflect the extractor-layer rerouting.

**rev 9 Round 5 audit frequency (2026-04-19).** This shape
appeared **2 / 10** times in the Round 5 `no_callers` draw on
the rev 9 baseline, after PR 9 (R46 closure) made the
`RD2_OpportunityMatcher::match(...)` family of property-getter
callees visible to the sampler. R39 is now empirically the
single largest rev-9 false-positive driver in the `no_callers`
bucket. See `HAND_AUDIT_LOG.md` §"Round 5 — Engine revision 9"
samples #1 and #2.

---

## R40. Apex `TypeName.staticMethod()` receiver resolution gap — every static-method call through a typed dotted receiver was silently dropped

**Status.** Closed — PR 8 (TR-A.4 follow-on; resolver scope).

**Observation.** During the rev 7 → rev 8 dry the Apex resolver
produced zero `Call` edges for the `Cls.staticMethod()` shape on
NPSP, even though the receiver expression is a class name in the
class registry. The resolver's `DottedDefer` arm carried only the
text of the dotted prefix; the `BareIdent` fallback in the second
position dropped the receiver type entirely and resolved on
function-name only, then drowned in the fanout cap.

**Root cause.** `apex/heuristic_resolver.rs::resolve` did not
contain a "type-name receiver" arm. For `Cls.m()`, the resolver
captured the textual prefix `"Cls"` in `DottedDefer { receiver_text }`
but never asked the class registry "is `Cls` a known user type, and
if so, does its `methods` table contain `m`?" Static-method calls
through typed receivers therefore matched the same fanout-collapse
path as untyped dotted ambiguity.

**Fix shipped (PR 8).** New `resolve_type_name_receiver` helper in
`apex/heuristic_resolver.rs`. The `DottedDefer` arm now carries the
receiver text into a typed lookup against `class_symbols`; on a hit,
the second-segment method resolves at `Confidence::High` and the
fanout fallback is bypassed for that call site. `BareIdent` fallback
preserves prior behaviour for non-typed receivers. Three new
fixtures under
`graphengine-parsing/tests/fixtures/apex_resolver/r40_type_name_receiver/`
exercise (a) cross-file static call, (b) intra-file static call to a
sibling class, (c) static call through an inner-class FQN.

**rev 8 NPSP impact.** `no_callers` 502 → 492 (−10 — the static-call
edges into previously-invisible callees recovered ten dead
classifications). Recorded in REGRESSION_RESULTS.md §"Engine
revision 8". No regressions on rev 7 fixtures.

**Why it lived undetected through Phase A acceptance.** The Phase A
fixture set covered intra-class and cross-file *constructor*
dispatch (TR-A.1 / TR-A.2) and field-typed instance dispatch
(TR-A.3) but no fixture exercised a static call through a typed
class-name receiver. The rev 7 NPSP dry surfaced it because the
NPSP `RD2_OpportunityEvaluation_BATCH` pipeline routes most of its
internal coordination through `Cls.staticHelper()` calls.

**Cross-reference.** PR 8 also opened R44 (R11 inheritance
direction asymmetry) — see R44 below.

---

## R41. Apex field-initializer body extraction gap — every method call inside a field default-value expression is silently dropped

**Status.** Open — extractor scope, separate PR. Not a Phase A
TR-A.x scope item; surfaces independently via the rev 9 Round 5
audit (1 / 10 frequency on the `no_callers` draw).

**Observation.** Apex permits non-trivial default-value expressions
on field declarations:

```apex
public class ALLO_ManageAllocations_CTRL {
    private Map<Id, List<Allocation__c>> allocCache =
        new Map<Id, List<Allocation__c>>{
            opp.Id => getMappedAllocationsForOpp(opp)
        };
    // ...
}
```

The call to `getMappedAllocationsForOpp(opp)` is the only call
site in the corpus. `getMappedAllocationsForOpp` shows zero
incoming `Call` edges in rev 9 and lands in `no_callers`.

**Root cause.** `class_symbols_extractor.rs` and the call-site
extractor walk `method_declaration` and `constructor_declaration`
bodies. They do not walk the right-hand-side expression tree of
`field_declaration` nodes. Every field-initializer expression —
map literals, list literals, set literals, constructor calls,
ternary expressions, method invocations — is invisible to the call
extractor.

**Verified shape.** Inspected the parse DB for the
`ALLO_ManageAllocations_CTRL.cls` `allocCache` field on rev 9:
the `Function` node for `getMappedAllocationsForOpp` exists; no
caller edge exists. Adding a `private void __init() {
allocCache = new Map<Id,...>{ opp.Id => getMappedAllocationsForOpp(opp) };
}` synthetic method to a private fixture causes the same call to
resolve correctly — confirming the issue is purely extractor
scope, not resolver scope.

**Blast radius.** Unknown without an audit. Every Apex field with
a method call in its initializer is invisible. Map / List / Set
literal initializers with call-valued entries are the most common
shape. NPSP frequency at rev 9: ≥ 1 / 10 in a single draw.
Conservative estimate: dozens of false-positive `no_callers` per
NPSP-scale codebase.

**Recommendation.** Ship R41 fix as its own extractor-scope PR
(R41.1):

1. Extend `configs/apex.yaml` to capture call sites inside
   `field_declaration > variable_declarator > * (initializer)`
   subtrees.
2. Extend `class_symbols_extractor.rs` so the synthesized
   declaration node owns the initializer expression as a callable
   body region; the call extractor walks it the same way it walks
   `method_declaration` bodies.
3. Resolve the FQN of the synthetic body the way `__trigger__`
   bodies are resolved: `<class>::<field>::__init__` (mirroring the
   R39 recommendation for property accessors). Cross-link to the
   field node via `Contains`.
4. Fixture: a one-file Apex class with a map-literal field
   initializer calling a sibling method. Assert 1 `Call` edge.
5. NPSP dry re-run after R41.1 lands: at least the
   `ALLO_ManageAllocations_CTRL::getMappedAllocationsForOpp` site
   (and any sibling callees discovered during the corpus audit)
   resolves with ≥ 1 incoming `Call` edge.

**Phase assignment.** Owned by the universal-fidelity sprint's
T8 add-on (extraction-coverage-aware classifier downgrade) for
the *honest workaround*, and by a future extractor PR (R41.1) for
the *real fix*. T8 in `docs/workstreams/universal-fidelity/`
mitigates the false-positive impact without back-filling
extractor work into Phase A.

**Cross-reference.** Filed during PR 9 (R46 closure) as a
direct outcome of the rev 9 Round 5 hand-audit. See
`HAND_AUDIT_LOG.md` §"Round 5 — Engine revision 9" sample #3.

---

## R44. R11 inheritance direction asymmetry — `extends` / `implements` edges flow only child → parent in the resolver, never parent → child

**Status.** Open — resolver scope, Phase B.

**Observation.** The Apex resolver emits `Edge::Extends` and
`Edge::Implements` from each subclass / implementer to its parent
or interface (the language-canonical direction). For consumers
that need to enumerate *all subclasses* of a parent (e.g.
"every `TDTM_Runnable` implementer that overrides `run()`" — a
key Phase B framework-entry signal) the resolver has no
parent → child index and instead does a full registry scan per
query. Discovered during rev 8 sample inspection while tracing
why `TDTM_iTableDataGateway` implementations were not lighting
up reflectively-dispatched `Call` edges from the TDTM router.

**Root cause.** `apex/class_registry.rs` indexes `extends` /
`implements` by the *subclass* FQN. There is no inverse index
keyed by the *parent* FQN. Every dispatch arm that asks "who
implements this interface?" or "who extends this class?" walks
the entire registry (O(n)) per query, which both hides the
asymmetry from a casual reader and makes the dispatch arm
unwilling to ask the question except in test-only contexts.

**Why this is asymmetric, not just a perf gap.** Phase B's
framework-entry edges for `Schedulable`, `Batchable`,
`Queueable`, `Database.Batchable`, the TDTM `iTableDataGateway`
interface, and the `fflib_ISObjectDomain` family all need
"give me every implementer" lookups. Without an inverse index,
these dispatch arms have to either (a) do an O(n) scan per
synthetic-edge emission, or (b) accept incomplete coverage.
Today they choose (b) silently — emitting framework edges only
for declarations the dispatch arm was specifically taught to
look for.

**Recommendation.**

1. Add `class_registry::implementers_of(parent_fqn)` and
   `subclasses_of(parent_fqn)` indices computed at registry-seed
   time (one extra `HashMap<Fqn, Vec<Fqn>>` per direction; O(n)
   to build, O(1) per query thereafter).
2. Phase B framework-entry dispatch arms (Batchable, Schedulable,
   Queueable, TDTM router, fflib domain factory) consume the
   inverse indices when emitting synthetic edges; eliminate the
   per-query registry scans.
3. Regression fixture: a parent interface with three concrete
   implementers; assert that the inverse-index lookup returns all
   three; assert that a hypothetical `FrameworkEntry` arm seeded
   with the parent emits one synthetic edge per implementer.

**Phase assignment.** Phase B (Apex Framework Resolver) — see
`FRAMEWORK_RESOLVER_PLAN.md` §3 dispatch matrix rows for
Batchable / Schedulable / Queueable / TDTM. Not in scope for
Phase A or for the universal-fidelity sprint.

**Cross-reference.** Discovered during rev 8 sample inspection
(2026-04-19); filed as part of the PR 9 honest-close doc sweep.

---

## R45. Chained call on a call-expression return value — `obj.first().second()` resolves `first()` but drops `second()` because the resolver does not feed `first()`'s return type back into `second()`'s receiver position

**Status.** Open — resolver receiver-typing scope, Phase B (or a
dedicated TR-B.x ticket if Phase B opens narrower).

**Observation.** Two NPSP shapes hit this:

```apex
// Shape A — singleton + accessor
GE_SettingsService settings = GE_SettingsService.getInstance().getDataImportSettings();

// Shape B — constructor + method
String result = new fflib_StringBuilder().add("a").build();
```

Phase A's TR-A.4 (bare-self-dispatch) and TR-A.3 (typed-field
dispatch) cover the *first* receiver in each chain. The *second*
position (the method invoked on the return value of the first
call) has no resolver arm — the heuristic resolver receives
`call_expression > field_access > call_expression(first)` and
gives up on the inner call's return-type question.

Discovered during rev 9 Round 5 audit (sample #4 —
`GE_SettingsService.getDataImportSettings()` shows zero callers
because the only callsite is
`GE_SettingsService.getInstance().getDataImportSettings()`).

**Root cause.** `apex/heuristic_resolver.rs::resolve` resolves the
outer `call_expression` by walking its `method_invocation` path:
the receiver of the outer call is the inner `call_expression`,
not a typed identifier. The resolver has no "infer return type
from inner call's resolved target" arm — return types are not
threaded through receiver position. The resolver does have
`ApexClassSymbols.methods[i].return_type` on every resolved
method (populated by TR-A.0); it just doesn't consume it for
chained dispatch.

**Two sub-variants.**

| Variant | Inner shape | Frequency in NPSP |
| ------- | ----------- | ------------------ |
| **R45.A** | `Cls.static().chained()` | High — singleton-instance idiom across NPSP services |
| **R45.B** | `new Cls().chained()` | Medium — fluent-builder idiom in fflib StringBuilder, fflib_QueryFactory, etc. |

R45.A is more common in the NPSP corpus; R45.B is more common in
fflib, fflib_apex-mocks, and apex-common library usage.

**Recommendation.**

1. Extend the resolver's receiver-typing logic with a
   `resolve_receiver_type_from_inner_call(inner: ResolvedCall) -> Option<ApexTypeRef>`
   arm. Inputs: the resolved inner-call's target method's
   `return_type` from `ApexClassSymbols`. Output: the type the
   outer call's receiver should resolve against.
2. The new arm runs once per chain depth — `a().b().c().d()`
   resolves left-to-right, threading the return type of each
   resolved call into the next receiver position. Stops when
   any intermediate call fails to resolve (silent fall-through
   to no edge for the remainder).
3. Confidence rule: if every intermediate call resolves at
   `High` or `Medium`, the outer call inherits `Medium`. If
   any intermediate is `Low` or unresolved, the outer call
   does not emit an edge.
4. Fixture set under
   `graphengine-parsing/tests/fixtures/apex_resolver/r45_chained_call_returns/`:
   - R45.A — `Singleton.getInstance().chained()` cross-file
   - R45.B — `new Builder().add().build()` intra-file
   - R45.A.NEG — chain with one unresolved middle (assert no
     edge for the tail; no false High emission)
   - R45.B.NEG — chain whose return type is `Object` (assert
     no edge; no nominal-typing fallback)
5. NPSP dry re-run after R45 lands: at minimum the
   `GE_SettingsService.getDataImportSettings` site recovers
   ≥ 1 incoming `Call` edge.

**Phase assignment.** Phase B (or a Phase-B-prerequisite ticket
in the universal-fidelity sprint if return-type propagation is
deemed prerequisite for any Layer-2 prototype that needs to
consume our `ApexTypeRef`). Out of scope for Phase A.

**Why this was not caught earlier.** No fixture in
TR-A.0–TR-A.6 chains a method call on a call-expression return
value. The TR-A.4 bare-self-dispatch arm covers the outer call
*only when the receiver is `this`* or implicit; chained calls
on return values are an orthogonal resolver gap that the Phase
A scope explicitly did not promise.

**Cross-reference.** Filed during PR 9 (R46 closure) as a
direct outcome of the rev 9 Round 5 hand-audit. See
`HAND_AUDIT_LOG.md` §"Round 5 — Engine revision 9" sample #4.

---

## R46. Cross-language reserved-keyword filter dropped legal Apex identifiers — `match`, `type`, `module`, `where`, etc. were silently filtered from the symbol extractor because they are reserved in some other language

**Status.** Closed — PR 9 (`name_validator.rs` per-language
reserved-keyword lists; integration test in
`graphengine-parsing/tests/apex_resolver_r46_keyword_extraction_fixtures.rs`).

**Discovered.** During rev 8 → rev 9 root-cause analysis of the
Phase A Round 5 gate failure on rev 8. Initial investigation
(samples 7 and 9 of the rev 8 draw) showed `match()` methods
absent from `SyntaxResults.symbols` despite being declared in
`RD2_OpportunityMatcher.cls`. AST inspection (via
`graphengine-parsing/tests/r46_diag.rs`, since deleted) confirmed
the `method_declaration` nodes were present in tree-sitter's parse
output but were dropped before reaching `class_symbols`.

**Root cause.** `graphengine-parsing/src/syntax/utils/name_validator.rs`
exposed a single global `RESERVED_KEYWORDS` constant containing
the union of every supported language's reserved word list. The
`is_reserved_keyword(name)` predicate was language-blind —
checking against the union, not against the calling extractor's
language. `symbol_extractor.rs` and `trait_context_detector.rs`
both invoked it without language context. Effect:

- `match` — reserved in Rust → dropped from Apex extraction
- `type` — reserved in TypeScript / Go → dropped from Apex
- `module` — reserved in TypeScript → dropped from Apex / Java / C#
- `where` — reserved in C# / Rust → dropped from Apex / Java
- `record` — reserved in Java 16+ / C# 9+ → dropped from Apex
- … and a dozen-plus other cross-language collisions.

The bug had been present since `name_validator.rs` was introduced;
no Apex fixture happened to use a method or class name that
collided with another language's reserved word, so the filter
was silently applied to every Apex codebase but invisible to the
crate's own tests.

**Fix shipped (PR 9).**

1. `name_validator.rs` refactored: `RESERVED_KEYWORDS` replaced
   with per-language `const &[&str]` arrays — `APEX_KEYWORDS`,
   `JAVA_KEYWORDS`, `CSHARP_KEYWORDS`, `RUST_KEYWORDS`,
   `TYPESCRIPT_KEYWORDS`, `JAVASCRIPT_KEYWORDS`, `PYTHON_KEYWORDS`,
   `GO_KEYWORDS`. `is_reserved_keyword(name, language)` looks up
   the language-specific list; unknown language falls through to
   "no keywords filtered" rather than the prior union behaviour.
2. `symbol_extractor.rs` and `trait_context_detector.rs` updated
   to pass `self.language_extractor.language()` as the language
   context.
3. Permanent regression fixture
   `graphengine-parsing/tests/fixtures/apex_resolver/r46_cross_language_keyword_names/CrossLangKeywordNames.cls`
   declares Apex methods named after keywords reserved in *other*
   languages: `match()`, `type()`, `module()`, `where()`,
   `record()`, plus the call-edge target shape so the fixture
   exercises both extraction visibility and resolver pickup.
4. Integration test
   `graphengine-parsing/tests/apex_resolver_r46_keyword_extraction_fixtures.rs`
   asserts every method is present in `SyntaxResults.symbols`
   AND that intra-file calls into them resolve to `Call` edges
   at expected confidence. Uses an `id → fqn` resolver helper to
   map node IDs back to FQNs for assertion logic.

**Closure does NOT imply Phase A gate passed.** R46 is an
extractor-layer cross-cutting fix, not an Apex-specific Phase A
TR-A.x deliverable. Closing it was correct and necessary — the
filter was silently corrupting Apex extraction since the
`name_validator.rs` introduction — but its closure surfaced
rather than satisfied the Phase A `< 2 / 10 wrong` gate. The
12 newly-extracted Function nodes per the rev 9 baseline
(`total_functions`: 14,936 → 14,948) entered the Round 5
sample pool and immediately revealed the underlying R39 / R41
/ R45 shapes that had been hidden by the extraction filter.

The honest framing is: R46 was load-bearing infrastructure debt
that had to be cleared before any Phase-A-class audit could
*see* the architectural gaps the audit was designed to
measure. R46 → R39 / R41 / R45 is the discovery chain Phase A's
audit-driven gate is supposed to surface. The product caught
the chain; the gate correctly refused to flip green.

**Phase assignment.** Closed (PR 9). Cross-cutting extractor
fix; affects every supported language whose keyword list
overlaps with another's, not just Apex. Other languages may
have the same shape; out-of-scope deep audit is filed as a
hygiene-backlog item for a future cross-language extractor
sweep.

**Cross-reference.** rev 9 baseline shipped in
`experiments/results/NPSP/rev9/`. PR 9 HEAD. See
`HAND_AUDIT_LOG.md` §"Round 5 — Engine revision 9" for the
Round 5 audit narrative that motivated and failed-to-be-closed-by
R46's fix.

---

## R37. Production `ApexHeuristicResolver` registry was never seeded with user-declared class symbols — every class-symbols-aware dispatch arm silently no-op on user code

**Status.** Closed — PR 3 (TR-A.3 field-type-aware dispatch) seeding hook.

**Observation.** The production factory
(`graphengine-parsing/src/application/use_cases/parse_repo/factory.rs:172`,
`ApexHeuristicResolver::with_standard_preload_only()`) constructs
the Apex heuristic resolver with an `ApexClassRegistry` seeded
only with Salesforce standard SObjects and system types. **At no
point** between factory construction and
`SemanticResolver::resolve` invocation did anything layer the
user's own class symbols (`SyntaxResults.class_symbols`, emitted
by `class_symbols_extractor.rs`) onto that registry.

Every class-symbols-consuming code path therefore read an empty
oracle for user types in production:

- **TR-A.1** (intra-file sibling constructor dispatch): looked up
  `registry.symbols_for(SiblingClassName)` → `None` → fell
  through to the name-only fanout path.
- **TR-A.2** (cross-file constructor dispatch): looked up
  `registry.symbols_for(OtherClassName)` → `None` → same
  fallback.
- **TR-A.3** (field-type-aware method dispatch, shipping in the
  same PR that discovered this gap): looked up
  `registry.symbols_for(field_declared_type)` → `None` → `resolve_field_type_call` returned
  `None`, TR-A.3 arm no-op, control fell to the name-only
  candidates path.

**Why the fixture tests did not catch it.** The TR-A.1 / TR-A.2 /
TR-A.3 fixture drivers
(`graphengine-parsing/tests/apex_resolver_r23_ctor_fixtures.rs`,
`apex_resolver_r23_a3_field_dispatch_fixtures.rs`) construct the
registry **themselves** via their own `build_registry_from_results`
helper before instantiating `ApexHeuristicResolver::new(registry)`.
That helper correctly two-passes `insert_user_declared` +
`attach_symbols` over `SyntaxResults.class_symbols`, so every
fixture ran against a fully-seeded registry and every assertion
passed. The production code path, which constructs the resolver
via `with_standard_preload_only()` and never touches the registry
again, had no equivalent seeding step. The gap was invisible to
every crate-local test because no test exercised the production
factory path end-to-end.

**Discovery.** After PR 3's field-type-aware dispatch tests
passed locally, a dry NPSP run was executed to confirm that the
three governing target FQNs (`UTIL_Permissions::canUpdate`,
`SfdoInstrumentationService::log`,
`Contacts::loadAccountByIdMap`) resolved live. The dry run
produced **zero** call edges to any of them. The Function nodes
were in the parse DB; the caller Function nodes were in the parse
DB; but the edges were missing. Tracing through the
`ApexHeuristicResolver::resolve` loop with added debug logging
revealed that `registry.symbols_for(field_type)` returned `None`
for every user-declared type the field resolver asked about, and
the TR-A.3 arm was therefore declining on every candidate call
site. Following `self.registry` back to its construction point
surfaced the `with_standard_preload_only()` call with no
downstream seed step.

**Blast radius.** Same caveat as R36: the SQLite `edges` table
uses `INSERT OR REPLACE` with a composite PK on
`(from_id, to_id, kind)`, so even though class-symbols-aware
dispatch silently no-op'd in production, the **absence** of
edges is the bug — not duplicated or corrupted edges. The
persisted impact is:

- **Apex parse DBs prior to this fix** (every rev through rev-6.1
  inclusive) carry **zero** edges that could only have been
  produced by class-symbols-aware dispatch in the heuristic
  resolver. `new X(...)` constructor edges that fell through to
  the name-only fanout and landed on the caller of a single
  matching function by coincidence are still present; the
  signal-carrying class-symbols path that TR-A.1 / TR-A.2 / TR-A.3
  were supposed to unlock simply never fired in production.
  Every baseline metric downstream (`no_callers`,
  `heuristic_call_fallbacks`, cohesion, fan-out, coupling) has
  been computed off a heuristic resolver that was no-op on every
  class-symbols arm.
- **R33 cross-language A/B (PR 2) remains valid.** R33 measured
  the delta added by extractor-level `new X(...)` CallSite
  extraction in Java, C#, TypeScript, and JavaScript — those
  languages have their own resolvers and their own registries and
  are not gated on the Apex registry. The Apex column of the R33
  A/B is what was disproportionately low; the other four
  languages were genuine gains.
- **LSP path unaffected.** The Apex LSP resolver
  (`apex-jorje-lsp`) resolves through the jorje type oracle, not
  the `ApexClassRegistry`. Users running Apex scans with a
  healthy LSP saw the real resolution numbers all along. The
  regression was purely on the heuristic-fallback path.
- **Non-Apex languages unaffected.** Only the Apex heuristic
  resolver consumes `ApexClassRegistry`.

**Fix shape.** Introduced
`apex::heuristic_resolver::seed_registry_from_hints(&self.registry, hints)`
as a first step inside every `ApexHeuristicResolver::resolve`
call. The helper clones the preloaded base registry and layers
on every entry from `SyntaxResults.class_symbols` using the same
two-pass `insert_user_declared` → `attach_symbols` pattern the
fixture drivers use, then returns a fresh registry that is
threaded through the rest of the resolve loop. All three
consuming arms (`constructor_resolver::resolve_constructor_call`,
`field_type_resolver::resolve_field_type_call`, and the
fallback-registry `.lookup(target)` step) now read the seeded
registry instead of `self.registry`.

Seeding lives inside `resolve` rather than inside `new` /
`with_standard_preload_only` because:

1. `resolve(&self, &SyntaxResults)` already carries the
   `SyntaxResults` argument — no plumbing required.
2. The registry needs to be rebuilt on every resolve call that
   may carry a different `class_symbols` set (e.g. incremental
   re-scans in Desktop). Caching on `&self` would require either
   interior mutability or a different factory contract.
3. A single short-circuit (`class_symbols.is_empty() → base.clone()`)
   makes the degenerate preload-only test path cheap.

Two regression tests pin this contract:

- `seed_registry_attaches_user_declared_class_symbols`: given a
  `SyntaxResults` with a declared class, the seeded registry
  must carry the class as user-defined AND expose its
  `ApexClassSymbols` payload via `symbols_for(...)`.
- `seed_registry_is_noop_when_hints_carry_no_class_symbols`: no
  class_symbols means the seeded registry is identical in size
  and collisions to the base registry.

Both sit in
`graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs`
so a refactor that drops the seeding step fails at the crate-lib
test layer, not only at the integration layer.

**Follow-through.** The rev-7 canary baseline (PR 7) MUST be
generated **after** this fix lands. The expected A/B direction:

- `heuristic_call_fallbacks` on Apex rises by the count of
  member-call sites that TR-A.1 / TR-A.2 / TR-A.3 now resolve to
  a typed target (the very edges this gap was suppressing).
  Directional sign is positive; magnitude depends on the NPSP
  shape of typed-field calls and sibling / cross-file ctor
  invocations.
- `no_callers` on the Apex Function node set falls by a similar
  count (modulo self-loops).
- Non-Apex language baselines are unchanged.
- `heuristic_call_ambiguous_drops` may fall because
  class-symbols-aware dispatch collapses name-only fanouts that
  previously tripped the `HEURISTIC_CALL_FANOUT_CAP` into single
  Medium-confidence edges.

The PR-7 baseline write-up should label this delta as "R37
registry-seeding fix (class-symbols-aware dispatch now fires in
production)" separate from TR-A.3's independent contribution, so
the envelope check does not read the combined uplift as a single
TR-A.3 signal.

**Parallel-resolver audit.** Non-Apex heuristic resolvers were
audited for the same shape: every other language's resolver
either consumes `SyntaxResults` directly (no registry
intermediary) or constructs its type oracle per-resolve inside
`resolve(...)`. No other language carries the "construct
resolver with empty oracle at factory, never seed it" anti-shape.

---

## R36. Apex AND Java `method_invocation` YAML queries double-counted every member-call site

**Status.** Closed — PR 3 (TR-A.3 field-type-aware dispatch).

**Blast radius correction.** Initially attributed to Apex only;
sweep-audit during the fix surfaced the identical shape in
`configs/java.yaml` → **both** Apex and Java had the bug. Fixed
in the same commit as PR 3. Non-Apex / non-Java languages (C#,
TypeScript, JavaScript, Go, Rust) are not affected — their two
`call_sites` patterns distinguish `function:` child-node types
(identifier vs member/selector expression) and are already
mutually exclusive.

**Observation (Apex).** `graphengine-parsing/configs/apex.yaml`
§`call_sites` declared two `method_invocation` patterns back-to-back:

```
(method_invocation
  name: (identifier) @method
  arguments: (argument_list) @args
) @method_call

(method_invocation
  object: (_) @receiver
  name: (identifier) @method
  arguments: (argument_list) @args
) @method_call
```

Tree-sitter matches patterns **independently**. Without a
negated-field predicate, the first pattern matches every
`method_invocation` node — including the `obj.method(args)` shape
— and the second pattern matches the same node again, emitting
**two identical call sites per member call**. The receiver-less
first site and the receiver-carrying second site both carried the
same function name and location, and the `ApexHeuristicResolver`
(and, pre-TR-A.3, the `ApexLspResolver` paths) emitted one edge
per call site → two call edges per member call throughout Apex.

Bare method calls (`method(args)`, no receiver) matched only the
first pattern and were emitted once.

**Observation (Java).** `graphengine-parsing/configs/java.yaml`
carried the identical two-pattern shape (inherited from the same
template as Apex when the Java extractor was added) and had the
same behaviour: every Java member invocation double-emitted. Bare
Java `method(args)` calls were single-emitted. Non-method call
shapes (`object_creation_expression`, etc.) were unaffected.

**Why it went unnoticed.** The existing TR-A.1 / TR-A.2
constructor fixtures exercised `object_creation_expression`, which
only has one ctor pattern — the duplicate never reproduced. Broad
corpus tests (`apex_heuristic_corpus.rs`) assert edge presence /
content but do not assert **edge cardinality per call site**, so a
2× inflation was invisible to the lock.

**Discovery.** TR-A.3 (PR 3) added an assertion that each
field-type-aware dispatch fixture emits exactly one Medium
heuristic edge to the declared target method. The very first run
produced two identical edges per fixture, with identical `(from,
to, provenance)` tuples. Diagnostic logging on `call_sites`
confirmed two entries per `obj.m(args)` source line — one with
`receiver_text = None`, one with `receiver_text = Some("obj")` —
both at the same `Range`.

**Blast radius (direction-of-change, no freshly measured numbers).**

Critical caveat: the SQLite `edges` table has a composite primary
key on `(from_id, to_id, kind)` and the persistence layer uses
`INSERT OR REPLACE`
(`graphengine-parsing/src/infrastructure/storage/sqlite_repository.rs:256`).
**Duplicate call edges were therefore silently deduplicated at
persistence time** — the bug inflated in-memory resolver counters
but did not corrupt the persisted edge set. This materially
narrows the blast radius relative to the initial read of the bug:

- **Persisted DB (parse.db `edges` table) for Apex AND Java** —
  **not** inflated. Every prior baseline's `edges` table matches
  the post-fix baseline for the member-call axis, because the
  second duplicate emission collapsed under the PK on insert. All
  downstream analysis consumers that read from the parse DB
  (`graphengine-analysis`, HealthReport, metric envelopes,
  cohesion / fan-out / centrality / coupling, Round 4 hand audit
  samples) are unaffected.
- **In-memory telemetry counters** — inflated. The resolver's
  `heuristic_call_count` and its `Heuristic=N` logline (e.g.
  NPSP: `Heuristic=141874` on this parse run) over-counted every
  Apex / Java member call by 1× because the increment happened
  per call site in the loop before the persistence-layer dedupe
  collapsed the duplicate pair. Downstream analytics driven by
  the parse-log telemetry — `R33_CROSS_LANGUAGE_EVIDENCE.md`
  tables, any telemetry-sourced "total heuristic edges" figure in
  planning docs — are inflated for Apex and Java.
- **R33 A/B deltas (PR 2) remain valid.** R33 measured the
  delta added by ctor-call edges
  (`object_creation_expression`), which are not covered by the
  doubled `method_invocation` patterns, and the PR-2-OFF /
  PR-2-ON runs both carried the same member-call over-count.
  The PR-2 delta is signal, not artefact.
- **C#, TypeScript, JavaScript, Go, Rust baselines are
  unaffected.** Their call-site queries distinguish `function:`
  child-node types (identifier vs member/selector expression)
  and were already mutually exclusive as written.

The fix is still worth shipping — resolver work is wasted on
duplicate call sites, and field-type-aware dispatch assertions
(TR-A.3 fixtures) require a unique-edge-per-call-site contract to
distinguish "resolved by TR-A.3" from "also resolved by the
fallback path". But the scope of the correction is
**telemetry-only**, not "re-baseline everything".

**Fix shape.** Added a `!object` negated-field predicate to the
first pattern in **both** `configs/apex.yaml` and
`configs/java.yaml` so tree-sitter matches only receiver-less
`method_invocation` nodes. The two patterns are now mutually
exclusive and each call site is emitted exactly once:

```
(method_invocation
  !object
  name: (identifier) @method
  arguments: (argument_list) @args
) @method_call

(method_invocation
  object: (_) @receiver
  name: (identifier) @method
  arguments: (argument_list) @args
) @method_call
```

Enforcement of single-emission lives at the query shape, not as a
defensive dedupe pass in the extractor — the latter would mask
any future regression in a similar query.

**Follow-through.** Because the DB was never corrupted (see
caveat above), the rev-7 canary baseline supersedes no prior DB
artefact — `experiments/results/NPSP/rev6.1/parse.ab.sqlite` and
all historical rev baselines retain their edge-table integrity.
The rev-7 baseline WILL show a lower "Heuristic=N" telemetry
number than the rev-6.1 log shows; the PR-7 write-up must label
that reduction as "member-call dedupe
(Apex+Java)", distinct from TR-A.3 / TR-A.4 / TR-A.6 dispatch
gains, so the envelope check does not misread the telemetry
drop as a correctness regression. Round-5 hand-audit draws
against the rev-7 parse DB are unaffected (DB was never
inflated).

**Parallel-language audit (completed).** Grepped every other
`configs/*.yaml` for the same shape (two method-invocation
patterns without a negated-field predicate). Only Apex and Java
used the `method_invocation` node with the two-pattern shape.
C#, TypeScript, JavaScript, Go, and Rust queries separate their
two `call_expression` / `invocation_expression` patterns by
`function:` child-node type (identifier vs member / selector
expression) and are mutually exclusive as written.

## R35. Analysis-pipeline non-determinism masks real regressions in every byte-identical / trend comparison

**Status.** Closed by PR 5.5 (WS-TRUTH-R35). All four recommendation
sub-items below landed:

1. Cohesion `modules` map -> `BTreeMap` (deterministic ID assignment).
2. Coupling `modules` map -> `BTreeMap` (deterministic finding order).
3. `distance_from_main_sequence::compute_distance` returns
   `BTreeMap<String, ModuleDistance>`; iteration keys sorted before
   accumulation (deterministic avg + finding order).
4. Regression gate:
   `graphengine-analysis/tests/determinism_integration.rs` runs
   `ge-analyze` twice on a synthetic multi-module DB and asserts
   byte-identical JSON output (after normalising `generated_at`,
   `analysis_duration_ms`, `engine_commit`, `engine_version`).

Re-armed `experiments/bin/assert_rev6_1_byte_identical.sh` on the
refreshed post-R35 baseline confirms identical normalised sha256
between the committed rev-6.1 baseline and a fresh local run
(`cbf0c11a…06bf1f`).

**Historical context retained below for audit trail.**

**Original status.** Open. Surfaced during WS-TRUTH-A PR 2 (TR-A.1 + TR-A.2 + R33)
pre-flight while diagnosing why
`experiments/bin/assert_rev6_1_byte_identical.sh` could not be used to
verify PR 2 did not perturb analysis on older parse DBs.

**Observation.** Running `ge-analyze` twice on the *same*
`experiments/results/NPSP/rev6.1/parse.ab.sqlite` from two independent
cargo builds at the same git SHA (`7cba9a9`) produces a ~24 000-line
normalised JSON diff. Concrete drift categories observed in that diff:

- Cohesion finding IDs reshuffled across runs (same findings, different
  identifier assignment) — points at HashMap iteration order being used
  to stamp IDs.
- `fan_out` counts shifted by ±1 on a subset of nodes — points at a
  non-deterministic aggregation (HashSet / HashMap iteration reaching a
  per-node accumulator in a different order).
- God-function rankings rotate — consistent with floating-point
  aggregation where summation order changes the last-ulp of the score
  used for ordering.
- `avg_cohesion` differs by a single float ulp between runs.

`ge-analyze` is read-only against the parse DB, so none of this comes
from parsing; the drift lives inside `graphengine-analysis`.

**Risk.** This is not a CI-tooling problem; it is a product-trust
problem with three concrete blast radii:

1. **Every byte-identical regression gate is defeated by noise.** The
   TR-A.0 `assert_rev6_1_byte_identical.sh` gate cannot detect a real
   regression under the current noise floor (24 k-line diff on a
   no-op rebuild vs. the 57-line diff observed between PR-2-on and
   PR-2-off). Every future Phase gate that asserts byte-identical
   behaviour on older DBs (Phase B, Phase C, Phase D) inherits the
   same blindness.
2. **Any trend product is noise-dominated.** The WS-DESKTOP trend
   endpoint (C.6) and the WS-DIAG trend ticket (T3) both assume that
   running the engine twice against the same repo yields identical
   metric inputs. Currently they cannot. Real "this metric got worse
   / better" signals will be mixed with per-run shuffling.
3. **Saved-report diffing is a lie.** The Desktop UI's "compare two
   reports" flow (future) would show spurious cohesion-finding
   reassignments and last-ulp metric differences on two reports of
   the same codebase scanned on the same day. That is the exact shape
   of false precision R1 / R2 were raised to prevent, but
   mechanically produced instead of propagated from a parser bug.

**Recommendation.** Single targeted sweep in `graphengine-analysis`:

1. **Deterministic cohesion finding IDs.** Replace any
   HashMap-iteration-derived ID assignment with an ordered walk
   (BTreeMap, or explicit sort on the key tuple used to canonicalise
   the finding) before ID allocation. Likely site:
   `graphengine-analysis/src/health/cohesion/` and its reporter.
2. **Deterministic fan-out / per-node aggregations.** Audit every
   `HashSet`/`HashMap` iteration that feeds into a numeric or ordinal
   metric; replace with sorted iteration or accumulate into a sorted
   structure before reducing.
3. **Deterministic floating-point aggregation.** For averages /
   weighted means, sort contributions by a canonical key before
   summation (or use a pair-wise / Kahan summation that's tolerant to
   input order if the sort is prohibitive). This is the last-ulp
   avg_cohesion issue.
4. **Regression gate for this property itself.** Add an analysis-crate
   integration test that runs `ge-analyze` against a canned parse DB
   twice in the same process and asserts byte-identical JSON output.
   Without this gate, R35 regresses the day someone adds a new
   `HashMap`-backed metric.

**Ordering constraint.** R35 must ship before
`experiments/bin/assert_rev6_1_byte_identical.sh` is re-armed (i.e.
before the rev-6.1 parse DB + baseline JSON artefacts are vendored
under `experiments/results/NPSP/`). Attempting to re-arm the gate on
top of R35-unresolved analysis will produce permanent spurious failures
and teach the team to ignore the gate — defeating the gate's purpose.
Concrete sequence: land R35 → refresh `rev6.1/baseline.json` against a
deterministic HEAD build → confirm script exits 0 → then apply PR 2
(and PR 3–7 deltas) on top and re-measure.

**Phase assignment.** Not WS-TRUTH-A Phase A scope (resolver work,
parse-side only). Owns a new sprint-plan row **WS-TRUTH-R35** scheduled
before the rev-6.1 artefact vending that unblocks the TR-A.0 byte-
identical gate in CI. Directional priority: before Phase B ships (Phase
B will ship more parse-DB shape changes that deserve a working byte-
identical gate), and before WS-DESKTOP-C.6 / T3 trend work (they need
reproducible inputs).

---

## Hygiene backlog (non-truth-impacting)

Items here are real observations surfaced during the WS-TRUTH
Phase-0 work that do not deserve their own R# because they do
not bear on engine correctness or customer-facing trust. Kept
recorded so future sweeps do not lose them.

- **Pre-existing `useless use of vec!` warning** in
  `graphengine-parsing/tests/domain/benches/graph_bench.rs`.
  Surfaced by `cargo clippy --all-targets` during TR-0.2
  validation; pre-dated the change. Not in the `--lib --tests`
  scope the WS-TRUTH CI gate applies, so it does not block
  merges — but it does mean `cargo clippy --all-targets` is
  noisy on any future run. Fix is a one-line `Vec::new()`
  substitution; bundle with the next unrelated bench edit or
  a dedicated hygiene sweep.

- **`assert_rev6_1_byte_identical.sh` has drifted from `ge-analyze`
  CLI and the committed rev-6.1 baseline is stale on HEAD.**
  Verified during PR 2 (TR-A.1 + TR-A.2 + R33) pre-flight:
    - The script invoked `ge-analyze --db ... > out.json` but the
      current `ge-analyze` requires `--output <OUTPUT>` as a flag
      rather than stdout piping. The missing flag caused the CLI
      to exit with a usage error, which the script silently ignored
      (the downstream `shasum`/`diff` operated on empty normalised
      files and returned exit 0 by accident). Patched in the same
      PR to pass `--output` explicitly.
    - Running the patched script on a clean `7cba9a9` tree (no PR 2
      changes) against `experiments/results/NPSP/rev6.1/parse.ab.sqlite`
      still produces a ~24 000-line normalised JSON diff vs
      `experiments/results/NPSP/rev6.1/baseline.json` — the gate
      is pre-existing-broken independent of PR 2. Evidence: cohesion
      finding IDs/values re-shuffled, fan_out counts shifted by ±1,
      god-function rankings rotated. Strongly suggests analysis-
      pipeline non-determinism (HashMap iteration order + float
      summation order) combined with at least one real shape change
      between `3ed4df6` (TR-A.0 ship) and `7cba9a9` that never got
      re-baselined.
    - Comparing PR-2-on vs PR-2-off outputs on the same rev-6.1 DB
      yields only a 57-line diff — cohesion ID re-shuffling and a
      single floating-point ulp in `avg_cohesion`. `ge-analyze` is
      read-only and cannot re-resolve, so none of this can come
      from PR 2's parser changes; it is noise from the aforementioned
      non-determinism across cargo rebuilds.
    - **Implication for rev-7:** The PR-2 byte-identical assertion
      against rev-6.1 cannot be evaluated until the gate is rebuilt:
      (i) fix non-determinism in the cohesion-finding ID assignment
      and avg_cohesion aggregation — **promoted to R35, tracked as
      WS-TRUTH-R35** (re-classified from hygiene to full risk; see
      §R35 above for blast radii and fix shape), (ii) refresh
      `experiments/results/NPSP/rev6.1/baseline.json` against a
      deterministic HEAD build, (iii) re-run the gate to confirm
      parity, (iv) then apply PR 2 on top and re-measure. Until R35
      ships, the gate is non-actionable. Sub-bullet (i) is the
      correctness work and no longer belongs under "hygiene"; this
      hygiene-backlog entry retains only the CLI-drift patch record
      and the ordering pointer into R35.
    - **Expected rev-7 baseline shift (when gate is re-armed and run
      on a freshly-parsed NPSP DB with PR 2 bits):**
        * `heuristic_call_fallbacks` increases by the number of Apex
          constructor call-sites that TR-A.1 / TR-A.2 successfully
          resolve at Medium confidence (sibling-inner, cross-file,
          overloaded, `this(...)` / `super(...)`).
        * `no_callers` decreases by the same count (bar self-loops).
        * `dynamic_dispatch_target` unchanged (Phase A does not
          touch interface / virtual-dispatch resolution).
        * `framework_annotation_unresolved` unchanged.
        * `declarative_wiring_unparsed` unchanged.
        * R33 unlocks `new X(...)` CallSite extraction in Java, C#,
          JavaScript, TypeScript — but NPSP is Apex-only, so the
          canary sees only the Apex-side edge gain.

- **NPSP rev-6.1 artefact tracking is half-complete after PR 5.5.**
  Surfaced during the PR 5.5 artefact-vending step. The
  `assert_rev6_1_byte_identical.sh` gate needs two inputs on a fresh
  clone: the frozen parse DB and the reference baseline JSON. PR 5.5
  tracks both as `experiments/results/NPSP/rev6.1/parse.ab.sqlite` +
  `rev6.1/baseline.json` (with top-level symlinks into `rev6.1/` so
  there is one source of truth, ~76 MB in git instead of ~152 MB).
  Three adjacent artefacts that the docs still reference are **not**
  yet tracked and will cause confusion on a fresh clone:
    1. `experiments/results/NPSP/rev6.1/parse.db` — the pre-A/B
       parse DB. Referenced in
       `docs/workstreams/proof-foundation-gap/PHASE_A_EXECUTION_PLAN.md`
       §4.1 (re-run contract) + §10 (fresh-parse replay recipe) and in
       `REGRESSION_RESULTS.md` §rev-6.1. Gitignored by `*.db`. Either
       add a `.gitignore` exception symmetric to `parse.ab.sqlite`, or
       rewrite the docs to drive off `parse.ab.sqlite` (which the gate
       already consumes). Recommendation: rewrite the docs — the gate
       contract is what actually ships; `parse.db` is a cachable
       intermediate.
    2. `experiments/results/NPSP/rev6.1/ab_report.json` — the
       rev-6.1 A/B-injected HealthReport. ~13 MB. Referenced in
       `PHASE_A_EXECUTION_PLAN.md` §4.1 ("Same script runs for the
       A/B artefact") and `REGRESSION_RESULTS.md` §rev-6.1 artefacts.
       Not gitignored by pattern; simply not `git add`-ed. No gate
       asserts on it yet — the PR 7 metric-envelope check in
       `PHASE_A_EXECUTION_PLAN.md` §11 compares the fresh rev-7
       `ab_report.json` against the rev-6.1 one. If that comparison
       is to be CI-enforceable, commit the rev-6.1 `ab_report.json`
       alongside the baseline; otherwise document it as "regenerate
       locally before running §11".
    3. `experiments/results/NPSP/rev6.1/parse.ab.sqlite-shm` +
       `parse.ab.sqlite-wal` sidecars are present in the working tree
       but correctly gitignored (SQLite regenerates them on open);
       mentioned here only so future readers do not mistake their
       absence from `git ls-files` for a vending gap.
  Bundle with PR 7 doc sweep (single commit that also does
  `REGRESSION_RESULTS.md` + `PHASE_A_EXECUTION_PLAN.md` updates).

- **`rev6.1/` directory contents are semantically heterogeneous after
  PR 5.5.** The directory now carries (a) the frozen rev-6.1 parse
  artefact (`parse.ab.sqlite`, binary-identical to the rev-6.1 ship)
  and (b) the post-R35 analysis baseline (`baseline.json`, generated
  by HEAD against that same parse DB). The naming `rev6.1/` implies
  everything inside is a rev-6.1 ship artefact, but the baseline is
  "HEAD-analysis-against-rev-6.1-parse" — a floating reference that
  shifts every time a deterministic analysis change lands. Two
  defensible strategies, either is fine:
    * Rename `rev6.1/` → `NPSP_rev6_1_parse/` and keep `baseline.json`
      in it, with a one-page `README.md` that states the contract
      ("the parse DB is frozen at rev 6.1; the baseline is regenerated
      every time the analysis pipeline changes shape — see
      §`assert_rev6_1_byte_identical.sh`").
    * Keep `rev6.1/` but split baseline into
      `baseline.head.json` (floats with HEAD) vs the historical
      `baseline.rev6_1_ship.json` (frozen; committed for audit
      replay only).
  Low priority — record during PR 7 doc sweep. Strategy 1 is simpler
  and aligns with the "single source of truth" symlink pattern.

- **`assert_rev6_1_byte_identical.sh` normaliser now also strips
  `db_path`.** PR 5.5 addition. `db_path` is `ge-analyze` invocation
  metadata (absolute vs relative depending on how CI vs local
  developers call the binary) and is not part of the byte-identical
  analysis contract. Recorded here so the normaliser growth is
  auditable — the rule is "normaliser strips non-analysis-output
  fields; strip-list growth requires a written justification". If a
  future diagnostic report adds a field that is per-invocation,
  extend the normaliser and append to this log.

- **TR-A.5 NPSP dry-run: 12 of 80 `.page` files reject under
  `quick-xml` strict parsing.** PR 6 shipped with `check_end_names =
  true` + `trim_text = true` on the reader. NPSP ships 80 pages;
  67 parse cleanly (134 synthetic nodes, 161 VF→Apex `Call` edges: 65
  Medium + 96 Low-confidence fanout, 113 bindings resolved at the VF
  stage). The 12 rejects are all real VF shapes that contain one of:
  unescaped ampersands inside attribute values, unmatched end tags
  emitted by the VF compiler (e.g. `<apex:param ...>` self-closed
  without `/`), or embedded `<script>` with `&lt;` / `&gt;` not
  properly escaped. Attempting a lenient `check_end_names = false` +
  `trim_text = false` on the same set parses more but produces
  spurious `Event::Start` events for self-closing tags, which would
  need a corresponding shape change in `read_vf_page_from_str`. Out
  of TR-A.5 scope (attribute-only); carry into Phase C TR-C.3
  (VF text-node + rerender + actionFunction) as a prerequisite: the
  lenient-mode scan needs to exist first so the richer VF surface
  isn't blocked on XML strictness. Failing pages (exact list as
  recorded by the VF stage WARN log) are ALLO_ManageAllocations,
  BDI_DataImport, CON_ContactMerge, CONV_Account_Conversion,
  LD_LeadConvertOverride, MTCH_FindGifts, REL_RelationshipsViewer,
  STG_PanelAddrVerification, STG_PanelCustomizableRollup,
  STG_PanelOppNaming, STG_PanelRD2Enablement, STG_SettingsManager.
  Each still appears in the source tree; they simply do not
  contribute synthetic VF nodes in Phase A.

- **TR-A.5 plan §7.4 named `UTIL_JobProgress.page` as the NPSP
  acceptance exemplar; that file does not exist in NPSP HEAD.**
  NPSP ships `UTIL_JobProgress_CTRL.cls` but no `UTIL_JobProgress.page`.
  Confirmed by `find ~/Desktop/apex_baseline_repos/NPSP -name
  'UTIL_JobProgress*'` on the PR 6 drop. The shape *is* proven by the
  fixture bundle `tests/fixtures/apex_resolver/r23_a5_vf/util_jobprogress/`
  (which exercises the same controller-binds-action idiom end-to-end
  through the extractor + VF stage + resolver) and by 65 Medium-
  confidence VF→Apex `Call` edges on real NPSP pages (see §R35 dry-run
  attachment). PR 7 doc sweep should reword §7.4 to either: (a) point
  at a representative NPSP page that does exist (e.g. `CON_DeleteContactOverride`
  → `CON_DeleteContactOverride_CTRL::processDelete()`, 4 Medium edges
  from one page; or `BDE_BatchEntry` → 2 edges); or (b) keep
  `UTIL_JobProgress_CTRL::refreshJobs` as the fixture exemplar and
  rename the NPSP acceptance line to "N Medium-confidence VF→Apex
  edges across M pages, with N, M ≥ plan thresholds". Recommendation:
  (b), because the fixture exemplar is the right level of detail for
  a worked example and the real NPSP pages are naturally covered by
  the §11 metric-envelope counts.

- **TR-A.5 Visualforce FQN shape: plan prose vs mirror-instruction
  conflict resolved in favour of the trigger mirror.**
  `PHASE_A_EXECUTION_PLAN.md` §7.2 item 4 specifies the synthetic VF
  node FQN twice, inconsistently: a literal form
  `<repo_path>::__vf_page__::<PageName>` *and* an instruction to
  "mirror the existing `__trigger__` convention". The existing
  `__trigger__` shape (see
  `graphengine-parsing/src/syntax/language/apex/fqn.rs`
  `build_trigger_body_fqn` / §extractor.rs L192–L195) is
  `<path>::<TriggerName>::__trigger__()` — i.e. the PageName sits in
  the container slot and `__trigger__()` / `__vf_page__()` sits in
  the method slot.
  PR 6 (TR-A.5) ships the mirror shape:
  container `Struct` FQN `<path>::<PageName>` + body `Function` FQN
  `<path>::<PageName>::__vf_page__()`. Three reasons the literal
  prose form was rejected:
    1. The literal form reverses the container/method order relative
       to every other Apex synthetic node; every downstream query
       written against the trigger shape would break.
    2. The mirror form gives Phase C TR-C.3 a stable container per
       PageName onto which further VF surfaces (rerender, action-
       function, text-node `{!...}` expressions) can hang without
       having to reshape the graph.
    3. The plan's §7.3 fixture table and §7.4 acceptance both speak
       of "one synthetic `__vf_page__` node per `.page` file" which
       is consistent with either shape; the literal prose is the
       only piece that disagrees, and it is the least-load-bearing
       of the three. The decision is recorded permanently in the
       `build_vf_page_body_fqn` docstring so the trade-off is
       discoverable from code as well as from this log.
  Action for PR 7 doc sweep: reconcile §7.2 item 4 of
  `PHASE_A_EXECUTION_PLAN.md` onto the mirror form (so the plan
  matches what shipped) and record the decision in the plan's
  appendix. Until then, reading §7.2 in isolation will mislead a
  future reader into thinking the literal prose is authoritative.

---

## Hygiene-backlog decisions (2026-04-19, universal-fidelity sprint Phase 0)

PR 7's pending doc-sweep bundle asked for three concrete decisions
on the rev-6.1 artefact-tracking items filed above. Phase 0 of the
universal-fidelity sprint (`docs/workstreams/universal-fidelity/`)
records the decisions here rather than embedded inside the
individual hygiene bullets, so the bullets above stay as-found
history and future sweeps look at this block for "what was
decided."

**(a) `rev6.1/parse.db` vending.** **Decision: rewrite docs onto
`parse.ab.sqlite`.** The byte-identical CI gate already consumes
`parse.ab.sqlite`; `parse.db` is a cachable intermediate and its
absence from `git ls-files` has never broken a pipeline. Doc
rewrites target `PHASE_A_EXECUTION_PLAN.md` §4.1 and §10 plus
`REGRESSION_RESULTS.md` §rev-6.1; any remaining `parse.db`
reference in docs becomes a broken-link hygiene follow-up if it
leaks past PR 9. Rationale: the gate contract is what ships; the
developer-local replay recipe drives off the same artefact the CI
uses. This decision also simplifies the universal-fidelity
sprint's T2 (content-stable IDs) work, which will need a single
canonical parse-DB location per canary rather than two.

**(b) `rev6.1/ab_report.json` vending.** **Decision: regenerate
locally; do not vend.** The envelope-comparison step
(`PHASE_A_EXECUTION_PLAN.md` §11) that consumes `ab_report.json`
is a manual pre-flight today, not a CI gate. Committing a 13-MB
analysis artefact that drifts every time the analysis pipeline
emits a new deterministic field adds tracking burden without a
matching CI enforcement benefit. Doc action: update
`PHASE_A_EXECUTION_PLAN.md` §11 to explicitly state "regenerate
`ab_report.json` locally before running the comparison" and link
to the one-line reproducer. If the universal-fidelity sprint's
T3 (dual-metric emission) elevates the envelope comparison to a
CI gate, revisit this decision in the same commit.

**(c) `rev6.1/` directory semantic-drift rename.** **Decision:
deferred until T2 (content-stable IDs) lands.** T2 in the
universal-fidelity sprint changes the node-ID derivation
formula, which invalidates every persisted `parse.ab.sqlite` —
the rev-6.1 artefact will need to be re-parsed against the new
formula anyway. Renaming the directory *now* and then re-parsing
is duplicated churn; waiting until T2 ships means one rename
does both jobs. Recorded action: during T2's ship, rename
`experiments/results/NPSP/rev6.1/` → `NPSP_rev6_1_parse/` and
commit a one-page `README.md` stating the contract (parse DB is
frozen; baseline regenerates against HEAD; byte-identical gate
rules). Placeholder entry added to the universal-fidelity
sprint's T2 ticket.

These three decisions close the PR 7 hygiene-backlog open items
transferred into the universal-fidelity sprint. The hygiene
bullets above are kept as the historical record of what was
surfaced; this block is the authoritative decision record.

## R46. Constructor-call position arithmetic off-by-segment-length — `session.rs:find_definition` pushes cursor past the `new` keyword for `Class::new` symbols

**Shape.** The Apex extractor encodes a constructor call `new Foo()` as
a synthetic call-site whose `function_name` is `Foo::new` (using the
Rust-style `::` separator even though Apex source itself writes `new Foo()`).
In `graphengine-parsing/src/infrastructure/lsp/session.rs` §941 the
definition handler then runs this adjustment:

```rust
if let Some(last_segment) = symbol_name.rsplit("::").next() {
    if let Some(offset) = symbol_name.rfind(last_segment) {
        character = character.saturating_add(offset as u32);
    }
}
```

For `symbol_name = "Property__c::new"`, `last_segment = "new"` and
`offset = rfind("new") = 13`.  Tree-sitter already hands us the
`start_char` of the call-site on the `n` of the `new` keyword, so the
correct LSP column is the cursor position as-is.  Adding 13 then
pushes the cursor 13 columns rightward, landing *past* `Property__c`
on the line `Property__c property = new Property__c();`.  jorje is
asked about a position that is effectively mid-way through the second
`Property__c` token, not the `new` keyword or the identifier that
follows it.

**Evidence.**  Run
`experiments/results/jorje-p0-debug-2026-04-18/trace.jsonl` (from
Tier 3 of the 48 h demo push) records all 28 `::`-bearing symbols
with `character = byte_col + 13` (or `+6`, `+14`, `+21`, etc.
depending on the prefix length).

**Why this is distinct from the P0 (indexing-timing).**  The H1
root cause means *every* request returns null regardless of
position, so R46 is masked on the current scan.  But once the
readiness barrier is tightened so jorje can actually resolve,
constructor calls will still miss until R46 is fixed.

**Recipe.**
1. In `session.rs:find_definition`, special-case the synthesized
   `X::new` shape: when `last_segment == "new"`, do *not* apply the
   segment-offset.  Tree-sitter's `start_char` is the correct cursor
   already (on the `new` keyword).
2. Audit the general case too: the current arithmetic assumes
   tree-sitter hands us the column of the *first* character of
   `symbol_name` as spelled (i.e. the type-qualifier), which matches
   only a subset of extractor emissions.  Prefer computing a
   "column of the callee identifier" signal at extraction time and
   stashing it in `CallSite` so this arithmetic can go away
   entirely.
3. Add a regression fixture at
   `graphengine-parsing/tests/lsp_constructor_position.rs` that
   builds a synthetic one-file Apex source, asks the real jorje (LSP
   feature-gated) to resolve `new Foo()`, and asserts the resolved
   location lands on the class declaration of `Foo` rather than on
   an unrelated token.

**Out of scope.**  Any fix here needs R46's chicken-and-egg with the
P0 resolved first (the fixture would return null today for
timing-not-position reasons).  R46 is tracked but not blocking the
48 h demo window.

## R47. Apex subtype-dispatch invisibility — interface-typed and base-class-typed call sites drop every receiver-bound call

**Shape.** The heuristic resolver attributes a method call `receiver.foo(...)` by
walking the class symbols registry for `receiver`'s declared type and
looking for a method named `foo`. When the receiver is typed as an
*interface* or *abstract base class*, the dispatch table on the declared
type does not contain an implementation body — only the signature — so
the resolver finds no method node and the call is dropped. Every caller
that holds the supertype reference is therefore invisible to every
concrete override:

- **R47.A Interface dispatch.** Receiver is typed as an interface (e.g.
  `fflib_IDomain dom = new fflib_Ids(...); dom.getObjects();`). The
  interface carries only the abstract signature; all implementations
  sit on concrete classes the resolver cannot reach from the
  interface's dispatch slot.
- **R47.B Virtual / abstract override via base-class reference.**
  Receiver is typed as the base class (e.g.
  `BDI_ObjectMappingLogic logic = ...; logic.populateObjects(...);`
  where `BDI_CustObjMappingGAUAllocation extends BDI_ObjectMappingLogic`
  and overrides `populateObjects`). The base class carries a virtual
  body that IS resolvable, but the override on the subtype is never
  attributed an inbound edge — subtype inbound fan-in is systematically
  zero.

**Evidence — NPSP rev 11, Round 5b hand-audit (seed 20260418).**

Three of ten draws fell into R47.A (`fflib_IDomain::getObjects()`,
`fflib_ISObjectUnitOfWork::registerDeleted(SObject)`,
`fflib_ISObjectUnitOfWork::registerRelationship(SingleEmailMessage,SObject)`).
One of ten fell into R47.B (`BDI_CustObjMappingGAUAllocation::populateObjects(BDI_ObjectWrapper[])`).
See `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md` "Round 5b"
for FQN-level evidence and direct source citations.

**Why this is distinct from R11/R44.** R11 tracks the `implements` /
`extends` edge direction; R44 tracks the inheritance-direction
asymmetry on those edges. Both are *schema-level*: they populate the
structural relationship. R47 is a *resolver-level* gap: even when the
inheritance edges exist, the heuristic's receiver-type lookup never
traverses them when looking up a method name. The two must close
together (schema correct AND resolver traverses) for subtype dispatch
to resolve.

**Why this is distinct from R45.** R45 is chained-call return-type
propagation inside a single expression (`first().second()`). R47 is
receiver-variable type lookup across a *declaration*
(`IDomain dom = ...; dom.foo()`). Different lookup paths; non-overlapping
failure modes.

**Recipe.**
1. Extend `ApexHeuristicResolver::dispatch_member_call` so that when a
   method name is not found on the *declared* receiver type, it walks
   the inheritance-edges index and retries the lookup on each direct
   subtype (for interface receivers) or walks the override-chain down
   from the base class (for abstract/virtual base receivers).
2. For interfaces: every class in the symbol registry that `implements`
   the interface is a candidate target; the resolver emits a call edge
   to each concrete implementation (low confidence, with a
   `dispatch_kind: "interface_polymorphic"` tag on the edge).
3. For virtual base classes: every class in the subtype-closure that
   `override`s the method is a candidate target; same low-confidence
   edge shape with `dispatch_kind: "virtual_polymorphic"`.
4. Pre-register two regression fixtures:
   - `apex_r47a_interface_dispatch_e2e.rs` with
     `fflib_IDomain dom = impl; dom.getObjects();` proving a call edge
     lands on the concrete implementation.
   - `apex_r47b_virtual_override_e2e.rs` with a base-class reference
     and an overriding subtype proving a call edge lands on the
     override.
5. NPSP regression assert: post-fix rev-N scan produces ≥1 inbound edge
   on `fflib_IDomain::getObjects()` AND on
   `BDI_CustObjMappingGAUAllocation::populateObjects(BDI_ObjectWrapper[])`.

**Acceptance.** Round 5-style draw on the post-R47 rev-N NPSP baseline:
interface-dispatch shape drops from 3/10 (rev 11) to 0/10 for three
consecutive draws at independent seeds.

**Out of scope.** Dynamic-dispatch-by-string (`Type.forName(...)
.newInstance()` + reflection) is R48 / separate; R47 only covers
statically-typed supertype references.

---

## R48. Apex Visualforce page binding → controller getter — declarative-wiring invisibility

**Shape.** Apex Visualforce pages bind page-expressions `{!foo}` to
controller-side getters `getFoo()` (and similarly for setters). The
binding is declarative — it lives in `<apex:page>` / `<apex:outputText
value="{!foo}" />` tags in `.page` files — and the resolver does not
currently parse Visualforce expression syntax. Every Apex `getFoo()`
whose only caller is a VF page binding shows up in the `no_callers`
bucket.

**Evidence — NPSP rev 11, Round 5b hand-audit.** Sample #9,
`PMT_PaymentWizard_CTRL::getTotalWrittenOff()`, is bound in
`force-app/main/default/pages/PMT_PaymentWizard.page:176`:
```
<apex:outputText value="{!totalWrittenOff}"  id="oppAmtWrittenOff" />
```
The engine parsed the `.page` file (it appears in the parse DB) but did
not extract the `{!...}` binding as an edge to the controller getter.

**Why this is the R25 (LWC) analogue for Apex Visualforce.** R25 tracks
LWC template-binding invisibility (`this.foo` in a `.js` controller
referenced by `{foo}` in a `.html` template). R48 is the Visualforce
equivalent for classic Apex: the binding syntax is different (`{!foo}`)
and the target convention is different (`getFoo()` conventional getter),
but the architecture is identical — a declarative wire with no call
expression in either file, so the heuristic resolver sees neither side.

**Recipe.**
1. Add a Visualforce `.page` extractor that tokenises `{!identifier}`
   expressions and emits a synthetic call edge from the `.page` file's
   `__file_scope__` (or a per-expression node) to the convention-based
   getter FQN `<controller>::get<Identifier>()` on each controller
   bound by `<apex:page controller="Foo">` / `extensions="[...]"`.
2. Where the conventional getter does not exist but a plain property
   does, treat the binding as a data-read (no edge) rather than a
   method call — the resolver must not over-invent call edges.
3. Regression fixture: `apex_r48_vf_page_binding_e2e.rs` with a one-
   page fixture and a controller exposing `getFoo()`; assert a Call
   edge with `confidence = Medium` lands on `getFoo()`.
4. Reclassify the resulting dead-code bucket: bindings are
   `declarative_wiring_unparsed` once identified, not `no_callers` —
   tie the new classifier rule into that bucket.

**Acceptance.** Round 5-style draw: VF-binding shape drops from 1/10
(rev 11) to 0/10 on two consecutive seeds.

**Out of scope.** Dynamic Visualforce (component-type binding via
`!dynamicallyBuiltComponentRef`), custom component manifests, and
`apex:actionFunction` rebindings — each distinct shape gets its own
follow-up risk if it appears in audits.

---

## R49. Apex static initializer block body extraction gap — call sites inside `static { ... }` are silently dropped

**Shape.** Apex classes can carry a class-level static initializer
block:
```apex
public class Foo {
    public static Bar Errors { get; private set; }
    static {
        Errors = new Bar();
    }
}
```
The `static { ... }` block runs once on class load. The extractor
currently synthesizes `Function` nodes for trigger bodies (`__trigger__`),
field initializers (R41, `<field>.__init__()`), and property accessors
(R39, `<prop>.__get__()` / `.__set__()`), but NOT for static initializer
blocks. The `call_sites` query still captures `new Bar()` at lexical
level, but the site has no enclosing Function in the graph, so the
heuristic resolver's `find_enclosing_function` returns `None` and the
call is dropped before any dispatch arm runs.

**Evidence — NPSP rev 11, Round 5b hand-audit.** Sample #4,
`fflib_SObjectDomain.ErrorFactory::ErrorFactory()` (zero-arg inner-class
constructor), has two caller sites, both inside static initializer
blocks:
- `force-app/infrastructure/apex-common/main/classes/fflib_SObjects.cls:40` inside `static { Errors = new ErrorFactory(); }`.
- `force-app/infrastructure/apex-common/main/classes/fflib_SObjectDomain.cls:109` inside `static { Errors = new fflib_SObjectDomain.ErrorFactory(); }`.

Neither R41 nor R39 closes this shape because `static { ... }` is a
`static_initializer` (or analogous) tree-sitter node, not a
`field_declaration` or `accessor_declaration`.

**Recipe.** Extend
`graphengine-parsing/src/syntax/language/apex/field_body_synth.rs`
with a fourth synthesis arm:
1. Walk the class body for `static_initializer` nodes (verify the
   exact tree-sitter node kind against
   `graphengine-parsing/vendor/tree-sitter-sfapex/apex/src/node-types.json`).
2. For each, synthesize one `Function` node with FQN
   `<class>::__static_init__()` covering the block's byte range.
3. Properties: `synthetic = true`, `synthetic_kind = "apex_static_initializer"`,
   `parent_class_id = <class_id>`. Tag the node as an entry-point
   because `static { ... }` is implicitly invoked by the Apex runtime
   on class load — without this, `__static_init__` would itself appear
   in the `no_callers` bucket.
4. Mirror for instance initializer blocks (`{ ... }` at class scope,
   without the `static` modifier), as `__instance_init__()` (called
   implicitly at object construction, treated as entry-point).
5. Fixture: `apex_r49_static_initializer_e2e.rs` reproducing the
   `fflib_SObjects` / `fflib_SObjectDomain` shape; assert the inner-
   class constructor resolves with ≥1 inbound Call edge.
6. NPSP regression assert: post-R49 scan produces ≥1 inbound edge on
   `fflib_SObjectDomain.ErrorFactory::ErrorFactory()`.

**Acceptance.** Round 5-style draw: static-initializer-block shape
drops from 1/10 (rev 11) to 0/10 on two consecutive seeds.

**Out of scope.** Trigger-context-specific static blocks and
`@IsTest(SeeAllData = true)` test-only initializer behaviour — those
are classifier-layer concerns once the extractor stops dropping the
call-sites themselves.
