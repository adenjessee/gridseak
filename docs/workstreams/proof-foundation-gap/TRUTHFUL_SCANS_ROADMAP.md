# Truthful-Scans Roadmap — Close the Foundation Gap

**Type:** Plan
**Created:** 2026-04-16
**Owner doc for:** The Phase 0 → D sequence that takes the engine from "classifier-truthful" (end of Wave 2) to "edge-truthful" (end of Phase D).
**Supersedes:** the "Next work" tail of `REGRESSION_RESULTS.md` for rev 5; the free-text paragraphs about "what's next" in `FOLLOWUP_RISKS.md`. Those files stay the source-of-truth for *evidence* and *risks*; this file stays the source-of-truth for *sequence, gates, and acceptance*.

**Companion docs.**

- `docs/workstreams/proof-foundation-gap/FINDINGS.md` — the original pre-registered experiment (why we know the gap exists).
- `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` — R1 – R28, every architectural/integrity risk surfaced so far.
- `docs/workstreams/proof-foundation-gap/REGRESSION_RESULTS.md` — numerical rev-over-rev results.
- `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md` — the Layer-5 manual-verdict log.
- `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` — the Apex-specific deep-dive that Phase B enumerates against.

---

## 1. Why this roadmap exists

At the end of Wave 2 (rev 5) the *classifier* is truthful: every dead-code FQN maps to a real `DeadCodeReason` under the correct framework dispatch, with evidence strings, and Layer-5 audits pass on every bucket **except** `no_callers`. That last exception is not a classifier bug — it is an **edge bug**. The parser is not linking real call edges that exist in the source code. No amount of classifier work can fix a missing edge.

This roadmap is the ordered work that turns "the classifier says the right thing about the graph we have" into "**the graph matches the source code**" — and then locks that contract down with test rhythm, schema evolution, and confidence gates so the engine is provably ready to put in front of customers.

It explicitly sequences every open risk from `FOLLOWUP_RISKS.md` and every parking-lot item noted during Waves 1–3 into either a phase below or an explicit `OUT-OF-SCOPE` row with the trigger that would activate it. Nothing is stranded.

---

## 2. North-star invariants

These are the invariants we will hold true by the end of Phase D. Every phase either preserves them or moves toward them; none may break them.

1. **Every edge carries provenance.** Every `Edge` emitted into the graph carries `(source: EdgeSource, confidence: f32, evidence: String)`. No anonymous edges. `DeadCodeReason` derives from the *absence* of authoritative edges, not from a hand-coded classifier arm.
2. **Per-node framework dispatch.** Every `GraphNode` carries a materialized `language` + `frameworks` list. Classification and resolution dispatch on those fields, never on a project-level `Ecosystem` label. (Wave 2 landed this; Phases A–D preserve it.)
3. **Reason taxonomy is data, not schema.** A new `DeadCodeReason` is a data edit to a registry, not a Rust enum variant that breaks every downstream consumer. (R27, Phase D.)
4. **Gate = audit, not metric.** A phase ships only when a fresh Layer-5 hand-audit on ≥10 sampled FQNs/bucket passes < 2 / 10 wrong on every bucket the phase promised to fix. Numerical deltas are pre-registered; a phase that hits its deltas but fails its audit has *not* shipped.
5. **Reality parity is measurable.** Every rev publishes rev-over-rev deltas (via `experiments/ab_inject/`) into `REGRESSION_RESULTS.md`. Any metric drift outside the pre-registered envelope is either a planned behaviour change (documented) or a regression (blocking).

---

## 3. Phase 0 — Sub-day cleanups (do first; buy audit confidence cheaply)

**Goal.** Retire the two remaining "we know how to fix this in <1 day and it sharpens the next audit" items. Zero customer-facing impact; big signal-to-noise improvement for Phase A's audit.

| Ticket | Scope | Source | Acceptance |
|---|---|---|---|
| TR-0.1 | `looks_like_tdtm_handler` substring scan → class-name / enclosing-type match. Strip parameters before matching; only the class or method name portion of the FQN is allowed to contribute. | R24 | **Shipped — rev 6 (initial) → rev 6.1 (corrected via TR-0.1.1).** Parameter-type leakage (R24) closed with unit coverage. TR-0.1's initial `class_tokens.iter().any(is_tdtm_token)` predicate was too permissive (inner-class / non-`run` over-match on `*_TDTM.cls`, R31). TR-0.1.1 (rev 6.1) restricts the match to outer-class token AND method ∈ {`run`, outer-class zero-arg ctor}, keeping the `run_on_handler` fallback. All 48 NPSP nodes R31 misattributed routed back to `no_callers`. Layer-5 Round 4 re-scoring (rev 6.1, seed `20260418`): `dynamic_dispatch_target` 0 / 10 wrong, `framework_annotation_unresolved` 0 / 10 wrong, `declarative_wiring_unparsed` 0 / 10 wrong. R31 closed. |
| TR-0.2 | `detect_frameworks_by_path` gains an `aura` tag via broad `aura/` path-segment match (sizing per §13 — folder convention IS the contract; broad-match survives non-canonical bundle helpers) and narrow `jest` / `vitest` tags keyed on `<runner>.setup.{js,ts,mjs,cjs}` and `<runner>.config.*` filenames (sizing per §13 — runner identity IS the contract; two tags share `frameworks/test_harness_common.rs` for evidence wording). Three new rule modules (`aura.rs`, `jest.rs`, `vitest.rs`) route Aura symbols to `declarative_wiring_unparsed` and harness symbols to `framework_annotation_unresolved`. | R28 | **Shipped — rev 6.** The 3 surviving NPSP rev-5 `visibility_private_unused` production items (1 aura + 2 jest) route to the correct new bucket. `polyglot_mixed_integration.rs` extended with a canonical Aura controller, a non-canonical Aura helper (`FormUtils.js`, locks down the broad-match contract against future narrow-regression), a `jest.setup.js`, and a `vitest.setup.ts`, plus a distinct-attribution assertion (`js-jest` ≠ `js-vitest`). All 36 parser + 41 classifier unit tests pass, polyglot integration test passes. Layer-5 Round 4 hand-audit scheduled against NPSP rev 6 before Phase A opens. |
| TR-0.3 | Evidence-string polish: every evidence string the classifier emits is either a tight human-readable clause or a pointer to a registry entry (not a stringly-typed reason name). | maintenance | Evidence strings rendered in a Desktop UI prototype table read as English, not as internal identifiers. No schema change. |

