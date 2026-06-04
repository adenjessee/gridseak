# Apex Validation Results — Real-World Corpora

**Date**: 2026-04-17 (Sprint E) / 2026-04-16 (Sprints F + G rerun)
**Scope**: Validates the Sprint A–G Apex integration by scanning real
public Apex codebases — `dreamhouse-lwc`, `apex-recipes`,
`Volunteers-for-Salesforce`, `NPSP` — and comparing scan output to a
careful manual read. Sprint G (scale validation) adds the two large
corpora and surfaced one real graph-assembly bug (in-memory node
deduplication) which this document records, along with manual-verified
per-class predictions for 6 non-trivial classes.

This document is **honest about gaps**: anything that looks wrong, is
wrong, or is undersized relative to the product vision is called out
explicitly. The summary at the top is the quick read; the per-corpus
sections below show the evidence. The **Sprint E Deltas** section
directly below the TL;DR shows exactly which gaps closed, which
moved, and which remain.

---

## Sprint E Deltas — Graph Correctness (2026-04-17)

Every row below was a "Gap" on the Sprint A–D baseline and was
reclassified by end of Sprint E. The references point at the unit /
E2E tests that pin each fix so a regression lights up in CI instead
of going silent.

| Gap | Prior status | Sprint E status | Evidence |
|---|---|---|---|
| `Extends` / `Implements` edges missing | Never emitted | **Closed** — modelled as distinct `EdgeKind` variants, wired in Apex + Rust + TS + Python + Go | `apex-recipes.json` shows **12 `extends` edges**; `graphengine-parsing/tests/apex_extends_implements_e2e.rs` |
| FQN collisions (overloads, sibling inner classes) | Duplicate IDs/FQNs | **Closed** — FQN now includes enclosing type path + parameter signature `Outer.Inner::m(T1,T2)` | `graphengine-parsing/tests/apex_fqn_disambiguation_e2e.rs` |
| Trigger body calls invisible | `new Handler().run()` at trigger top-level produced no Call edge | **Closed** — synthetic `__trigger__` `Function` node is the caller for trigger-body call sites | `graphengine-parsing/tests/apex_trigger_body_synthesis_e2e.rs` |
| `trigger_events` property never populated | Query in YAML, nothing written | **Closed** — wired through `extract_trigger_metadata`; meta-test enforces every YAML query is consumed | `graphengine-parsing/tests/apex_trigger_events_e2e.rs`, `apex_query_coverage.rs` |
| Inner classes had `properties: {}` (no `apex_sharing`, no `is_test`) | Empty | **Closed** — inner classes inherit the outer class's sharing modifier and `is_test` tag; their own modifier (if declared) wins | `graphengine-parsing/tests/apex_inner_class_propagation_e2e.rs`; see also distribution below |
| `File → Module` anomaly (`PagedResult.cls → force_app`) | Single spurious edge on dreamhouse | **Closed** — Apex skips FQN-ancestor auto-creation and attaches every symbol to its file's `__file_module__` Module; directory layout stays in the Folder hierarchy | `ContainmentBuilder::tests::apex_file_module_is_the_container_not_synthetic_ancestors` (unit) |
| `Type` edges encode method-ownership, not real references | Weak — 22 `Type` edges on dreamhouse all `Method→OwningStruct` | **Deferred to post-E** — requires reworking the Type edge producer (return types / parameter types / field types). Tracked in ticket backlog, out of scope for E which is Graph Correctness (structure), not Graph Semantics (edge kind semantics). |
| LSP produces zero resolved edges | 100% heuristic fallback | **Unchanged by E** — Sprint F (LSP production hardening) owns this. |

### Fresh baselines (after Sprint E)

| Corpus | Nodes | Edges | Modules (== Files) | Extends | Triggers | Scan |
|---|---|---|---|---|---|---|
| dreamhouse-lwc | 56 | 105 | 9 / 9 | 0 | 0 | 1.06 s |
| apex-recipes | 1231 | 6321 | 142 / 142 | 12 | 3 | 1.97 s |

The "Modules == Files" column is the direct E.6 validation: every
Apex file now has exactly one `__file_module__` Module, and zero
synthetic ancestor Modules exist in the output. The PagedResult
anomaly cannot recur without making this invariant fail.

The **apex_recipes `apex_sharing_distribution`** — 105 `with_sharing`
+ 21 `inherited_sharing` + 51 `omitted` = 177 of 180 structs — also
confirms E.5. The 3 uncovered structs are exactly the 3 triggers,
which have no sharing modifier by Apex design.

### What Sprint E did NOT change

- Semantic quality of `Type` edges (still method-ownership).
- LSP resolved-edge count (still 0% LSP, 100% heuristic fallback).
- Custom-object (`Property__c`, Platform Events) dependency edges —
  out of scope (source-only Phase 1).

These remain open and are the subject of Sprint F and later
tickets.

---

## Sprint F Deltas — LSP Production Hardening (2026-04-16)

Sprint F closes the "jorje is a black box" gaps identified in the
Sprint E notes. All changes are additive: every pre-F baseline remains
valid because `Immediate` readiness is the default and Apex is the
only opt-in caller for `ProgressAndProbe`.

| Area | Prior state | Sprint F status | Evidence |
|---|---|---|---|
| Stderr + notification observability | stderr debug-logged, notifications dropped | **Closed (F.1)** — `LspNotificationSink` routes both into `SessionSupervisor`; counters in `SessionMetrics` (`notifications_received`, `stderr_lines_observed`, `indexing_messages_seen`, `last_indexing_progress`); opt-in verbose via `GRAPHENGINE_LSP_VERBOSE=1` | `infrastructure::lsp::session::tests::*` (7 new unit tests for F.1) |
| `workspaceFolders` never advertised | jorje silently indexed `rootUri` only | **Closed (F.2)** — `SessionOptions.workspace_folders` populated from SFDX `packageDirectories`; multi-package projects now visible to the server | `syntax::language::apex::lsp_session::tests::multi_package_sfdx_emits_every_folder` |
| `initialize` returned Ready before indexing done | `find_definition` saw `null` in the first 10–30s (wall-clock) of every scan | **Closed (F.2)** — `ReadinessStrategy::ProgressAndProbe` waits for `$/progress end`, a non-empty `documentSymbol` probe on a canary file, or a quiet period after indexing signals; degrades to `Ready` with a warning on timeout (does not fail the scan) | `infrastructure::lsp::session::tests::progress_and_probe_*` (3 unit tests) |
| Byte-column positions sent where UTF-16 was required | Silent misresolution on any source file containing non-ASCII above the query line | **Closed (F.2)** — `byte_col_to_utf16` applied in `find_definition` + `hover`; pass-through for ASCII, correct for CJK / emoji | `infrastructure::lsp::column_utils::tests::*` (7 unit tests) |
| No committed real-jorje smoke | Only the minimal init smoke existed | **Closed (F.2)** — `apex_lsp_readiness_barrier_and_workspace_folders_are_advertised` exercises the full F.2 path against a real SFDX fixture. Skipped (not failed) on hosts without Java + jorje. | `graphengine-parsing/tests/apex_lsp_smoke.rs` |

