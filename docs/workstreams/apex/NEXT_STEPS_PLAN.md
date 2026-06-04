# Apex Integration — Next Steps Plan (post-verification)

Sequencing plan for completing the Apex/Salesforce integration after the
initial implementation sprint. Produced **after live verification** that
surfaced one shipped-would-have-been-silent production bug, so the plan
is informed by real evidence, not speculation.

This document is a companion to:
- `docs/workstreams/apex/INTEGRATION.md` — the authoritative reference for what is
  implemented today.
- `.cursor/plans/apex_salesforce_integration_plan_*.plan.md` — the
  approved implementation plan (do not edit).

---

## 0. Evidence gathered during verification

Before drafting sequencing, real tests were written and run. What was
learned:

### 0.1 Load-bearing bug found in the LSP protocol layer

**Finding.** `graphengine-parsing/src/infrastructure/lsp/protocol.rs`
declared 35 LSP-protocol structs with `#[derive(Serialize, Deserialize)]`
but **zero** `#[serde(rename_all = "camelCase")]` attributes. The LSP
specification mandates camelCase on the wire. The struct fields are in
Rust-native snake_case.

**Why it shipped.** The minimal init path
(`create_initialize_request_minimal`) hand-rolls its JSON with
camelCase keys (`processId`, `rootUri`, `capabilities`), bypassing the
struct. All pre-Apex languages hit only this path because their YAML
configs left `lsp_initialization_options` unset. The camelCase/snake_case
mismatch was **completely invisible** until the first language
(Apex) set `lsp_initialization_options` and routed through
`create_initialize_request_with_options`, which does use the struct.

**Downstream consequence (silent).** The `if let Ok(init_result) =
serde_json::from_value::<InitializeResult>(...)` in
`SimpleLspClient::initialize` was always failing for real servers:
the server sends `definitionProvider`, our struct expects
`definition_provider`. The `Ok` branch was never taken, so
`server_capabilities` was never populated. The session worked anyway
because document operations don't gate on capability info.

**Fix applied (this verification pass).**
`#[serde(rename_all = "camelCase")]` added to all 35 affected structs.
Two new wire-level unit tests pin the contract:

- `with_options_emits_initialization_options_on_wire` — proves Apex
  `initializationOptions` survive the serializer.
- `minimal_path_omits_initialization_options` — proves the minimal path
  for every other language is byte-identical to pre-Apex behaviour.

**Regression sweep.** All 700+ non-LSP tests pass. `cargo test
--features lsp-tests --test lsp_integration_real` has the same 2
failures before and after the fix
(`test_rust_lsp_cross_file_call_edges`, `test_python_lsp_initialization`),
so the fix introduces no new regressions. Whether the camelCase fix now
*improves* `test_rust_lsp_cross_file_call_edges` is worth checking in
Sprint A — if server capabilities are now populating, LSP-backed call
resolution may produce the 6+ edges the test expects (previously it
produced 0).

**Takeaway.** Verification pays. Without the smoke-test harness and
wire-shape assertions, we would have shipped Apex with a silently
degraded semantic pipeline. Every future LSP integration should land
with a wire-shape test *before* the LSP-dependent logic that relies
on it.

### 0.2 Apex smoke test harness committed, cannot run locally

`graphengine-parsing/tests/apex_lsp_smoke.rs` is committed and compiles.
It is feature-gated behind `lsp-tests` (matches existing LSP test
convention) and skips cleanly when the apex-jorje stack is unavailable.
On this host: no Java runtime installed, no `apex-jorje-lsp.jar`
cached — the test skips today. Sprint A must provision both so the
test actually fires in CI or on the desktop build machine.

### 0.3 Pre-existing debt surfaced during regression testing

Not caused by Apex work but now visible:

- **`tests/typescript_lsp_tests.rs`** calls `ensure_initialized()` and
  `shutdown()` on `SessionSupervisor` — neither method exists. The file
  fails to compile under `--features lsp-tests`. Must be fixed or the
  gated LSP test suite cannot build end-to-end.
- **`tests/infrastructure/benches/infrastructure_bench.rs`** fails to
  compile (missing `SyntaxExtractor` trait import). Blocks
  `cargo build --all-targets`. Low-value but easy fix.