**Sequencing.** TR-0.1 and TR-0.2 ran in parallel. TR-0.3 piggybacks on whichever ships second. TR-0.1.1 (R31) landed on rev 6.1 as a Phase-0 revisit.
**Layer-5 Round 4** initial run on rev 6 FAILED `dynamic_dispatch_target` (4 / 10 wrong, R31). Round 4 re-scoring on rev 6.1 (post-TR-0.1.1) PASSED — see `HAND_AUDIT_LOG.md` §"Round 4 re-scoring — Engine revision 6.1".
**Confidence exit criterion.** Met: `dynamic_dispatch_target` 0 / 10 wrong on rev 6.1; `framework_annotation_unresolved` 0 / 10 wrong; `declarative_wiring_unparsed` 0 / 10 wrong; `visibility_private_unused` collapsed to n = 0. **Phase A is open.**

---

## 4. Phase A — Apex AST resolver gaps (R23)

**Status (2026-04-19): OPEN.** rev 9 Round 5 hand-audit scored
**4 / 10 wrong** against the `< 2 / 10 wrong` closure gate. PR 9
closed R46 (cross-language reserved-keyword extractor filter)
but did not address the three shapes the Round 5 wrong verdicts
surfaced: R39 (property-accessor extraction gap — 2 samples),
R41 (field-initializer extraction gap — 1 sample, filed in PR 9),
R45 (chained-call receiver-typing gap — 1 sample, filed in PR 9).
PR 8 shipped TR-A.4's static-receiver path (R40 closure). TR-A.0,
TR-A.1, TR-A.2, TR-A.3, TR-A.5, TR-A.6 have not shipped. See
`HAND_AUDIT_LOG.md` §"Round 5 — Engine revision 9" for the
evidence, `FOLLOWUP_RISKS.md` §R39 / §R41 / §R44 / §R45 / §R46
for the risk state, and
`docs/workstreams/universal-fidelity/ (sprint directory)`
for the intervening work that runs before Phase A reopens for
re-audit.

**Execution contract.** The ticket-level contract for Phase A (PR
inventory, sub-scope detail, extractor-fix blast radius, trigger
scope carve-out, Round-5 draw-pool pre-filter, byte-identical
CI-gate mechanism) lives in
[`PHASE_A_EXECUTION_PLAN.md`](./PHASE_A_EXECUTION_PLAN.md). This
section stays source-of-truth for *sequence* and *acceptance*;
cite the execution plan for *how*.

**Goal.** Close the seven failure classes the Wave 1 Round 2 hand-audit identified in the `no_callers` bucket. This is a *resolver* fix, not a classifier fix. When it ships, `no_callers` drops from inflated to honestly-representative on every Apex repo, not just NPSP.

**Rationale for sequencing before Phase B.** Phase B (FrameworkEntry edges) builds on the class registry. The class registry is incomplete today because the resolver does not consult it for constructor calls or field-typed dispatch. Phase A makes the class registry the authoritative type oracle; Phase B then wires framework edges into it cleanly. Reversing the order means Phase B emits correct framework edges against a graph whose in-repo edges are still wrong, and the `no_callers` audit still fails for unrelated reasons.