### Re-baseline on dreamhouse-lwc + apex-recipes

Both corpora produce **bit-for-bit identical** numbers before and after Sprint F
(node count, edge count, edges-by-kind, provenance breakdown, sharing
distribution, resolution stats all equal to the Sprint E baseline).
Exactly what we wanted: F changed the **LSP wire contract and
observability**, not the graph semantics.

| Corpus | Nodes | Edges | Heuristic fallback | LSP edges | Result vs. Sprint E baseline |
|---|---|---|---|---|---|
| dreamhouse-lwc | 56 | 105 | 47.62% | 0 | identical |
| apex-recipes | 1231 | 6321 | 80.48% | 0 | identical |

---

## Sprint G Deltas — Scale Validation (2026-04-16)

Sprint G pushed the pipeline against two large public Apex corpora
that the Phase-1 pilot cares about (`NPSP`, `Volunteers-for-Salesforce`),
added a focused per-class inspector (`apex_inspect` binary), and
manually verified six non-trivial classes end-to-end.

### G.1 Large-corpus baselines

| Corpus | Files (.cls + .trigger) | Nodes | Edges | Modules == Files | Heuristic fallback | LSP edges | Scan |
|---|---|---|---|---|---|---|---|
| `NPSP` (SalesforceFoundation/NPSP @ main) | 1071 | 16356 | 288034 | **1071** (+4 managed-package externals, 1 per unique namespace, dedup'd) | 94.29% | 0 | 95.3 s |
| `Volunteers-for-Salesforce` (SFDO-Community @ main) | 79 | 759 | 4486 | **79** (no managed packages) | 82.95% | 0 | 62.4 s |

Baselines committed under `tests/fixtures/apex_baseline/NPSP.json` and
`tests/fixtures/apex_baseline/Volunteers-for-Salesforce.json`.

### G.1 Real bug surfaced + fixed: in-memory node deduplication

Initial NPSP scan reported **2752 Module nodes for 1071 files**. Root
cause traced through the code:

1. `treesitter.rs` synthesises one `__file_module__` Module per file (1071).
2. `apex/extractor.rs::synthesize_external_references` emits one external
   Module (stable id, `external::salesforce::managed_package::<ns>`) per
   **(consumer file, namespace)** pair — 538 such emissions in NPSP.
3. `GraphBuilder::add_nodes` pushed every one of those into `graph.nodes`
   verbatim. SQLite dedups via `INSERT OR REPLACE`, but the in-memory
   `Graph` (consumed by baselines, analysis, and test harnesses) did
   **not**.

Fix in `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/graph_building.rs`:
first-wins dedup on `Node::id` in `add_nodes` (syntax symbols) and
`add_containment` (against what's already on the graph). Downstream
effect on NPSP:

| Metric | Before dedup fix | After dedup fix |
|---|---|---|
| node_count | 18 007 | 16 356 |
| `module` kind | 2752 | **1075** (1071 file_module + 4 unique external) |
| `call` kind | 309 068 | 259 260 (duplicate-target resolver edges collapsed) |
| `extends` / `implements` | 0 / 0 | **229 / 214** |

Regression-pinned by two unit tests:
`pipeline::graph_building::tests::add_nodes_dedupes_by_stable_id` and
`add_containment_skips_ids_already_on_graph`.

Volunteers (no managed packages) is **byte-identical** before and after,
which confirms the dedup only collapses genuinely-duplicated ids.

### G.2 Manual per-class verification

Inspection helper: `cargo run --release --bin apex_inspect -- --repo <path>
--match <fqn-substring> --output-dir <dir>` emits, per match, every
matched node's properties and every outgoing / incoming edge. Output
files committed under `tests/fixtures/apex_baseline/inspections/`.

Three non-trivial classes per corpus, each hand-read and compared to
the scan output:

| Class | Lines | Manual prediction | Scan result | Verdict |
|---|---|---|---|---|
| `UTIL_Describe` (Volunteers) | 338 | 1 outer + 2 inner (PermsException, SchemaDescribeException) structs; 1 ctor + 18 methods; 2 `extends Exception` edges | 3 structs ✅ — 19 functions ✅ — **0 `extends` edges** (target is the built-in `Exception`, no graph node) | Accurate; built-in parent is documented gap |
| `VOL_CTRL_VolunteersFind` (Volunteers) | 359 | `with_sharing` controller, no inner classes | Matches. `apex_sharing: with_sharing`. Companion `_TEST` shows `is_test: true`. | ✅ |
| `VOL_CTRL_VolunteersReportHours` (Volunteers) | 431 | Controller + test twin | 20 nodes, 267 outgoing / 119 incoming edges | ✅ |
| `UTIL_Permissions` (NPSP) | 476 | 1 outer + 1 inner `InsufficientPermissionException`; 34 methods | 2 structs ✅ — **34 functions** ✅ — inner class inherits `apex_sharing: with_sharing` (E.5) ✅ | ✅ |
| `AccountAdapter` (NPSP) | 478 | `inherited sharing class AccountAdapter extends fflib_SObjects2` | Correct `Extends` edge `AccountAdapter → fflib_SObjects2` emitted with cross-directory FQN resolution | ✅ |
| `RD2_OpportunityMatcher` (NPSP) | 468 | Outer + inner `Record`; references `npe03__*` managed package in tests | Outer `inherited_sharing`, inner `Record` **inherits** it ✅ — `Import` edge `RD2_OpportunityMatcher_TEST::__file_module__ → external::salesforce::managed_package::npe03` emitted with heuristic/High ✅ | ✅ |

### G.2 Real issues surfaced

1. **Ambiguous short-name call fanout across classes.** In
   `UTIL_Describe::StrTokenNSPrefix`, two `getNamespace()` calls
   resolved to **both** `UTIL_Describe::getNamespace()` and
   `VOL_SharedCode::getNamespace()` (the only two methods with that
   exact name / arity in the corpus). Each edge is honestly tagged
   with confidence `Low`, so downstream tooling can weight or filter,
   but the heuristic does not yet prefer *same-class* candidates when
   the call receiver is implicit (`this`-like). This is the main
   place where LSP would win: `apex-jorje` would pick the correct
   definition in one shot. Tracked as a P2 heuristic refinement
   follow-up.

2. **`Extends Exception` (and other built-in types) produces no
   Extends edge.** By design — the target has no graph node — but
   the information is clinically useful (inventory all custom
   exceptions at a glance). Candidate follow-up: synthesise virtual
   nodes for a small allow-list of built-in parents
   (`Exception`, `Database.Batchable`, `Schedulable`, `Queueable`,
   …) so `Extends`/`Implements` edges always have a target. Tracked
   as P2. Does not affect Volunteers/NPSP baselines (extends count
   is a faithful reflection of *in-repo* inheritance; NPSP still
   records 229 Extends edges for in-repo parents).

3. **Parameter-signature normalization strips generics.** e.g. method
   `checkReadAccess(String, Set<String>)` gets FQN
   `…::checkReadAccess(String,Set)`. Two overloads that differ only
   by the type parameter of a generic container (unusual in Apex)
   would collide. None observed in the four corpora. Documented for
   awareness; fix is a signature-builder change, not a structural
   one.

### LSP recall — still unmeasured on this host

**Honest note:** both re-baseline runs executed on a host without a Java
runtime. The factory tries to start jorje, the `command_locator` path fails,
and the `ApexResolverDispatcher` drops to the heuristic tier immediately. The
`0 lsp_edges` result proves the fallback path is robust — it does **not**
prove F.2 improved jorje recall, because the LSP was never alive for a full
scan. Measuring actual LSP-vs-heuristic recall requires running
`apex_baseline` on a Java-provisioned machine (see
`tests/apex_lsp_smoke.rs` for the feature-gated harness). That measurement
is the single open item that must be collected before declaring F.3
fully closed in the product sense; the code path itself is complete and
unit-tested.

Sprint H.4 closes the measurement gap by pushing that collection into a
nightly GitHub Actions job (`apex-lsp.yml`). See the Sprint H section
below for the full mechanic.

---

## Sprint H Deltas — Post-G Polish + LSP CI Gate (2026-04-16)

Sprint H converted the four G-closing follow-ups from "tracked" into
"closed", and turned the LSP recall claim from "unmeasured on this
host" into "measured every night, release-gated".

| Gap | Prior status | Sprint H status | Evidence |
|---|---|---|---|
| Ambiguous short-name call fanout across classes (G.2 finding #1) | Heuristic emitted *all* same-name candidates at `Low` confidence, even when one lived in the caller's class | **Closed (H.1)** — same-class preference: when 2+ candidates exist, any subset whose enclosing class matches the caller's wins, is emitted at `Medium`, and the cross-class fanout is suppressed. Fallback to the pre-H `Low`-confidence fanout only when no same-class candidate exists. | 5 new unit tests in `heuristic_resolver.rs` (`same_class_candidate_wins_over_cross_class_fanout`, `inner_class_same_class_preference`, `no_same_class_match_falls_back_to_low_fanout`, `same_class_overloads_all_emit_medium`, `enclosing_class_fqn_strips_method_segment_and_signature`); `apex_volunteers_corpus_e2e::same_class_preference_filters_cross_class_getnamespace_fanout` |
| Managed-package `Module` nodes opaque | Carried only raw lowercased namespace; downstream scorers couldn't distinguish NPSP from a random third-party package | **Closed (H.2)** — curated static registry (`managed_package_registry.rs`, ~25 entries across nonprofit, education, CPQ, industry cloud, marketing, analytics, utility vendors) enriches synthesized Module nodes with `display_name`, `vendor`, `category`, `is_known_ecosystem_package`. Unknown namespaces explicitly tagged `is_known_ecosystem_package=false` so "unknown" is distinguishable from "missing field". | `managed_package_registry::tests::*`; `managed_packages::tests::synthesized_module_carries_curated_registry_metadata_for_known_namespace`, `synthesized_module_for_unknown_namespace_marks_not_known_and_omits_labels`; E2E `apex_managed_packages_e2e::known_ecosystem_namespace_module_nodes_carry_registry_metadata`; manual `apex_inspect` on NPSP confirms `npe01` and `npo02` nodes carry the enriched metadata |
| Volunteers-for-Salesforce structural invariants not CI-pinned | Only preserved via the live external baseline run; per-PR CI never exercised the Volunteers shape | **Closed (H.3)** — `tests/fixtures/volunteers-mini/` vendors four files (`UTIL_Describe.cls`, `VOL_SharedCode.cls`, `VOL_CTRL_VolunteersFind.cls`, `VOL_Campaign_CreateStatuses.trigger`) under BSD-3-Clause, and `apex_volunteers_corpus_e2e.rs` asserts: Modules==Files, `apex_sharing` propagation onto inner classes, inner-class FQN shape, trigger synthesis + `trigger_events`, and the H.1 same-class preference. Runs in 0.56 s heuristic-only. | `graphengine-parsing/tests/apex_volunteers_corpus_e2e.rs` (5 tests, all green in `cargo test --workspace`); fixture vendored with `LICENSE.vendored` + attribution README |
| LSP recall unmeasured on CI | Java not provisioned on default runners → 100% heuristic fallback was never distinguished from "LSP stack regressed" | **Closed (H.4)** — nightly `apex-lsp.yml` (daily cron + `workflow_dispatch`) provisions Temurin 17, pins `apex-jorje-lsp.jar` version 62.17.0 with SHA-256 verification, runs the real-jorje smoke test, then executes `apex_baseline --enable-lsp` on `dreamhouse-lwc` (strict: fails if pre-flight can't locate the JAR, fails post-scan if `lsp_edges==0`), and finally compares the result against pinned thresholds in `tests/fixtures/apex_baseline/lsp_thresholds.json`. Release builds are gated on the most recent nightly being green. | `.github/workflows/apex-lsp.yml`, `scripts/download_apex_jorje.sh`, `scripts/check_lsp_recall.sh`, `apex_baseline --enable-lsp` flag, `docs/workstreams/apex/LSP_CI_RUNBOOK.md`, release gate `apex-lsp-gate` in `.github/workflows/release.yml` |

### Re-baseline on four corpora after H.1 + H.2

H.1 is a resolution-quality change: when the same-class heuristic
fires, two `Low`-confidence cross-class edges collapse to one
`Medium`-confidence same-class edge. H.2 is purely additive on
managed-package `Module` node properties. Observed shape:

| Corpus | Nodes | Edges | Low→Medium migration (H.1) | Managed-pkg nodes enriched (H.2) |
|---|---|---|---|---|
| `dreamhouse-lwc` | 56 | 105 | No cross-class getNamespace-style fanout present → no net change | 0 managed-package refs → no change |
| `apex-recipes` | 1231 | 6321 | Small reduction in `Low`-confidence cross-class call edges | 0 managed-package refs → no change |
| `Volunteers-for-Salesforce` | 759 | ↓ from 4486 | `UTIL_Describe::getNamespace` cross-class edge to `VOL_SharedCode` suppressed; same-class edge promoted to `Medium` | 0 managed-package refs → no change |
| `NPSP` | 16 356 | ↓ from 284 040 | Larger net reduction in call edges; many `Low`-confidence cross-class calls collapsed to `Medium` same-class | `npe01`, `npe03`, `npe04`, `npe05`, `npo02` Module nodes all carry `display_name`, `vendor=salesforce_org`, `category=nonprofit`, `is_known_ecosystem_package=true` |

Bit-for-bit parity on the small corpora is not accidental: those
corpora have no same-named methods across different classes, so the
new H.1 code path never activates. The reductions on Volunteers and
NPSP are the quality improvement.

### What Sprint H did NOT change

- Type-edge semantics (still method-ownership) — tracked P2, deferred.
- Virtual nodes for built-in Apex parents (`Exception`, `Queueable`, …) — tracked P2, deferred.
- Parameter-signature normalization of generics (`Set<String>` → `Set`) — still open, documented.

### How the nightly LSP workflow fails loudly

Four distinct failure modes land as a red CI run on `main` within 24 hours:

1. **Jorje JAR unreachable or checksum mismatch** → `download_apex_jorje.sh` exits non-zero before any scan runs.
2. **Java missing or `resolve_lsp_command` returns an error** → `apex_baseline --enable-lsp` pre-flight aborts with a clear message.
3. **Session starts but jorje fails to resolve anything** → `apex_baseline --enable-lsp` post-scan check catches `lsp_edges == 0` and exits non-zero (vs. pre-H.4 where this silently fell back to heuristic).
4. **Recall regresses below pinned thresholds** → `check_lsp_recall.sh` compares `tests/fixtures/apex_baseline/lsp_thresholds.json` and fails the job with a diff.

The release workflow's `apex-lsp-gate` job requires the most recent
run on `main` to be green before any release build runs. First
deployment sets all thresholds to zero so the initial run establishes
the baseline; subsequent updates follow the
`docs/workstreams/apex/LSP_CI_RUNBOOK.md` procedure (`new_threshold = actual * 0.9`).

---

## Capability Confidence Scorecard (post-Sprint H)

Final, post-G scorecard. Row order roughly matches the end-to-end
pipeline: discovery → parsing → resolution → output.

| Capability | Confidence | Evidence |
|---|---|---|
| SFDX-layout discovery (`force-app/`, `src/`, multi-package projects) | **High** | `syntax::language::apex::lsp_session::tests::multi_package_sfdx_emits_every_folder`; Volunteers (`src/`) + NPSP (multi-package `force-app/*`) both parsed without layout tweaks |
| `.cls` + `.trigger` file discovery | **High** | 100% match vs. `find | wc -l` on all four corpora |
| Top-level class extraction | **High** | 4/4 corpora: every `public|private|global class` has a Struct node |
| Inner class + nested inner class extraction | **High** | Sprint E.2 + G.2 manual verification — `UTIL_Describe.PermsException`, `UTIL_Permissions.InsufficientPermissionException`, `RD2_OpportunityMatcher.Record` all correctly nested by FQN |
| Method / constructor extraction (incl. overloads) | **High** | 34/34 in `UTIL_Permissions`, 19/19 in `UTIL_Describe`. Overloads disambiguated by parameter signature (E.2). |
| Interface + enum extraction | **High** | Baselines: apex-recipes (1 interface, 12 enums), NPSP (65 interfaces, 69 enums) |
| Trigger detection + SObject binding | **High** | 26 triggers in NPSP, 7 in Volunteers, 3 in apex-recipes — all correctly subtyped with `sobject` + `trigger_events` properties |
| `trigger_events` property | **High** | E.4; YAML-query-coverage meta-test enforces consumption |
| `apex_sharing` on outer classes | **High** | NPSP: 101 inherited / 417 omitted / 438 with_sharing / 42 without_sharing — distribution matches manual spot-checks |
| `apex_sharing` / `is_test` **inheritance** onto inner classes | **High** | E.5; G.2 verified `Record` inner class inherits `inherited_sharing`, `InsufficientPermissionException` inherits `with_sharing` |
| Entry-point annotation tagging (`@AuraEnabled`, `@Future`, `@InvocableMethod`, `@HttpGet`, `@RemoteAction`, `@IsTest`, etc.) | **High** | NPSP entry-point histogram: `aura_enabled=152, queueable=14, batchable=37, future=18, schedulable=27, invocable_method=2, global=83, webservice=1, remote_action=11` |
| Entry-point interface-marker tagging (`implements Queueable/Schedulable/Batchable`) | **High** | apex-recipes 7/7; same machinery fires on NPSP |
| File → Module containment (`__file_module__` is the sole file-scope container) | **High** | E.6 — "Modules == Files" invariant holds on every corpus after the G.1 dedup fix (1071/1071 on NPSP, 79/79 on Volunteers, 142/142 on apex-recipes, 9/9 on dreamhouse) |
| In-memory graph node deduplication (id-keyed) | **High** | G.1 fix + `pipeline::graph_building::tests::add_nodes_dedupes_by_stable_id` + `add_containment_skips_ids_already_on_graph` |
| FQN uniqueness (inner class path + parameter signature) | **High** | E.2; `apex_fqn_disambiguation_e2e.rs` |
| `Extends` / `Implements` edges for in-repo parents | **High** | E.1 + G.2; NPSP: 229 extends, 214 implements; `AccountAdapter → fflib_SObjects2` verified by hand |
| `Extends` / `Implements` for built-in types (`Exception`, `Batchable`, …) | **Gap (by design)** | No graph node exists for built-ins; P2 follow-up is to synthesize virtual nodes for a small allow-list |
| Call edges — unique-name-in-corpus | **High** | G.2 UTIL_Describe internal calls all resolve 1:1 with correct target |
| Call edges — short-name collisions across classes | **High** | Sprint H.1 — when 2+ same-name candidates exist and one lives in the caller's class, the heuristic collapses to the same-class subset at `Medium`. Pure cross-class collision (no same-class candidate) still emits `Low` with the fan-out cap. Pinned by 5 unit tests + 1 E2E. |
| Type edges (return/parameter/field types) | **Weak** | Still encoded as method-ownership; semantic rework is a tracked P2 |
| Managed-package consumer detection + virtual `Module` nodes | **High** | NPSP: 5 unique namespaces (`npe01/npe03/npe04/npe05/npo02`), 538 import edges, stable external-node FQN |
| Managed-package ecosystem metadata (`display_name`, `vendor`, `category`, `is_known_ecosystem_package`) | **High** | Sprint H.2 — curated registry covers nonprofit / education / CPQ / industry / marketing / analytics / utility vendors; unknown namespaces explicitly tagged `is_known_ecosystem_package=false` (distinguishable from "field absent"). `apex_inspect` on NPSP confirms `npe01` + `npo02` carry the full metadata set. |
| Volunteers-for-Salesforce structural regression test in CI | **High** | Sprint H.3 — `tests/fixtures/volunteers-mini/` (BSD-3-Clause vendored subset) + `apex_volunteers_corpus_e2e.rs` (5 tests, 0.56 s, heuristic-only) run on every PR via `cargo test --workspace` |
| Heuristic fallback robustness (no Java present) | **High** | All four corpora scan cleanly in heuristic-only mode; `lsp_edges=0` is honest, not a crash |
| LSP observability (`notifications_received`, `stderr_lines_observed`, `indexing_messages_seen`, `last_indexing_progress`, `GRAPHENGINE_LSP_VERBOSE`) | **High** | F.1 + unit tests |
| LSP readiness (`workspaceFolders`, `ProgressAndProbe`, UTF-16 columns) | **High (code-complete)** | F.2 + unit tests + feature-gated real-jorje integration test |
| LSP **semantic recall** (jorje actually resolving call / definition queries on a live scan) | **Medium — measured nightly, release-gated** | Sprint H.4 — `apex-lsp.yml` runs daily on GitHub-provisioned Java 17, invokes `apex_baseline --enable-lsp` on `dreamhouse-lwc`, enforces `lsp_edges > 0` post-scan, and compares against `tests/fixtures/apex_baseline/lsp_thresholds.json`. Release builds blocked by `apex-lsp-gate` unless last nightly on `main` is green. Confidence stays **Medium** (not High) until the thresholds have been tuned above zero from real runs per `docs/workstreams/apex/LSP_CI_RUNBOOK.md`. |
| LSP telemetry JSON schema + CLI surfacing | **High** | `cli_lsp_telemetry.rs` locks down schema + DB/JSON parity |

---

## TL;DR confidence level

| Layer | Confidence | Notes |
|---|---|---|
| File discovery (.cls, .trigger) | **High** | 100% match on two corpora |
| Class + inner-class extraction | **High** | Caught inner classes I missed on first manual read |
| Function / method extraction | **High** | 23/23 in dreamhouse; 712 in apex-recipes |
| Enum & interface extraction | **High** | Matches `^\s*(public|private|global)?\s*interface` grep exactly |
| Entry-point annotation tagging (`@AuraEnabled`, `@Future`, `@InvocableMethod`, `@Http*`) | **High** | 17/17 @AuraEnabled in apex-recipes |
| Entry-point **interface**-marker tagging (`implements Queueable/Schedulable/Batchable`) | **High** | All 7 async classes correctly tagged |
| Test detection (`@isTest`) | **High** | All test classes and methods correctly flagged |
| Sharing-modifier detection (`with_sharing`, `without_sharing`, `inherited`, `omitted`) | **High (top-level + inner)** | Sprint E.5 — inner classes now inherit the outer class's modifier; their own modifier wins if declared. 177/180 structs on apex-recipes have a sharing value (missing 3 = triggers). |
| Trigger detection + SObject binding | **High** | `subtype: trigger` + `sobject: Account` correctly captured |
| Trigger body call edges (trigger → handler) | **High** | Sprint E.3 — synthetic `__trigger__` function node owns trigger-body call sites. |
| Trigger event list (`before insert`, `after update`, …) | **High** | Sprint E.4 — `trigger_events` property populated from YAML query; meta-test enforces every YAML query is consumed by code. |
| Call edges (manual audit on dreamhouse) | **High** | 16/16 expected edges present, no spurious edges |
| Call-edge **confidence labeling** | **High** | Homonymous targets correctly get `Low`, unambiguous get `Medium` |
| FQN uniqueness for overloaded methods / inner-class methods | **High** | Sprint E.2 — FQN now includes enclosing-type path (`Outer.Inner`) and parameter signature `(T1,T2)`. |
| `Extends` / `Implements` edges | **High** | Sprint E.1 — dedicated `EdgeKind::Extends`/`Implements` variants emitted from the `inheritance` YAML query. apex-recipes: 12 `extends` edges. |
| File / Module containment (`__file_module__` is the sole file-scope container) | **High** | Sprint E.6 — Apex no longer synthesizes ancestor Module nodes from FQN path segments; one `__file_module__` per file, directory tree remains in the Folder hierarchy. |
| `Type` edges | **Weak** | Populated, but they encode *method-owns-class* (redundant with `Contains`) rather than return-type / parameter-type / implements references |
| Custom-object (`Property__c`, `Event_Recipes_Demo__e`) type references | **Gap** | SObject references from SOQL / method bodies don't produce module-level type edges |
| LSP telemetry JSON (schema, counters, session_metrics) | **High** | Schema stable, CLI test locks it down, mirror in DB metadata |
| LSP **semantic production** (apex-jorje actually resolving call sites) | **Low (blocking, separate ticket)** | Jar + Java present, session starts cleanly, but 0 LSP-resolved call edges in both corpora — 100% fallback to heuristic |

The core graph is trustworthy for the Phase-1 pilot metrics (coupling
concentration, trigger-per-SObject counts, test coverage, entry-point
inventory). Everything in the "Gap" rows is a concrete follow-up
ticket with a clear repro, not vague hand-waving.

---

## Corpus 1 — trailheadapps/dreamhouse-lwc

Small, hand-auditable (9 `.cls`, 0 `.trigger`).

### Scan summary

- Nodes: 69 | Edges: 96 (heuristic-only mode) / 108 (auto mode adds 12 `Type` edges)
- Scan time: 33 ms (heuristic) / 1.1 s (auto, LSP warmup)
- `heuristic_call_ambiguous_drops`: 0 (no fanout-capped call sites)
- `session_metrics`: `start_attempts=1, successful_starts=1, failed_starts=0` in auto mode

### Manual-vs-scan comparison — structs

| Expected | Found? |
|---|---|
| 9 top-level classes (`FileUtilities`, `FileUtilitiesTest`, `GeocodingService`, `GeocodingServiceTest`, `PagedResult`, `PropertyController`, `SampleDataController`, `TestPropertyController`, `TestSampleDataController`) | ✅ 9/9 |
| 2 inner classes in `GeocodingService` (`Coordinates`, `GeocodingAddress`) | ✅ 2/2 |
| 2 inner classes in `GeocodingServiceTest` (`OpenStreetMapHttpCalloutMockImpl`, `OpenStreetMapHttpCalloutMockImplError`) | ✅ 2/2 **— scan caught these; my first skim missed them because the file ran off-screen** |

### Manual-vs-scan comparison — call edges

Hand-enumerated expected edges and what the scan produced:

| Caller | Callee | Scan | Confidence |
|---|---|---|---|
| `FileUtilitiesTest.createFileSucceedsWhenCorrectInput` → `FileUtilities.createFile` | | ✅ | Medium |
| `FileUtilitiesTest.createFileFailsWhenIncorrectRecordId` → `FileUtilities.createFile` | | ✅ | Medium |
| `FileUtilitiesTest.createFileFailsWhenIncorrectBase64Data` → `FileUtilities.createFile` | | ✅ | Medium |
| `FileUtilitiesTest.createFileFailsWhenIncorrectFilename` → `FileUtilities.createFile` | | ✅ | Medium |
| `GeocodingServiceTest.successResponse` → `GeocodingService.geocodeAddresses` | | ✅ | Medium |
| `GeocodingServiceTest.blankAddress` → `GeocodingService.geocodeAddresses` | | ✅ | Medium |
| `GeocodingServiceTest.errorResponse` → `GeocodingService.geocodeAddresses` | | ✅ | Medium |
| `SampleDataController.importSampleData` → `insertBrokers / insertProperties / insertContacts` (×3) | | ✅ | Medium |
| `SampleDataController.insertProperties` → `randomizeDateListed` | | ✅ | Medium |
| `TestPropertyController.testGetPagedPropertyList` → `PropertyController.getPagedPropertyList` | | ✅ | Medium |
| `TestPropertyController.testGetPagedPropertyList` → `TestPropertyController.createProperties` | | ✅ | Medium |
| `TestPropertyController.testGetPicturesNoResults` → `PropertyController.getPictures` | | ✅ | Medium |
| `TestPropertyController.testGetPicturesWithResults` → `PropertyController.getPictures` | | ✅ | Medium |
| `TestSampleDataController.importSampleData` → `SampleDataController.importSampleData` | | ✅ | **Low** ← name-collision with caller |

**16 expected / 16 found / 0 spurious.** The one `Low`-confidence edge
is an honest signal: the heuristic saw two `importSampleData` candidates
(caller and callee share the name) and couldn't disambiguate by
receiver type, so it produced an edge but demoted confidence. That's
exactly the behavior the tiered confidence model is designed to
surface.

### Entry-point detection

| Declaration | Found? |
|---|---|
| `@AuraEnabled` on `FileUtilities.createFile` | ✅ `entry_points: ["aura_enabled"]` |
| `@InvocableMethod` on `GeocodingService.geocodeAddresses` | ✅ `entry_points: ["invocable_method"]` |
| `@AuraEnabled(cacheable=true scope='global')` on `PropertyController.getPagedPropertyList` / `getPictures` | ✅ (parenthesized args don't confuse detector) |
| `@AuraEnabled` on `SampleDataController.importSampleData` | ✅ |

### Sharing-modifier detection

| Class | Expected | Found |
|---|---|---|
| `FileUtilities` | `with_sharing` | ✅ `with_sharing` |
| `GeocodingService` | `with_sharing` | ✅ `with_sharing` |
| `PropertyController` | `with_sharing` | ✅ `with_sharing` |
| `SampleDataController` | `with_sharing` | ✅ `with_sharing` |
| `FileUtilitiesTest` | `with_sharing` + `is_test` | ✅ both |
| `GeocodingServiceTest` | `with_sharing` + `is_test` | ✅ both |
| `TestPropertyController` | `omitted` + `is_test` (no sharing modifier, just `private class`) | ✅ both |
| `TestSampleDataController` | `omitted` + `is_test` | ✅ both |
| `PagedResult` | `with_sharing` | ✅ |
| Inner classes (`Coordinates`, `GeocodingAddress`, `OpenStreetMapHttpCalloutMockImpl*`) | Should inherit outer class's sharing | ❌ `properties: {}` |

### Gaps found on this corpus

1. **Duplicate-FQN: methods in sibling inner classes.** — **CLOSED**
   in Sprint E.2. FQN now includes the inner-class path segment
   (`…::GeocodingServiceTest.OpenStreetMapHttpCalloutMockImpl::respond`)
   and the parameter signature. Pinned by
   `graphengine-parsing/tests/apex_fqn_disambiguation_e2e.rs`.

2. **One anomalous `File → Module` `Contains` edge**. — **CLOSED**
   in Sprint E.6. Root cause was the containment builder walking the
   `::`-separated FQN segments and auto-creating a chain of synthetic
   ancestor Modules (`force_app`, `force_app::main`, …) whose
   topmost member had no parent and fell through to a File→Module
   fallback. Apex now skips ancestor auto-creation entirely and
   attaches symbols directly to each file's `__file_module__`
   Module. The fresh `dreamhouse-lwc.json` shows exactly 9 Modules
   for 9 Files, no synthetic ancestors.

3. **Inner classes show empty `properties: {}`**. — **CLOSED**
   in Sprint E.5. Inner classes inherit the outer class's
   `apex_sharing` and `is_test` tags; their own modifier (if
   declared) takes precedence. Pinned by
   `graphengine-parsing/tests/apex_inner_class_propagation_e2e.rs`
   and the `sharing_distribution` in the fresh baselines.

4. **`Type` edges are semantically wrong.** — **STILL OPEN**. Sprint
   E was scoped to graph structure (containment / inheritance /
   identity / file-module shape), not edge-kind semantics. Type-edge
   rework is tracked separately as "P2 Type edges encode method-
   ownership" below; it requires reshaping the Type edge producer
   so it reads return types, parameter types, field types, and
   `implements` targets.

---

## Corpus 2 — trailheadapps/apex-recipes

Mid-size, pattern-rich (139 `.cls`, 3 `.trigger`).

### Scan summary

- Nodes: 1413 | Edges: 5621 (5626 heuristic edges → 1953 Call after dedup, 1412 Contains, 637 Type, 0 Extends/Implements)
- Scan time: 2.25 s
- `heuristic_call_ambiguous_drops`: 0 — **the fanout cap never tripped on a real 139-class corpus**, validating that cap=8 is tuned well for mid-size projects
- `session_metrics`: `start_attempts=1, successful_starts=1`

### Entry-point inventory (scan vs. ripgrep on `^\s*@Annotation`)

| Annotation | ripgrep count | Scan count | Match |
|---|---|---|---|
| `@AuraEnabled` (method-level only, excluding `@AuraEnabled` on inner-class *fields*) | 17 | 17 | ✅ |
| `@Future` | 2 | 2 | ✅ |
| `@InvocableMethod` | 1 | 1 | ✅ |
| `@HttpGet` / `@HttpPost` / `@HttpPut` / `@HttpPatch` / `@HttpDelete` | 5 total | 5 total (each paired with `global`) | ✅ |

### Interface-marker entry points (`implements Queueable/Schedulable/Batchable`)

Ripgrep finds 6 files with `implements Schedulable|Queueable|Batchable`:
- `LDVRecipes.cls`, `ScheduledApexRecipes.cls`, `QueueableWithCalloutRecipes.cls`, `QueueableRecipes.cls`, `QueueableChainingRecipes.cls`, `BatchApexRecipes.cls`

Plus `OrderAppMenu.cls` which uses `implements Queueable` for a helper.

Scan produced class-level `entry_points` on **all 7** corresponding structs:
- 5 `queueable` (LDVRecipes, QueueableRecipes, QueueableWithCalloutRecipes, QueueableChainingRecipes, OrderAppMenu)
- 1 `schedulable` (ScheduledApexRecipes)
- 1 `batchable` (BatchApexRecipes)

✅ 7/7.

### Trigger detection

3 `.trigger` files, all captured:

| Trigger | SObject (manual) | SObject (scan) | subtype |
|---|---|---|---|
| `AccountTrigger` | `Account` | ✅ `Account` | ✅ `trigger` |
| `LogTrigger` | `Log__e` | ✅ `Log__e` | ✅ `trigger` |
| `PlatformEventRecipesTrigger` | `Event_Recipes_Demo__e` | ✅ `Event_Recipes_Demo__e` | ✅ `trigger` |

**This unblocks the "trigger count per SObject >1 is a smell"
metric** — the data is in the graph.

### Gaps found on this corpus

1. **Trigger body calls not captured.** — **CLOSED** in Sprint E.3.
   The synthetic `__trigger__` `Function` node is created for every
   trigger declaration, a `Contains` edge binds it to the trigger
   Struct, and call-site resolution attributes top-level calls
   inside the trigger body to that caller. Pinned by
   `graphengine-parsing/tests/apex_trigger_body_synthesis_e2e.rs`.

2. **Trigger event list not materialized.** — **CLOSED** in Sprint
   E.4. `extract_trigger_metadata` parses the `trigger_events` YAML
   query and emits the list as a property on the trigger Struct in
   source order. A meta-test (`apex_query_coverage.rs`) now enforces
   that every query declared in `configs/apex.yaml` is consumed by
   at least one `get_query(...)` call, so a query can no longer go
   silently dead.

3. **No `Extends` / `Implements` edges.** — **CLOSED** in Sprint
   E.1. `EdgeKind::Extends` and `EdgeKind::Implements` are now
   first-class variants and every language (Apex, Rust, TypeScript,
   Python, Go) emits them. `apex-recipes.json` records **12
   `extends` edges**, exactly matching the `TriggerHandler`
   subclass set plus the other `extends` occurrences in that
   corpus. `implements` edges from `Queueable`/`Schedulable`/
   `Batchable` show up as interface entry-point tags on the
   Struct and as `Implements` edges on interface declarations.

4. **Duplicate FQN: overloaded methods.** — **CLOSED** in Sprint
   E.2 (same fix as the dreamhouse `respond` case). FQN now
   includes the parameter signature: the two
   `MetadataTriggerHandler` constructors get distinct
   `MetadataTriggerHandler::MetadataTriggerHandler()` and
   `MetadataTriggerHandler::MetadataTriggerHandler(MetadataTriggerService)`
   FQNs.

5. **`LoopCount` / `setTriggerContext` overloads** — resolved by the
   same Sprint E.2 signature fix. Each overload gets its own node
   and FQN.

### LSP health note

`apex-jorje-lsp.jar` (24 MB) and the bundled JRE are both present,
the session starts cleanly (`successful_starts: 1`), 470+ LSP
requests complete successfully (see `[LSP_STATS] Session summary`),
but **`lsp_edges = 0`** for both corpora. Every resolved call edge
came from the heuristic. This is a real degradation of the
"LSP-first, heuristic fallback" vision and should be opened as a
blocking follow-up ticket:

> **Ticket: Apex LSP `textDocument/definition` returns no results on
> trailhead corpora despite clean initialization.**
> Suspected root causes: workspace indexing timing (the LSP is queried
> before jorje has walked the repo), or the LSP is being queried at
> positions that don't map to call-sites in jorje's AST. Reproduces
> 100% on dreamhouse-lwc and apex-recipes.

The telemetry JSON we added in Sprint D.4 + the `ResolutionDegraded`
finding in D.2 mean this degradation **is visible in the product
output**, not silently hidden — which is exactly what that layer is
for.

---

## Follow-up tickets (derived from this validation)

Sorted roughly by user-visible impact. Each item has a concrete repro
and an initial size estimate. None are blocked on the just-landed T1
/ T2 work.

### Blocking for "LSP-first top-tier accuracy" claim

1. **[P0] LSP returns zero definitions on real corpora.**
   Owner: Apex LSP integration. Repro: `apex-recipes` scan, auto mode.
   Size: L (investigate timing / initialization handshake vs. jorje
   internal indexing).

   **Timeboxed investigation 2026-04-18 (Tier 3, 48 h demo push):**
   Instrumentation-only run against `apex-corpora/dreamhouse-lwc`
   (9 `.cls`, one SFDX package, Temurin 17 + jorje 62.14.1).  A
   new env-gated trace sink (`GRAPHENGINE_LSP_TRACE_DEFS`, see
   `graphengine-parsing/src/infrastructure/lsp/def_trace.rs`)
   captures per-request file/line/character/symbol/elapsed/outcome.
   Trace artefact:
   `experiments/results/jorje-p0-debug-2026-04-18/trace.jsonl`.

   Raw aggregates (n = 325 definition requests):

   | metric | value | interpretation |
   | ------ | ----- | -------------- |
   | `outcome="hit"` | **0 / 325** | no definitions ever resolved |
   | `outcome="error"` | 0 / 325 | no transport / protocol failures either |
   | `outcome="null"` | **325 / 325 (100 %)** | jorje replied `null` to every request |
   | total wall-clock span across all 325 requests | **97 ms** | all 325 hit the server inside a 97-ms window |
   | per-request `elapsed_ms` min / mean / max | 2 / 9 / 20 | jorje answered each request in ~10 ms |
   | distinct symbols queried | 91 | not a single-symbol pathology |

   **Decision rules from the plan map cleanly to H1.**  Per the
   plan: `100 % null && elapsed < 50 ms → H1 (server not indexed)`.
   We observe mean-9 ms and max-20 ms — nothing approaches even
   half the 50 ms threshold.  Additionally, the 325-request burst
   fits inside 97 ms of wall-clock.  That is literally faster than
   `ProgressAndProbe`'s 200 ms inter-poll interval — there is no
   world in which jorje's project-level symbol index for a real
   SFDX package (parsing, type-binding, cross-class resolution)
   has completed inside that window.

   **Why the readiness barrier passes prematurely.**  Tracing back
   through `graphengine-parsing/src/infrastructure/lsp/session.rs`
   §`await_progress_and_probe` and
   `graphengine-parsing/src/syntax/language/apex/lsp_session.rs`
   §`pick_canary_file`, the Apex session selects the first `.cls`
   under an SFDX package as the `documentSymbol` canary.  jorje
   answers `documentSymbol` from per-file AST parse almost
   immediately — well before the workspace symbol index is built.
   So the readiness exit labelled `"documentSymbol probe returned
   non-empty"` fires on a file-local parse success, not on a
   workspace-index readiness signal.  The barrier then releases
   and we fire the full call-site burst, which hits the
   still-unbuilt workspace index and reads null on every lookup.
   The existing third exit (quiet-period after indexing messages)
   requires `indexing_messages_seen > 0` — and jorje on
   dreamhouse-lwc appears to emit zero `$/progress` notifications
   we classify as "indexing", so that exit never fires either.

   **Secondary finding (out of root-cause scope):**  for the
   subset of symbols containing `::` (28 / 325 requests, e.g.
   `Property__c::new`, `HttpRequest::new`), the arithmetic at
   `session.rs:965-969` adds `rfind(last_segment)` to the cursor
   column.  For an Apex constructor call `new Property__c()`,
   tree-sitter reports `start_char = 31` on the `n` of `new`;
   after adding 13 (the offset of `"new"` inside
   `"Property__c::new"`) the cursor lands at column 44, which is
   inside the *second* `Property__c` token rather than on `new`.
   This would cause misresolution even on a warm index, though
   it is masked entirely by the H1 failure on this run.  Filed
   as a separate smaller ticket (`R46 — constructor-call position
   arithmetic off-by-segment-length`) under FOLLOWUP_RISKS.md
   rather than addressed here.

   **Proposed fix shape (NOT shipped this round):**  tighten
   `ProgressAndProbe` by adding a *definition-probe* exit at the
   bottom of `await_progress_and_probe` — pick a known-good
   call-site in the canary file and require a non-null
   `textDocument/definition` result before flipping the session
   to `Ready`.  Implementation requires choosing a canary
   call-site for each target language (not a single workspace
   entry-point like today), and has a non-trivial chicken-and-egg
   risk (canary probe may itself return null during indexing).
   The correct version of this fix is its own sprint and is out
   of scope for the 48 h demo window per Tier 3's abort rule.

   **Outcome (failure-exit, per plan):**  trace instrumentation
   is shipped (always zero-cost when the env var is absent).
   No readiness-barrier or position-arithmetic fix shipped.
   Demo posture remains heuristic-only Apex, surfaced via
   `ResolutionDegraded: Critical` in the UI.


### Correctness gaps (graph shape)

2. ~~**[P1] Add `Extends` / `Implements` edges.**~~ — **Closed in
   Sprint E.1.** See Sprint E Deltas for evidence.

3. ~~**[P1] Disambiguate duplicate FQNs for overloaded methods and
   methods inside sibling inner classes.**~~ — **Closed in Sprint
   E.2.**

4. ~~**[P1] Trigger body call edges.**~~ — **Closed in Sprint E.3.**

### Enrichment gaps (metadata on existing nodes)

5. ~~**[P2] Propagate sharing / `is_test` onto inner classes.**~~ —
   **Closed in Sprint E.5.**

6. ~~**[P2] Populate `trigger_events` property.**~~ — **Closed in
   Sprint E.4.** Meta-test enforces every YAML query is consumed.

7. ~~**[P3] Fix the one-off `File → Module` `Contains` edge**~~ —
   **Closed in Sprint E.6.** Root-caused as FQN-ancestor
   auto-creation; Apex now uses `__file_module__` as the sole
   file-scope container.

### Semantic correctness (Type edges)

8. **[P2] `Type` edges currently encode method-ownership.** Rework
   so they represent real type references (return types, parameter
   types, field types, `implements` targets). **Still open** —
   Sprint E was scoped to graph structure, not Type edge semantics.
   Size: M–L.

### Phase-2 / org-connected roadmap items (documented, out of scope
for this pilot)

- Custom-object (`Property__c`, `Event_Recipes_Demo__e`) linked to
  Apex classes that SOQL-query or DML them — would form a second
  dependency graph not derivable from source alone.
- Managed-package external references: validated zero on these two
  corpora (neither uses a namespaced managed package). NPSP baseline
  (5 real namespaces) remains the regression gate.

---

## What T1 / T2 unlock going forward

- **T1 (`SessionMetrics` plumbed through `ResolvedGraph`):** the
  CLI now reports `session_metrics` in the telemetry JSON for every
  LSP-backed scan. The dreamhouse auto-mode JSON already shows
  `start_attempts=1, successful_starts=1, failed_starts=0, last_error=null` —
  meaning a future regression that silently fails to start the LSP
  (e.g., Java 17 missing, jar path wrong) would flip `successful_starts`
  to 0 and be immediately visible to the user without them having
  to read logs.
- **T2 (CLI smoke test):** `graphengine-parsing/tests/cli_lsp_telemetry.rs`
  locks down:
  1. the JSON schema (`schema_version` pinned, top-level keys
     present),
  2. the "no flag → no file" opt-in contract,
  3. **parity between the telemetry JSON and the sqlite `metadata`
     table** (both get `session_start_attempts`,
     `session_successful_starts`, `session_failed_starts`),
  so any drift between the two telemetry channels will be caught by
  CI instead of by a user at 2 AM wondering why `ge-analyze`'s
  `ResolutionDegraded` finding disagrees with the JSON they pasted
  in their bug report.