- **`test_rust_lsp_cross_file_call_edges`** produces 0 LSP-backed call
  edges. Either rust-analyzer's symbol index is not being consulted
  correctly, or the broken capability deserialization was suppressing
  edge attribution. Needs a root-cause pass — may self-heal after the
  camelCase fix lands.
- **`test_python_lsp_initialization`** times out at 30s. Either pyright
  changed its startup cost, or something in our initialization flow
  stalls on pyright specifically.

---

## 1. Framing: how in-depth should this go?

You asked whether you were overthinking. **No, but you are close to the
right amount of thinking.** Here is the honest read:

### What is cheap and high-value (do now, Sprint A)

- Finish the remaining Apex-feature todos from the approved plan. They
  are mostly additive and well-scoped.
- Run the smoke test against a real apex-jorje on a provisioned machine
  once — this is the **single most-important verification step in the
  whole integration** and currently blocked only by provisioning.
- Wire the two pre-existing breakages (`typescript_lsp_tests`, bench).
  Failing-to-compile files hide real regressions; they must stay green.

### What is medium-cost, medium-value (Sprint B, after smoke test passes)

- Run the end-to-end validation against `dreamhouse-lwc` and NPSP
  and record real recall/coupling numbers in `docs/01-status/CURRENT_STATE.md`.
  This is where "is the integration actually good?" gets a real answer.
- Harden LSP lifecycle: retry/restart on crash, health-check cadence,
  diagnostic surfacing. `SessionSupervisor` already has state machinery
  for this but the wiring is incomplete.

### What is the architectural-debt question (Sprint C, ONE design decision)

The scattered-`match`-statement-across-shared-extractors pattern. Right
now every new language adds an arm to `complexity_extractor.rs`,
`receiver_detector.rs`, `symbol_extractor.rs` and a handful of other
files. Each of those `match`-on-`language` sites is a potential
regression for every existing language when a new one is added. Apex
raises the cost of getting this wrong because the Apex-specific logic
is substantially more elaborate (trigger context variables, managed
package namespaces, annotations as entry points) than the current
"Java-ish-but-different" arms we have.

**Recommendation: do the refactor, but in a bounded way.** Introduce a
`LanguageSpecificExtractor` trait with one implementation per language,
registered in `loader.rs`. Move Apex-specific logic into its impl;
leave other languages as a trivial `GenericExtractor` until they need
specialization. Result: Apex complexity is isolated, existing
languages are unchanged, the pattern is set for future
specialization without forcing it.

This is NOT a rewrite. It is a ~400–600-line refactor that reduces
cross-file coupling and prevents Apex from turning the shared
extractors into a 9-language switch statement. Estimated effort is
2–3 days, testable incrementally.

### What is not worth doing now

- Rebuilding a "perfect" custom parser. You already said no, and I
  agree. apex-jorje is good enough that the marginal accuracy gain
  from a custom parser is not worth the 6-month effort.
- Phase 2 (org-connected / Tooling API). Still deferred per the
  approved plan.
- Deep refactoring of `SimpleLspClient` itself. The camelCase fix
  closes the real bug; further refactoring should wait for concrete
  pain to justify it.

---

## 2. Sequenced plan — four sprints

Each sprint is sized for ~1 week of focused work. Every sprint ends
with the full test suite green and a short `docs/01-status/CURRENT_STATE.md`
update showing what changed.

### Sprint A — Close the verification loop and clear pre-existing debt

Goal: prove the foundation works end-to-end, not just in theory.

1. **Provision a machine with Java 17 and `apex-jorje-lsp.jar`.** Use
   Eclipse Temurin 17 (matches the planned desktop bundle). Verify
   the jar's SHA and source (Salesforce publishes a canonical build
   in `@salesforce/apex-ls-node`; mirror that).
2. **Run `apex_lsp_smoke`.** Expected: green. If it fails, that is a
   real finding and Sprint A pauses to fix it. Likely failure modes:
   - Java major-version mismatch (apex-jorje needs 11+; 17 is safest).
   - `clientInfo` shape differs from apex-jorje's expectation.
   - `capabilities` object too minimal (unlikely but possible).