| Ticket | Scope | Source | Acceptance |
|---|---|---|---|
| **TR-A.0** | **Apex type-oracle foundation.** Introduce `ApexClassSymbols { fields, methods, constructors, inner_classes, parent_class, implemented_interfaces }` co-located with `ClassEntry` in `graphengine-parsing/src/syntax/language/apex/class_registry.rs`. New module `graphengine-parsing/src/syntax/language/apex/class_symbols.rs` carrying `ApexField`, `ApexMethod`, `ApexConstructor`, `ApexParameter`, `ApexTypeRef` (`Primitive` / `Sobject` / `UserDefined` / `Collection` / `Map` / `Generic` / `Unresolved`), and `CollectionKind`. Extractor populates symbols on the existing class-declaration AST pass (no second pass). New parse-DB table `apex_class_symbols (api_name TEXT COLLATE NOCASE PRIMARY KEY, symbols_json TEXT NOT NULL)`. Case-folded `api_name` throughout — Apex is case-insensitive. Overloads keyed on full `Vec<ApexParameter>` signatures, not `(name, arity)`. New `parse_meta` table with `schema_version INTEGER NOT NULL` (bumped to 2); `ge-analyze` reads it first and emits `CAVEAT_STALE_PARSE_DB_V1` on pre-upgrade artefacts. **Additionally ships a shared dotted-path containment walker as dormant infra** (`containment_walker.rs`) consumed by TR-A.3 / TR-A.4 / TR-A.6 after TR-A.0 lands; zero resolver code path exercises it in TR-A.0, preserving the byte-identical gate. **Out of scope: `.trigger` files** — trigger context variables (`Trigger.new`, `Trigger.old`, etc.) require a separate `TriggerSymbols { sobject_type, events, trigger_body_fqn }` struct owned by Phase B's FrameworkEntry scope. TR-A.0 extractor skips `.trigger` files; `apex_class_symbols` rows are `.cls`-declared classes only. Analyse-side reads `symbols.as_ref()` defensively; the oracle is dormant at the analyse layer until TR-A.1 consumes it. **No resolver changes, no metric deltas, no graph-shape changes expected.** | R23, foundation for A.1–A.6; R32 (parse-DB schema versioning) | Re-parsing NPSP populates ~2,500 rows in `apex_class_symbols`; spot-check 10 classes by hand (5 NPSP-Apex, 5 fflib, at least 1 with ≥3 overloads, at least 1 with ≥2 inner classes) and verify every declared method / constructor / field is present with correct `ApexTypeRef`; `cargo test` green; rev-6.1 A/B run reproduces byte-identical analysis output (CI gate: `sha256sum` + `diff` on normalised JSON — mechanism detailed in `PHASE_A_EXECUTION_PLAN.md` §2.2 A.0.8). |
| TR-A.1 | Intra-class constructor resolution: `new X(...)` where `X` is a sibling class in the same file. Consumes `ApexClassSymbols.inner_classes` + `.constructors` via TR-A.0. | R23, audit sample ≥3 | The 3 NPSP audit FQNs in this class (`new Logger(...)`, `new BatchJob(...)`, etc.) resolve to edges. |
| TR-A.2 | Cross-file constructor resolution via `class_registry`: `new X(...)` where `X` is declared in a different `.cls`. Consumes `ApexClassSymbols.constructors` via TR-A.0; arg-type-aware via `ApexTypeRef`. | R23, audit sample ≥2 | `new HouseholdMembers(...)` @ `HouseholdNamingService.cls:257` resolves. |
| TR-A.3 | Field-type-aware dispatch: parse Apex field declarations and carry the declared type through `instance.method(...)` resolution. Consumes `ApexClassSymbols.fields` + the resolved class's `.methods` via TR-A.0. | R23, audit sample ≥2 | `permissionsService.canUpdate(...)` resolves to the permissions-service class, not `no_callers`. |
| TR-A.4 | Intra-class overload dispatch: `this.foo(x,y)` from one overload resolves to another within the class using Apex's overload rules (exact-match > widening > implicit-conversion). Consumes full `ApexClassSymbols.methods` signatures via TR-A.0. | R23, audit sample ≥1 | `fflib_Comparator.compare(String,String)` resolves from the `compare(Object,Object)` overload. |
| TR-A.5 | `<apex:page extensions="X">` Visualforce binding: every `{!methodName}` in the `.page` file emits an edge to `X.methodName` in Apex. Consumes `ApexClassSymbols.methods` lookups via TR-A.0. | R23, part of R25 scope | UTIL_JobProgress_CTRL + its `.page` fixture linked. |
| **TR-A.6** | **Inner-class containment walking.** *Consumes* the dotted-path containment walker shipped as dormant infra by TR-A.0 (design decision: the walker is type-oracle infrastructure, not inner-class-specific behaviour; co-locating it with TR-A.0 decouples A.3 / A.4 / A.6 so they run genuinely in parallel). TR-A.6 wires the resolver arms that call the walker for `new Outer.Inner(...)` and typed-field `Outer.Inner::method()`. Inner-class method overrides resolve against the outer class's `parent_class` / `implemented_interfaces` chain. Load-bearing for the 48-node §4.11.1 R31-revert population (≈ 40 of 48 are inner-class shapes). | R23, §4.11.1 revert population | Round 4 `no_callers` samples #1 (`HH_HouseholdNamingSettingValidator.Notification::getErrors()`), #2 (`RD2_VisualizeScheduleController.DataTableColumn::getType(...)`), #9 (`UTIL_OrderBy.SortableRecord::SortableRecord(...)`) all link up. ≥ 38 / 48 of the §4.11.1 revert population re-resolves. |