3. **Record smoke-test results in `docs/01-status/CURRENT_STATE.md`** with the
   exact JRE/jar versions used.
4. **Check whether the camelCase fix restores
   `test_rust_lsp_cross_file_call_edges`.** If yes, great — remove the
   pre-existing-failure note. If no, file a focused ticket.
5. **Fix `tests/typescript_lsp_tests.rs`.** Two missing methods
   (`ensure_initialized`, `shutdown`). Either add them to
   `SessionSupervisor` or rewrite the test to use `initialize`.
6. **Fix `tests/infrastructure/benches/infrastructure_bench.rs`.**
   Missing `use graphengine_parsing::application::SyntaxExtractor`.

Exit criteria: `cargo test --workspace --tests --lib --features
lsp-tests` green when apex-jorje + rust-analyzer + tsserver are on the
host.

### Sprint B — Finish remaining Apex feature work

Goal: land the seven pending todos from the approved plan. Ordered by
dependency.

1. **`complexity_receiver`** — Add `"apex"` arms to
   `complexity_extractor.rs` and `receiver_detector.rs`.
   **But do it via the `LanguageSpecificExtractor` trait from Sprint C
   if Sprint C lands first.** This task's shape depends on whether
   Sprint C went first.
2. **`test_detector`** — `apex_test_detector.rs` for `@IsTest`
   annotations. Wire into `symbol_extractor` and
   `trait_context_detector`. Apex-specific, but the wiring points are
   the same shared extractors as above — same architectural question.
3. **`annotation_coverage`** — Full annotation entry-point set:
   `@AuraEnabled`, `@InvocableMethod`, `@HttpGet/Post/Put/Delete/Patch`,
   `@RestResource`, `global`/`webservice`,
   `@Future`, `Schedulable`/`Batchable`/`Queueable.execute`. Each
   flags a method as an entry point. These change the blast-radius
   calculation (entry points are sinks, not sources, in the "how much
   depends on me" direction).
4. **`managed_package_ns`** — Detect managed package namespace
   prefixes. Emit virtual external nodes grouped by namespace. This is
   coupling-to-external telemetry and a major demo talking point.
5. **`sharing_model_metadata`** — Capture `with sharing`, `without
   sharing`, `inherited sharing` as node metadata. Surfaces in
   security-review findings.
6. **`parser_tests`** — `apex_config_loading`, `apex_query_validation`,
   `apex_lsp_tests`, `apex_heuristic_tests` + corpus fixtures.
7. **`e2e_validation`** — Run against `dreamhouse-lwc` (small) and
   NPSP (large). Record scores in `docs/01-status/CURRENT_STATE.md`. Validate
   ≥95% LSP recall on the committed Apex files in NPSP. Any drop
   below 90% is a finding and pauses Sprint B.

Exit criteria: all Apex todos COMPLETED, NPSP produces a graph,
`docs/01-status/CURRENT_STATE.md` has real recall numbers.

### Sprint B.7 results + remediation block (2026-04-16)

B.1–B.7 landed. Baselines are committed at `tests/fixtures/apex_baseline/dreamhouse-lwc.json`
and `tests/fixtures/apex_baseline/NPSP.json` (heuristic-only; LSP intentionally off so these
represent the fallback floor, not the ceiling). Evidence is captured in
`docs/01-status/CURRENT_STATE.md` §Sprint B. Scanner: `graphengine-parsing/src/bin/apex_baseline.rs`.

| Repo | Nodes | Edges | Classes | Functions | Triggers | Elapsed |
|---|---:|---:|---:|---:|---:|---:|
| dreamhouse-lwc | 69 | 96 | 13 | 23 | 0 | 146 ms |
| NPSP | 17 468 | 706 183 | 1 658 | 12 351 | 65 | 22.5 s |

Apex-specific signal on NPSP is healthy (1 trigger per SObject, sharing distribution
surfaces 417 `omitted` = security-ambiguity signal, entry-point histogram exposes the
external API surface cleanly). But three concrete defects surfaced that the corpus
tests could not hit. These **must** land before Sprint D — D.2's value proposition
depends on P1.

#### B.7-P0 — Wire managed-package detection into the real pipeline

**Finding.** `graphengine-parsing/src/syntax/language/apex/managed_packages.rs` is
fully implemented, unit-tested, and re-exported. `apex_baseline.rs` is ready to
aggregate the output. But **no call site in `syntax/` or `application/use_cases/`
invokes `extract`** — the virtual `Module` nodes and `Import` edges are never
synthesized.

**Evidence.** NPSP's own `.cls` files reference `npe01__`, `npo02__`, `npsp__`
10+ times (verified directly in `force-app/test/HouseholdTests_TEST.cls`,
`LegacyHouseholdMembers_TEST.cls`, `HouseholdService_TEST.cls`). The baseline
reports `managed_package_consumers: {}`. The primitives shipped, the orchestration
did not. My prior summary claimed B.4 complete; it was not.

**Fix.** Add a call to `managed_packages::extract` inside the Apex branch of
`TreeSitterExtractor::extract` (or `ApexExtractor::extract_with_tree`) per file.
Feed the resulting `ManagedReferenceSite`s into a per-namespace inventory. On graph
assembly (in `ContainmentBuilder` or a new `apex_external_modules::build` pass),
synthesize one virtual `Module` node per namespace via `synthesize_module_node` and
one `Import` edge per unique `(consumer_file, namespace)` pair via
`synthesize_import_edge`. Re-run `apex_baseline` against NPSP and assert
`managed_package_consumers` is non-empty with at least `npe01`, `npo02`, `npsp`.

**Acceptance.** New integration test at
`graphengine-parsing/tests/apex_managed_packages_e2e.rs`: point the pipeline at
`tests/apex/force-app/.../ManagedPackageConsumer.cls` (already in the corpus) and
assert a virtual Module node + at least one Import edge materialize in the final
graph. NPSP baseline regenerated and committed.

Estimated effort: 0.5–1 day.

#### B.7-P1 — Stop clobbering confidence in graph assembly

**Finding.** `application/use_cases/parse_repo/pipeline/graph_building.rs:50,60`
unconditionally overwrites every node's confidence to `Medium` and every edge's
to `High`, ignoring whatever the extractor or resolver set.

```rust
// line 50
node.provenance.confidence = Confidence::Medium; // Syntax extraction is medium confidence
// line 60
edge.provenance.confidence = Confidence::High;   // Semantic resolution is high confidence
```

**Impact.** Destroys the Medium-vs-Low tiering `ApexHeuristicResolver` carefully
emits for unambiguous-match vs ambiguous-overload. NPSP baseline shows all 706 183
edges as `High` even though 688 716 came through the heuristic tier. This is
**cross-language pre-existing behaviour** (not Sprint B's doing) — but it directly
blocks **Sprint D.2**, whose `ResolutionDegraded` finding is specified to trigger on
"fallback rate OR elevated share of Low-confidence edges", a signal this clobber
erases.

**Fix.** Remove both overrides. Trust the provenance the extractor/resolver set.
Audit the cross-language call sites that emit nodes/edges without explicit
confidence and fix them at their source if they were relying on the clobber.

Known upstream confidence-emitting sites (verified via grep):
- `resolution/fallback.rs` — sets `Confidence::Low` / `Medium` explicitly. ✓
- `infrastructure/lsp/real_resolver.rs`, `call_resolver.rs`, `resolvers/*` — set
  `Confidence::High` on LSP-backed edges. ✓
- `syntax/language/apex/managed_packages.rs` — sets `Confidence::High` on synthesized
  external-module edges. ✓
- `apex/heuristic_resolver.rs` — sets `Confidence::Medium` (unique) or `Low`
  (ambiguous). ✓

Containment edges and node creation paths must be audited; any that emitted
uninitialized `Confidence::Low` and relied on the clobber to upgrade to Medium/High
need to be fixed at source.

**Acceptance.** New regression test at
`graphengine-parsing/tests/graph_builder_confidence_preservation.rs`: feed a
hand-built `ResolvedEdges` containing one Heuristic/Low edge, one Heuristic/Medium
edge, and one Lsp/High edge through `GraphBuilder::build_from_results`; assert all
three confidences survive unchanged. NPSP baseline re-run shows
`edges_by_confidence` distributed across High/Medium/Low, not collapsed to High.

Estimated effort: 1 day (the override fix itself is a 2-line change; the audit and
test coverage is what consumes the time).

#### B.7-P2 — Decide on short-name fanout cap (contingent on P1)

**Finding.** NPSP has ~56 call edges per function. `ApexHeuristicResolver`
emits one edge to **every** candidate with a matching short name. On a codebase
where common method names like `save`, `execute`, `initialize`, `getName` exist
on dozens of classes, each call site becomes a fan-out explosion.

**Current mitigation.** Each such edge is correctly tagged `Confidence::Low`. The
signal is preserved — users can filter or down-weight Low-confidence edges in
reports. P1's fix makes this actionable.

**Decision gate.** After P1 lands, regenerate the NPSP baseline with confidence
preserved. If reports and analyses that filter to `Confidence::Medium` or higher
produce reasonable fan-in/blast-radius numbers, P2 is closed with no code change.
If Low-confidence edges still bleed into top-level metrics, choose one of:

- **Cap fanout at N candidates** (e.g. drop if >8 candidates matched; emit a single
  "ambiguous" edge with Low confidence to a synthetic `ambiguous::<name>` node).
- **Narrow by receiver-type hints** when available from call-site capture.

Estimated effort: 0.5 day (decision + optional 1 day if a cap or narrowing is chosen).

#### Sequencing

1. **B.7-P1 first** (confidence clobber). Cheapest. Unblocks honest measurement of
   everything downstream.
2. **B.7-P0 second** (managed-package wiring). Observable regression in the NPSP
   baseline proves the fix.
3. **B.7-P2 decision gate** using the regenerated NPSP baseline.
4. **Then Sprint D** with a realistic confidence distribution in the dataset.

---

### B.7-P0 / P1 / P2 — resolution (2026-04-16)

All three remediations landed. Evidence is the final regenerated baselines at
`tests/fixtures/apex_baseline/dreamhouse-lwc.json` and `tests/fixtures/apex_baseline/NPSP.json`.

**B.7-P1 — Confidence clobber: fixed.**
`graphengine-parsing/src/application/use_cases/parse_repo/pipeline/graph_building.rs`
no longer overwrites node or edge confidence. `validate_graph` is invoked with
`Confidence::Low` as the structural minimum so legitimate Low-tier heuristic edges
are not rejected by the pipeline; `min_confidence` becomes an advisory
filter for downstream consumers. Regression pinned by
`graphengine-parsing/tests/graph_builder_confidence_preservation.rs` (3 tests
covering High/Medium/Low for both nodes and edges).

**B.7-P0 — Managed-package wiring: fixed.**
Four modules changed:

- `application/ports.rs` — `SyntaxResults` gains `synthesized_edges: Vec<Edge>`
  plus an `add_synthesized_edge` helper so language extractors can inject
  deterministic edges that sit outside the semantic-resolver contract.
- `syntax/language/extractor.rs` — `LanguageSpecificExtractor` trait gains
  `synthesize_external_references(tree, source, file, consumer_id) →
  ExternalReferenceResult` with a default no-op impl. Other languages are
  unaffected.
- `syntax/language/apex/extractor.rs` — Apex impl deduplicates namespaces per
  file and emits one virtual `Module` node + one `Import` edge per unique
  namespace-per-consumer via `managed_packages::synthesize_module_node` /
  `synthesize_import_edge`.
- `syntax/treesitter.rs` — `parse_file` invokes the hook after the file-module
  is created; `extract` merges `synthesized_edges` across parallel file parses.
- `application/use_cases/parse_repo/pipeline/graph_building.rs` — new
  `add_synthesized_edges` step.

The first pass detected far too much (3 291 pseudo-namespaces on NPSP, dominated
by identifiers like `a`, `acc`, `this`, `response`, `result`). Root cause was that
`managed_packages::extract` walked `field_access` and `method_invocation` nodes
and ran a `qualified_left_segment_namespace` fallback on any dotted token,
which fires for every local variable receiver (`this.x`, `acc.Name`,
`response.body`). Two surgical fixes:

1. `managed_packages::extract` restricted to `identifier`, `type_identifier`,
   `scoped_type_identifier`, `annotation`. The `__`-marker rule still fires on
   all of those; the bare-dotted fallback fires only on `scoped_type_identifier`
   and `annotation` (the Apex grammar emits those kinds only in type contexts).
2. `class_registry::extract_managed_namespace` now enforces the actual Salesforce
   managed-package spec on the namespace shape — 1–15 ASCII alphanumerics, no
   underscores, must start with a lowercase letter or contain a digit. Also
   expanded the custom-entity suffix marker set to cover `__r` (relationship),
   `__s` (geolocation sub-field), `__pc` (person-account), `__History`,
   `__Share`, `__Feed`, `__Tag`, `__ChangeEvent`.

Result on NPSP: **5 real managed packages** (`npe01`: 205 consumers, `npe03`:
169, `npo02`: 105, `npe4`: 38, `npe5`: 21) with zero false positives. On
dreamhouse-lwc: 0 namespaces (correct — it uses none). Behaviour pinned by:

- New `managed_packages::tests::pure_method_chain_without_type_declaration_is_not_detected`
  and `sobject_relationship_traversal_on_local_var_is_not_detected` — document
  the intentional gap and prevent regression of the NPSP false-positive class.
- New `managed_packages::tests::detects_namespaces_through_type_declaration_before_method_use`
  — proves realistic managed-package usage (declare a typed variable, then
  invoke methods on it) is still detected.
- Expanded `class_registry::extract_managed_namespace_handles_edge_cases` with
  every NPSP-observed false-positive shape.
- New integration test
  `graphengine-parsing/tests/apex_managed_packages_e2e.rs` (2 cases).

Intentional honest gap: pure expression-chain managed-package calls
(`fflib.Application.UnitOfWork.commitWork()` with no prior declaration) are
*not* detected. They are syntactically indistinguishable from SObject
relationship traversal (`opp.Account.Owner.Name.toString()`) without type
information. LSP covers this.

**B.7-P2 — Fanout cap: chose cap over receiver-narrowing.**

After P1 made the Low-confidence tier honestly observable, the regenerated NPSP
baseline showed 591 084 Low-confidence edges (~90% of all edges) produced by
`ApexHeuristicResolver` emitting one edge per candidate when a short-name match
returned N results. With `strip_prefix_and_receiver` discarding receiver info,
the resolver has no way to pick which of 30 classes' `save()` the call means.

Chosen remedy: **hard cap at 8 candidates**
(`HEURISTIC_CALL_FANOUT_CAP = 8`, documented in-module). Over-cap sites emit
**zero** edges — emitting the 8 alphabetically-first picks would be false
precision. Each dropped site is recorded in
`ResolutionStatsSummary::heuristic_call_ambiguous_drops` so Sprint D.2's
`ResolutionDegraded` finding can quantify recoverable signal. Receiver-type
narrowing was deferred: it properly belongs in the LSP path (which has real
type info) and retrofitting it into the heuristic would be substantial work
for modest accuracy gains over the cap.

Effect on NPSP baseline:

| Metric | Before P2 | After P2 |
|---|---:|---:|
| Total edges | 654 880 | 338 491 |
| Low-confidence edges | 591 084 | 274 695 |
| Medium-confidence edges | 45 755 | 45 755 (unchanged — cap is Low-only) |
| High-confidence edges | 18 041 | 18 041 (unchanged) |
| `heuristic_call_ambiguous_drops` | n/a | 19 260 |

The cap removed ~316K low-value heuristic edges while preserving every
Medium+ edge — exactly the surgical outcome. Regression coverage:
`over_cap_fanout_emits_zero_edges_and_records_drop`,
`exactly_at_cap_still_emits_edges`,
`ambiguous_overload_emits_all_candidates_as_low_confidence` (extended to
assert no drops when below cap).

With these three remediations, Sprint D can proceed against a dataset with a
realistic confidence distribution and a clean managed-package inventory.

### Sprint C — Extractor architecture refactor (the ONE architectural decision)

Goal: isolate per-language logic so adding a 10th language does not
mutate 6 shared files.

1. **Design.** New trait `LanguageSpecificExtractor` with hooks for
   complexity, receiver detection, symbol annotation, trait/interface
   context, test detection. Generic default implementation covers
   languages without specialization. Apex gets its own impl.
2. **Register.** `loader.rs` returns a `LanguageSpecificExtractor` via
   `Box<dyn ...>` alongside the existing grammar + config.
3. **Migrate Apex first.** Move all Apex arms from
   `complexity_extractor`, `receiver_detector`, `symbol_extractor`,
   `trait_context_detector` into a new `apex::ApexExtractor` struct.
   Keep existing language arms untouched.
4. **Prove no regression.** Full test suite stays green. No other
   language is migrated in this sprint — they stay on the existing
   match-arm pattern and get migrated one-at-a-time in subsequent
   work only when a concrete change forces specialization.

Exit criteria: Apex's per-language logic lives in one module.
`complexity_extractor.rs` and `receiver_detector.rs` no longer have
`"apex" =>` arms. All tests green.

Risk: low. This is a mechanical refactor with tight test coverage on
the paths being moved. Estimated effort 2–3 days.

**Sequencing note.** Sprint C *can* run in parallel with Sprint B if
two people are available. Solo: run Sprint C *before* the
`complexity_receiver`, `test_detector`, and `annotation_coverage`
items in Sprint B, because those items are what would otherwise add
new match arms. Doing Sprint C first means adding Apex logic directly
in the new pattern instead of rewriting it later.

### Sprint D — LSP robustness

Goal: make apex-jorje failures visible and recoverable, not invisible
and catastrophic.

1. **Retry/restart on crash.** `SessionSupervisor` has
   `SessionState::Failed` but the restart wiring is incomplete.
   Implement bounded-retry with exponential backoff, capped at 3 by
   default (the field is already there).
2. **Diagnostic surfacing.** When LSP falls back to heuristic,
   emit a structured `ResolutionDegraded` event that the CLI and UI
   can display. Users must see when they are not getting the
   top-tier resolution they think they are paying for.
3. **Health check.** Periodic no-op request (spec allows
   `$/cancelRequest` on a bogus id for a cheap liveness probe, or we
   can use `workspace/configuration`). If no response in 3 intervals,
   mark session `Degraded`.
4. **Telemetry.** Expose `SessionMetrics` and
   `ResolutionStatsSummary` via the CLI as a machine-readable JSON
   block at end of scan. Enables post-hoc audit of "did LSP actually
   do the work?" — critical for the desktop pilot where customers
   may silently degrade without telling us.

Exit criteria: a forced kill of the apex-jorje process mid-scan does
not corrupt the scan and produces a clear "LSP crashed, fell back to
heuristic on N files" report.

---

## 3. Size estimate (honest)

| Sprint | Solo effort | Risk |
| ------ | ----------- | ---- |
| A | 2–3 days | Low — well-scoped fixes |
| B | 4–5 days | Medium — `e2e_validation` may surface real accuracy issues |
| C | 2–3 days | Low — mechanical refactor |
| D | 3–4 days | Medium — LSP lifecycle edge cases are subtle |
| **Total** | **11–15 days** | — |

Previous estimate was 10–13 days. The delta comes from adding the two
pre-existing-debt items to Sprint A. No new work was invented; the
existing debt just became visible during verification.

---

## 4. Answers to the direct questions

> How much work is there to solve all the issues you pointed out?

11–15 days solo, sequenced as four sprints. The camelCase protocol bug
is already fixed in this verification pass — that was the largest
unknown and it is closed.

> What are the most complete solutions?

1. The camelCase protocol fix is complete and unit-tested. No further
   work needed there.
2. For the extractor architecture: the `LanguageSpecificExtractor`
   trait is the complete solution. Half-measures (adding Apex match
   arms and pretending the pattern will hold) have a short shelf-life.
3. For LSP robustness: wire up the state machinery that already
   exists in `SessionSupervisor`. No new abstractions needed.
4. For verification: the smoke-test harness is committed.
   Provisioning Java and the jar is the only remaining step.

> Sticking with LSP (not rapid custom parser), what does that imply?

- Java is a runtime dependency for Apex users. Desktop must bundle
  Temurin 17; CLI docs must mention it. No way around this.
- LSP startup cost (~3–5 seconds on apex-jorje) is real. The scanner
  must amortize it by scanning the whole repo in one LSP session.
- apex-jorje is an opaque binary from Salesforce. If Salesforce
  changes its LSP dialect, we have to track it. Mitigation: pin the
  jar version in the desktop build; smoke test verifies the pinned
  version works.
- Heuristic-only scans are **possible but degraded**. The
  dispatcher already downgrades gracefully. The pilot's value prop
  depends on LSP being the default path, so documentation must be
  clear about what users lose when LSP is unavailable.

> Am I overthinking?

You are slightly under-thinking about verification and exactly
right-thinking about architecture. The sprint plan above reflects
both: Sprint A forces verification to happen before anything else;
Sprint C confronts the architectural debt now rather than letting it
compound.

> New plan? Just how in depth will this go? Simple fixes? In depth?

This is the new plan. It is specific enough to execute and brief
enough to read. Simple fixes (Sprints A and D) and architecture
(Sprint C) sit alongside the remaining feature work (Sprint B).

---

## 5. What I recommend you do next

1. Approve this plan (or adjust sprint sequencing).
2. Decide: Sprint C first (extractor refactor) vs Sprint B first
   (feature completion)? Recommendation: Sprint A (verification)
   → Sprint C (refactor) → Sprint B (features use the new pattern) →
   Sprint D (robustness).
3. Provision the verification machine (Temurin 17 + apex-jorje-lsp.jar).
   This is the one external dependency the plan needs.

---

## Workstream B: Apex Framework Resolver (new, post-hand-audit)

Surfaced by the Wave 1 Layer-5 hand-audit
(`docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`). Seven of ten sampled NPSP `no_callers`
FQNs have real callers in source that the Apex heuristic resolver
failed to link — basic `new X(...)` constructor calls, typed-field
dispatch, intra-class overload dispatch, and Visualforce page
extensions. The heuristic resolver cannot be patched incrementally
past these classes of misses without the `_CTRL`-style one-off
whitelists we explicitly ruled out of Wave 1.

**Full design plan landed at `docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md`**
(Wave 3.1 of the truthful-scans simplification plan). The plan
contains:

- Enumerated backlog (152 + 2 + 17 FQNs) against NPSP rev 5 A/B,
  clustered by dispatch idiom.
- 22-row dispatch matrix covering every observed idiom → resolver
  type (authoritative / synthetic / declarative) → `EdgeSource`
  variant → on-disk evidence source.
- Pre-registered acceptance gates per cluster (Batchable: −72,
  SCHED: −26, Queueable: −14, trigger body: −26, …).
- North-star edge-provenance architecture: every edge carries
  `(source, confidence, evidence)` so `DeadCodeReason` becomes
  a derived threshold rather than a hand-coded enum.

Original (pre-plan) summary of scope — now superseded by the
linked plan but preserved for history:

- Cross-class inheritance-aware interface method propagation
  (covers `CRLP_Batch_Base_NonSkew.start(...)` subclassed by
  `CRLP_Account_BATCH implements Database.Batchable`).
- Name-resolution pass for `new X(...)` and `field.method(...)`
  consulting `class_registry` consistently (R23).
- Visualforce page parsing: `<apex:page extensions="X">` links
  every `{!methodName}` in the page to `X.methodName` in Apex.
- Flow / Process Builder / Platform-Event handler linkages
  (declarative wiring).
- Declarative rule engine so new Salesforce platform dispatch
  idioms can be added without touching
  `dead_code_classifier/apex.rs` (R26).

**Gating.** The `no_callers` hand-audit re-runs on rev 6 (post-Apex
Framework Resolver) must show < 2 / 10 wrong before the resolver
is considered shipped. The 7 rev 4 + 10 rev 5 failed samples
(`docs/workstreams/apex/FRAMEWORK_RESOLVER_PLAN.md` §4.11 + §8.3) are its
first regression fixtures.