**Explicitly deferred from Phase A.** Implicit `toString()` call synthesis in string-concatenation contexts (idiom 6 of §4.11 in `FRAMEWORK_RESOLVER_PLAN.md`). Its honest emission requires per-edge numeric confidence (confidence = 0.7 per §4.11 resolver-work item 5), which is Phase D / TR-D.1 scope. Promoted to Phase D (TR-D.3) with two NPSP fixtures as pre-registered `sampled_positives`: `fflib_StringBuilder.CommaDelimitedListBuilder::toString()` (from §8.3 Round 3) and `fflib_MatcherDefinitions.Eq::toString()` (from rev-6.1 Round 4 `no_callers` sample #10). Shipping it in Phase A against the current schema would emit heuristic synthesis indistinguishable from AST-linked fact — the exact R26/R31 failure mode the product refuses.

**Regression fixtures.** The 7 Wave 1 Round 2 failed samples + the 10 Wave 2 Round 3 failed samples (17 total, enumerated in `HAND_AUDIT_LOG.md` and promoted into `FRAMEWORK_RESOLVER_PLAN.md` §8.3) become permanent unit fixtures under `graphengine-parsing/tests/fixtures/apex_resolver/r23_*.cls`. **Plus** the rev-6 → rev-6.1 revert population from TR-0.1.1: the 48 NPSP nodes R31 over-classified (≈ 40 inner-class constructors, ≈ 8 outer-class override / typed-field dispatches). That delta is snapshotted to `FRAMEWORK_RESOLVER_PLAN.md` §4.11.1 / §8.3 as part of TR-0.1.1's ship steps so Phase A inherits the full population. Of the 17 Rounds-2+3 fixtures, **16 resolve live post-Phase-A**; the one `toString()`-implicit fixture (`fflib_StringBuilder.CommaDelimitedListBuilder::toString()`) is carved out per the Phase-D deferral above.

**Pre-registered NPSP deltas (rev 7, post-Phase-A on rev-6.1 baseline).**

| Metric | Envelope |
|---|---|
| `no_callers` production count | 1,350 → 700 – 1,150 (−200 to −650 in-repo links recovered) |
| `edge_provenance_counts.AstLinked` | current baseline + ≥ 200 |
| `no_callers` Layer-5 Round 5 hand-audit | must be < 2 / 10 wrong |
| Rounds-2+3 regression fixtures (§8.3) resolving live | **16 of 17** (one `toString()`-implicit carved out to TR-D.3) |
| §4.11.1 revert population resolving live | **≥ 42 of 48** |
| `framework_annotation_unresolved` | unchanged ± 5 (Phase A does not touch framework edges) |
| `dynamic_dispatch_target` | unchanged ± 2 |
| `apex_class_symbols` rows present (parse DB) | ~2,500 (one per NPSP Apex class) |
| `parse_meta.schema_version` | 2 |

**Confidence exit criterion.** `no_callers` audit passes < 2 / 10 wrong on a re-sampled draw (not the same 10 FQNs used as fixtures). This is the first audit where the bucket has passed since Wave 1. Round 5's `random.seed(...)` is stamped at draw time (not pre-declared) per the seed convention in `HAND_AUDIT_LOG.md`; the draw-pool pre-filter excludes `toString()`-named FQNs so the §4.11.2 carve-out does not pollute the gate — protocol in `PHASE_A_EXECUTION_PLAN.md` §12.

**Effort estimate.** TR-A.0: 2–3 d. TR-A.1 + TR-A.2: 1–2 d combined (both consume the oracle). TR-A.3: 1–2 d. TR-A.4: 1–1.5 d. TR-A.5: 1 d. TR-A.6: 1–1.5 d. Total Phase A: **7–10 days**. The jump from the original 4–7-day estimate reflects TR-A.0 foundation work + TR-A.6 inner-class ticket addition; per-ticket cognitive load drops because every A.1–A.6 consumes a known-good oracle rather than each building its own type-resolution arm.

**Out-of-scope for Phase A.** Framework-entry edges (Phase B). LWC template parsing (Phase C). Declarative wiring (Phase C). Implicit `toString()` synthesis (Phase D TR-D.3 — see deferral note above). Generic type-argument substitution (`List<T>.add(T)` stays resolved as-declared, no substitution). Full Apex type inference (trust declared types only).

---

## 5. Phase B — FrameworkEntry edge emission (Apex Framework Resolver)

**Goal.** Implement the full backlog in `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`. Every Apex framework-invoked method (Batchable/Schedulable/Queueable/trigger/AuraEnabled/Invocable/RestResource/…) gets a `FrameworkEntry(tag)` synthetic edge from a synthetic source node representing the Salesforce platform, with evidence string and confidence. `framework_annotation_unresolved` and `dynamic_dispatch_target` go to zero.

**Authoritative plan.** `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` (the 22-row dispatch matrix, the clustered backlog, the pre-registered deltas, the Layer-5 Round 4 test protocol). This file does not re-document any of that — it only fixes this phase's position in the overall sequence and confidence gate.

**Dependency on Phase A.** Phase A ships the authoritative class registry. Phase B's synthetic edges reference classes by those registry entries; without Phase A, the framework-entry → class resolution still has the constructor/field-dispatch gaps and the downstream `no_callers` audit cannot isolate "framework gap" from "in-repo gap" failures.

**Pre-registered NPSP deltas (rev 7, cumulative with Phase A).**

| Metric | Envelope |
|---|---|
| `framework_annotation_unresolved` | 190 → 0 (± 5 for unknown future idioms) |
| `dynamic_dispatch_target` | 47 → 0 |
| `edge_provenance_counts.FrameworkEntry` | 0 → ≥ 300 |
| `no_callers` | −20 to −60 incremental over Phase A (Batchable callees recovered) |
| `health_score` (project-level) | +4 to +9 |

**Confidence exit criterion.** Layer-5 Round 6 audit: every bucket Phase B claimed to zero *is* zero, and the remaining `no_callers` audit passes on a re-sample. The Round-5 (Phase A) and Round-6 (Phase B) audits together are the first evidence that the Apex track holds up end-to-end.

---

## 6. Phase C — Declarative-wiring resolvers (R25, R28's LWC/Aura families, VF outside §5)

**Goal.** Parse the declarative side — LWC `.html` templates, Aura `.cmp` files, Visualforce `.page` files (beyond the §5 extensions-attribute subset), Salesforce Flows, Platform-Event handler metadata, Apex install/uninstall handlers — and emit `DeclarativeWiring(kind)` edges for every binding that today routes to `declarative_wiring_unparsed`.

| Ticket | Scope | Source |
|---|---|---|
| TR-C.1 | LWC template parser: `<tag on<event>={method}>`, `{getter}`, `{method(…)}` → `Call` edges to module methods. | R25 |
| TR-C.2 | Aura `.cmp` + `.app` parser: `<aura:registerEvent>`, `<aura:handler action="{!c.method}">`, and `action="{!c.method}"` in any child element. | R28 part 2 |
| TR-C.3 | Full Visualforce: `<apex:page>` inline expressions, `rerender` targets, `action=` attributes beyond the already-resolved `extensions=` subset. | R23/R25 overlap |
| TR-C.4 | Flow / Process Builder XML: `actionCalls` → `InvocableMethod` handlers; `faultConnector` → handlers; `subflows` → other flows. | R13 row 22 |
| TR-C.5 | Platform-Event handlers: `<messaging:emailTemplate>`, `@PlatformEventSubscriber`, `Trigger(before/after insert)` of `__e` objects. | R13 row 14 |
| TR-C.6 | Install/uninstall handler interfaces (`InstallHandler`, `UninstallHandler`) get `FrameworkEntry` edges. | R13 rows 17-18 (small; can piggyback in Phase B if convenient) |

**Pre-registered NPSP deltas (rev 8).**

| Metric | Envelope |
|---|---|
| `declarative_wiring_unparsed` | 137 → < 20 |
| `no_callers` | −20 to −80 incremental (Aura + VF callees recovered) |
| `edge_provenance_counts.DeclarativeWiring` | 0 → ≥ 200 |

**Confidence exit criterion.** Layer-5 Round 7 audit: `declarative_wiring_unparsed` < 2 / 10 wrong; LWC-heavy toy fixture (`dreamhouse-lwc`) shows zero false-positive unused handlers.

**Scope boundary.** Salesforce Flows are XML; we parse them as data, not as a language. No tree-sitter grammar is built for `.flow-meta.xml`. Same for `.cmp`. Same for `.page`. This is an explicit choice: declarative files are authored-as-data, and data-level parsing is sufficient for binding extraction.

---

## 7. Phase D — Edge-provenance consolidation (R26 + R27)

**Goal.** Land the north-star architecture from §2 as code: `EdgeSource` enum, per-edge confidence + evidence, declarative classifier rule registry, open `DeadCodeReason` behind a schema-version bump.

| Ticket | Scope | Source |
|---|---|---|
| TR-D.1 | `EdgeSource` enum (`AstLinked` / `FrameworkEntry(tag)` / `DeclarativeWiring(kind)` / `HeuristicName`) with required `confidence: f32` + `evidence: String`. Every edge emitter in parsing + analysis updated to populate all three. | R26 dependency |
| TR-D.2 | `DeadCodeReason` becomes a derived label: the classifier reads `edge_provenance_counts[node]` + confidence thresholds and derives the reason. The closed enum is retired in favour of a `reason_id: String` + `reasons.toml` registry. Schema-version bump with a `CAVEAT_DEAD_CODE_REASON_OPEN_V1` stamp; downstream consumers treat unrecognised reasons as `Unclassified`. | R27 |
| TR-D.3 | Classifier rule engine: the remaining `if` chains in `dead_code_classifier/frameworks/*.rs` become declarative rules (`condition`, `reason_id`, `evidence_template`, `priority`, `sampled_positives: Vec<NodeRef>`, `sampled_negatives: Vec<NodeRef>`). Rules live alongside the framework detector data (not in Rust). **Binding acceptance criterion:** a Layer-2 invariant check refuses to promote a rule whose `sampled_negatives` set is empty against the NPSP canary — the structural mitigation for the R31 failure mode (see R26 "Concrete worked example" + the supporting-evidence paragraph below). A rule engine that ships without this invariant has not closed R26's underlying risk, regardless of how elegant the rule-row shape is. | R26 |
| TR-D.4 | `report_schema_version` stamped on every `HealthReport`; Desktop UI caveat dispatcher updated to honour it. | R27 dependency on Desktop |

**Pre-registered NPSP deltas (rev 9).**

No metric-level change should occur from Phase D alone — this is a refactor of the *shape* of the data. Any unintended delta is a regression. Layer-5 Round 8 audit on the same sample set as Round 7 must produce identical verdicts.

**Supporting evidence for TR-D.3 priority.** R31 (TR-0.1 regression on rev 6) is a live example of a heuristic that passed Layer 1 – 3 gates (unit tests, polyglot integration, clippy) and still regressed on the Layer 4 population canary because its fixture set did not cover the decomposition cross-product (inner class × class-token prefix). Declarative rule rows — each carrying its sampled positive and negative population alongside the predicate, with a Layer-2 invariant that refuses a rule whose canary coverage is empty — are the structural fix. Do not defer TR-D.3 on the argument "rules are just data."

**Confidence exit criterion.** Schema-version bump propagates cleanly to Desktop + API. Layer-5 Round 8 verdicts match Round 7. Adding a *new* Salesforce dispatch idiom (test: suppose hypothetical `@SubscribeToPlatformEvent`) is a data-only PR — no Rust `match` arms touched.

---

## 8. Test rhythm (Layer 1 – 5 every revision)

This is not a one-time setup; it is the ongoing contract between phases and shipping.

| Layer | Purpose | Who runs it | When it gates |
|---|---|---|---|
| 1. Unit | Per-function correctness in `frameworks/*.rs`, `heuristic_resolver.rs`, `frameworks.rs`. | `cargo test -p graphengine-parsing`/`-p graphengine-analysis`. | CI on every push. |
| 2. Invariant | Per-graph invariants: "every `FrameworkEntry` edge has `confidence ∈ [0,1]` and non-empty evidence"; "no `DeadCodeReason::FrameworkAnnotationUnresolved` on a node that also has an incoming `FrameworkEntry` edge". | `cargo test --features invariant`. | CI on every push. |
| 3. Integration | Polyglot fixtures (`tests/fixtures/polyglot_mixed/`) extended for every new framework. | `cargo test -p graphengine-analysis --test polyglot_mixed_integration`. | CI on every push. |
| 4. NPSP canary | Full parse + analyze of the NPSP clone under `experiments/results/NPSP/`, A/B-inject comparison, metric deltas asserted against `REGRESSION_RESULTS.md` pre-registered envelopes. | Nightly GitHub Action + manual before-merge of any parsing-crate PR. | Blocking before a rev N+1 is declared. |
| 5. Hand-audit | 10-FQN manual sampling per bucket, verdicts recorded in `HAND_AUDIT_LOG.md`. < 2/10 wrong required per bucket. | Human, before any phase is declared shipped. | Blocking before phase declared shipped. |

**New requirement.** Layer 2 "edge-provenance invariants" becomes a CI gate the moment Phase D's `EdgeSource` lands. Today it is not yet enforceable.

---

## 9. Confidence gates — when is it safe to put in front of customers?

These are the **cumulative** bars the engine must clear before the Apex track is ready for paying-customer scans. "Ready" here means a customer scanning a Salesforce repo today sees metrics that, when audited by the customer themselves, do not disclose a known-wrong bucket.

| Bar | Cleared when | Scope of "ready" that clears |
|---|---|---|
| **B1** — Truthful classifier | End of Wave 2 (today). | Internal benchmarking. Not customer-ready; `no_callers` still inflated. |
| **B2** — Truthful in-repo edges | End of Phase A + Round 5 audit. | Alpha customers who accept caveats; `framework_annotation_unresolved` still inflated. |
| **B3** — Truthful framework edges | End of Phase B + Round 6 audit. | Paid Apex pilots. `no_callers`, `framework_annotation_unresolved`, `dynamic_dispatch_target` all Layer-5-audited. |
| **B4** — Truthful declarative edges | End of Phase C + Round 7 audit. | General-availability Salesforce diagnostic. |
| **B5** — Data-as-data rule engine | End of Phase D + Round 8 audit. | Self-service framework rule contributions; schema stability promise. |

Every bar is additive. B2 does not retire the need for B1's audits; B3 does not retire B2's; etc.

---

## 10. Horizon view — everything in one table

This is the single place that lists every open engine-truthfulness item across every horizon, classified. If an item is not in this table and not in `FUTURE_PLAN.md` §Priority 1 – 5 and not in `archive/`, it is stranded and this table must be corrected.

### Now (Phase 0 — complete)

| Item | Owner risk | Status |
|---|---|---|
| TR-0.1 TDTM substring fix | R24 | Shipped rev 6. |
| TR-0.1.1 TDTM inner-class / non-`run` over-match fix | R31 | Shipped rev 6.1. R31 closed by Round 4 re-scoring (0 / 10 wrong). |
| TR-0.2 Aura + Jest + Vitest detectors | R28 | Shipped rev 6. `visibility_private_unused` bucket collapsed to 0 on NPSP. |
| TR-0.3 Evidence-string polish | — | Open; low priority; piggyback on any Phase-A / B PR. |

### Next (Phase A — OPEN, closure gate failed rev 9 Round 5)

| Item | Owner risk | Gated by |
|---|---|---|
| TR-A.0 Apex type-oracle foundation + TR-A.1 – TR-A.6 Apex AST resolver fixes | R23 (+ R32 closed by TR-A.0) | **Status: OPEN.** Phase A closure gate is the rev 9 Round 5 hand-audit on `no_callers` (< 2 / 10 wrong). rev 9 Round 5 scored **4 / 10 wrong** — see `HAND_AUDIT_LOG.md` §"Round 5 — Engine revision 9". Four wrong verdicts decompose as: 2 × R39 (property-accessor extraction gap), 1 × R41 (field-initializer extraction gap — NEW, filed in PR 9), 1 × R45 (chained-call receiver-typing gap — NEW, filed in PR 9). The R46 extractor-layer fix in PR 9 (`name_validator.rs` per-language keyword lists) closed a silent class of missing symbols but did not address R39/R41/R45, which are distinct gaps. TR-A.0..A.6 have NOT shipped; PR 8 shipped only TR-A.4's `TypeName.staticMethod()` receiver path (R40 closure). Re-attempt of the gate is gated on either (a) extractor fixes for R39 / R41 in a dedicated extractor-scope PR family, or (b) universal-fidelity sprint T8 (extraction-coverage-aware classifier downgrade) reducing false-positive rate by honest construction rather than resolver work, or (c) full TR-A.x ship in Phase B. See `FOLLOWUP_RISKS.md` §R41 / §R44 / §R45 / §R46 for the full risk state, and `docs/workstreams/universal-fidelity/ (sprint directory)` for the intervening sprint. |

### Next+ (Phase B — deliberately deferred; re-evaluate after universal-fidelity sprint Phase 4)

| Item | Owner risk | Gated by |
|---|---|---|
| Full Apex Framework Resolver backlog | R9, R13, R23 dependency | **Deliberately deferred.** Phase A closure gate has not passed; the universal-fidelity sprint (`docs/workstreams/universal-fidelity/ (sprint directory)`) runs in the interval and includes a Phase 4 decision point that re-evaluates Phase B scope against a proven Rust Layer 2 adapter (T6), namespaced framework edges (T1), dual-metric emission (T3), and measured fidelity tiers (T4). Phase B may open narrower (declarative-wiring-focused only), may open at the original 2–3 week scope with deliberate Apex-depth commitment, or may stay paused for a second Layer 2 adapter. No work happens here until that decision lands. Owner doc: `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`. |

### Short-term (Phase C — 2 to 4 weeks)

| Item | Owner risk | Gated by |
|---|---|---|
| LWC / Aura / VF / Flow / PE declarative parsers | R25, R28 part 1 | Phase B audit passes. |

### Medium-term (Phase D — 1 to 2 weeks after Phase C)

| Item | Owner risk | Gated by |
|---|---|---|
| Edge-provenance + declarative rule engine + open reason taxonomy | R26, R27 | Phase C audit passes. Desktop UI on current caveat schema. |

### Deferred (plans exist, not now)

| Item | Parking location | Activation trigger |
|---|---|---|
| Kotlin / additional languages | `FUTURE_PLAN.md` §Priority 4 | Customer request. |
| GridLink native resolution | `docs/deferred/GRIDLINK_NATIVE_RESOLUTION.md` | Customer performance bottleneck. |
| UE 3D visualization | `docs/archive/ue-visualization-2026-05/` | $15K/mo sustained + explicit client demand. |
| Coverage / SARIF import | `FUTURE_PLAN.md` §Priority 4 | Enterprise demand. |
| Incremental re-parse | `FUTURE_PLAN.md` §Priority 4 | MCP real-time queries matter. |
| R3 — `resolution_quality` first-class UI | `FOLLOWUP_RISKS.md` §R3, `SPRINT_PLAN.md` WS-PROOF-R3 | WS-APEX-D `ResolutionDegraded` lands. |

### Resolved (do not re-open without new evidence)

| Item | Resolution recorded in |
|---|---|
| R1 / R2 / R4 confidence + caveat contract | `FOLLOWUP_RISKS.md`; `SPRINT_PLAN.md` WS-PROOF-R1, R2, R4. |
| R5 – R8 invariants + benchmarking | `FOLLOWUP_RISKS.md` §R5 – R8. |
| R9 Dead-code aggregate taxonomy | `FOLLOWUP_RISKS.md` §R9 (reason_breakdown shipped). |
| R10 – R12 classifier provenance | `FOLLOWUP_RISKS.md` §R10 – R12. |
| R13 NPSP dispatch matrix | Promoted to `FRAMEWORK_RESOLVER_PLAN.md` §3. |
| R14 – R16 `DeadCodeResult` scope + norms | `FOLLOWUP_RISKS.md` §R14 – R16 (typed scope contract, population loader fix). |
| R17 Pre-classifier caveat | `FOLLOWUP_RISKS.md` §R17. |
| R18 – R22 Wave-1 parser bugs | `FOLLOWUP_RISKS.md` §R18 – R22 (all fixed in rev 4). |

---

## 11. Cross-links to every affected doc

| Doc | Role after this roadmap merges |
|---|---|
| `docs/00-strategy/FUTURE_PLAN.md` | Priority-0 table references this roadmap for Apex-track status; Priority-1 – 5 unchanged. |
| `docs/02-strategy/SPRINT_PLAN.md` | New `WS-TRUTH` section indexes tickets TR-0.* → TR-D.*. |
| `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` | Each of R23, R25, R26, R27, R28 carries "Phase N — see TRUTHFUL_SCANS_ROADMAP.md" in its Recommendation section. |
| `docs/workstreams/proof-foundation-gap/REGRESSION_RESULTS.md` | "Next work" is the pre-registered deltas table in §4, §5, §6 above; local "Next work" section condenses to a pointer. |
| `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md` | Round 4 onward recorded against the phase gates in §3 – §7. |
| `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` | This file's Phase B *is* that plan's execution. No divergence; the resolver plan is the Apex-specific deep-dive, this file is the sequence across languages. |
| `docs/workstreams/apex/NEXT_STEPS_PLAN.md` | "Workstream B" section points to Phase B here. |

---

## 12. Non-goals of this roadmap (explicit)

The following items are intentionally *not* in Phase 0 – D. Each has a home; none is stranded.

- Non-Apex language resolvers (Python Django dispatch, Node Express routing, Rust proc-macro expansion). Tracked in `FUTURE_PLAN.md` §Priority 4. Activated by customer demand for the relevant language.
- Report trend / baseline feature work (T3 in `SPRINT_PLAN.md`). Separate track (`WS-DIAG`). Phase D's schema-version stamp must be compatible with it; coordination is a pre-merge check, not a phase.
- Desktop UI work. Tracked in `WS-DESKTOP`. Phase D's `CAVEAT_DEAD_CODE_REASON_OPEN_V1` stamp requires a Desktop-side handler; that handler's ticket is `WS-DESKTOP-F.*` territory, not this roadmap.
- Full LSP integration (`docs/04-architecture/LSP_INTEGRATION_PLAN.md`). Phase A's Apex AST resolver fixes *reduce* the need for LSP on Apex specifically. LSP is still the long-term semantic-accuracy answer for languages we do not build a framework resolver for.

---

## 13. Framework-tag sizing rule

Every time we add a new entry to `graphengine-parsing/src/domain/frameworks.rs::tag`, we make a decision that ripples forward into every downstream classifier module, edge emitter, evidence string, and — once Phase D lands — declarative rule-registry row. The decision is deceptively small; the consequences are not. This section fixes the rule so the same decision is made the same way every time.

### Principle

> **Tag broadly when the folder or file convention is the contract** (path expresses uniform authorial intent).
> **Tag narrowly when the runner or framework identity is the contract** (runtime behaviour diverges across superficially-similar tags).

Under-tagging (too narrow) causes silent miscategorization: files that *are* part of the framework's contract fall through to the generic classifier and earn `visibility_private_unused` — exactly the R28 bug shape. Over-unifying (too broad, collapsing distinct runners into one tag) destroys attribution honesty: the `reason_breakdown` histogram stops being able to tell customers which runner accounts for which share of their dead-code signal, and future runner-specific rules require conditional escape hatches inside a merged module — the anti-pattern Wave 2 eliminated when we replaced ecosystem-keyed dispatch.

### Worked examples

| Framework | Contract shape | Tag sizing | Detector shape |
|---|---|---|---|
| **LWC** | `lwc/<Component>/` bundle — folder IS the contract (markup + JS + CSS + metadata authored as one unit) | **Broad.** Any file whose repo-relative path has an `lwc` segment. | `rel.split('/').any(\|s\| s == "lwc")` |
| **Aura** | `aura/<Component>/` bundle — same shape as LWC; canonical filenames (`<Name>Controller.js`, `<Name>Helper.js`, `<Name>Renderer.js`) coexist with non-canonical helpers (`FormUtils.js`) imported from the canonical files | **Broad.** Mirror LWC. A narrow rule keyed only on the canonical filenames would miss the non-canonical helpers and re-create R28 in a new shape. | `rel.split('/').any(\|s\| s == "aura")` |
| **TDTM** | A specific Apex class convention — `TDTM_<X>` / `<X>_TDTM` / `TDTM` class *names* register with the NPSP trigger runner via reflection | **Narrow, structural.** Path-agnostic; class-segment token match (not filename substring). See TR-0.1 / R24 for the parameter-type false-positive this narrowness prevents. | `class_tokens.any(is_tdtm_token)` |
| **Triggers (`triggerdml`)** | The `.trigger` extension IS the dispatch contract (platform DML pipeline) | **Broad.** Any file with a `.trigger` extension. | `file_name.ends_with(".trigger")` |
| **REST resource / AuraEnabled / @task** | An *annotation* on the symbol is the contract; the file can be anywhere | **Narrow, symbol-level.** The framework tag lives on the `entry_points` list on the symbol, promoted to the file only when a per-file routing rule (like REST) needs a file-scope dispatch key. | `augment_frameworks_from_symbol_tags` |
| **Jest** | `jest.setup.*` / `jest.config.*` files — the runner IS the contract | **Narrow.** Filename prefix + extension allowlist. | `stem == "jest.setup" \|\| stem == "jest.config"` |
| **Vitest** | Same setup contract as Jest today, divergent semantics (module mocks, snapshots) | **Narrow, distinct tag from Jest.** Two tags + shared helper keep `reason_breakdown` honest today and give future runner-specific rules a clean home. A single merged `js-test-harness` tag would smuggle runner identity back in as a sub-discriminator — the pattern we removed when we replaced `Ecosystem`-keyed dispatch in Wave 2. | `stem == "vitest.setup" \|\| stem == "vitest.config"` |

### Rule of thumb

- If a team renames one file inside the folder and it should still be part of the framework → tag the **folder segment**.
- If the tag differentiates *which runner / which framework* emitted the symbol (matters for attribution, matters for future divergence) → give it **its own tag**, even if rule logic is shared today via a helper module (e.g. `frameworks/test_harness_common.rs`).
- If the tag is emitted per-symbol by an annotation extractor → keep it on `entry_points`, not as a file-scope framework, unless file-scope dispatch is required.

### Process requirement

Every PR that adds a new framework tag must state its sizing verdict in the PR description, citing this section. New tags without a stated verdict are blocked in review. When `.cmp` / `.page` / `.html` / Flow XML parsers land in Phase C (six or more new tags arriving in a single workstream), each one gets the same discipline.

### Shared-helper consolidation is an ongoing obligation

The "shared helper module" half of the principle is not free: once
two tags share logic, *every* other tag in the same structural
family must reach the same helper, or the helper becomes a fourth
copy instead of a single source of truth. TR-0.2 shipped
`frameworks/test_harness_common.rs` with the shared `parent_path`
helper and consumed it from `jest.rs` and `vitest.rs` only. The
pre-existing `django.rs` and `celery.rs` modules each carry a
verbatim copy of the same helper and have not yet been migrated.
The outstanding migration is tracked as **R30** in
`FOLLOWUP_RISKS.md` — refactor-only, non-blocking, but required
before Phase D's declarative rule engine (R26) is allowed to
touch these modules so the later refactor does not inherit three
ways to spell the same operation.

See R28 in `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` for the original forcing case that surfaced this rule.

---

**Updates rule.** When a phase ships, update this file in the same commit: mark the phase row "Shipped — rev N", record the Layer-5 round number and verdicts, and move any unexpectedly-deferred item to either the next phase or the deferred table with a dated rationale.
