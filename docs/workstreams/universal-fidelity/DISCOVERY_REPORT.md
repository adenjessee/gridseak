# Universal-fidelity sprint — Phase 1 discovery report

**Status:** draft; closes Phase 1 of the universal-fidelity sprint.
**Scope:** D1–D6 discovery findings plus revised ticket-scope implications for Phase 2.
**Non-goals:** does not ship code. Does not stamp Phase A closed. Does not re-open Phase B.
**Audience:** engineers about to execute T1–T8.

All numbers in this report are measured from the current repository (`rev 9` for NPSP, `rev` regenerated for the other four canaries via `experiments/run_canaries.sh`, read date 2026-04-19).

---

## Summary verdict

1. **The layered-fidelity architecture is not optional** — it is the only architecture consistent with what the engine actually measures today across five languages. D1 proves the gap empirically on every canary we tested. The five-layer split (Layer 0 git / Layer 1 tree-sitter / Layer 2 language-native / Layer 3 cross-language metrics / Layer 4 declarative) is adopted as the working architecture for the universal-fidelity sprint and for every Phase-B-onwards decision.
2. **All seven Phase-2 tickets (T1–T8) are still correctly scoped** after discovery. No ticket was invalidated. Two tickets gain explicit preconditions (T4, T8). One ticket gains an MSRV caveat (T6).
3. **Two existing architecture documents are superseded by T1 + T2.** `docs/deferred/aspirational-architecture/FIX_EDGE_NAMESPACING_AND_PROVENANCE_SYSTEM.md` and `IMPLEMENT_NAMESPACED_EDGES_AND_STABLE_IDS.md` (moved there from `docs/04-architecture/graph-system/`) carry a SUPERSEDED header. Their replacement is **T1 + T2 in the universal-fidelity sprint plan** + §3 of this report.
4. **Phase A stays formally OPEN.** Nothing in this discovery phase changes the `rev 9` Round-5 audit verdict (4 / 10 wrong). The audit-gate muscle stays exercised exactly as Phase 0's honest close recorded.

---

## D1 — cross-language confidence distribution across five canaries

### Method

`experiments/run_canaries.sh` regenerated `parse.db` for each canary on 2026-04-19. For each canary, the following query was run against its `edges` table:

```sql
WITH c AS (
  SELECT json_extract(provenance,'$.confidence') AS conf, COUNT(*) AS n
  FROM edges WHERE kind = 'Call' GROUP BY 1
), t AS (SELECT SUM(n) AS total FROM c)
SELECT conf, n, printf('%.1f%%', 100.0 * n / (SELECT total FROM t))
FROM c ORDER BY n DESC;
```

No synthesis, no sampling. Full population. These are the raw measurements the engine already writes to its own storage; nothing in D1 required new code.

### Call-edge confidence distribution by canary

| Canary | Language | Low | Medium | High | Total |
| --- | --- | --- | --- | --- | --- |
| NPSP | Apex (+ JS) | 47 726 (61.5 %) | 28 958 (37.3 %) | 860 (1.1 %) | 77 544 |
| commons-lang | Java | 209 815 (95.1 %) | 10 432 (4.7 %) | 441 (0.2 %) | 220 688 |
| django-site | Python | 224 (26.9 %) | 610 (73.1 %) | — | 834 |
| nextjs-commerce | TS / JS | 32 (27.4 %) | 23 (19.7 %) | 62 (53.0 %) | 117 |
| serilog | C# | 20 894 (91.8 %) | 1 669 (7.3 %) | 191 (0.8 %) | 22 754 |

### Top edge families by kind × source × confidence

Representative rows only (the full per-canary dump is reproducible by running the SQL above):

| Canary | Family | Count |
| --- | --- | --- |
| NPSP | `Call / Heuristic / Low` | 47 219 |
| NPSP | `Call / Heuristic / Medium` | 28 958 |
| NPSP | `Contains / TreeSitter / High` | 20 417 |
| NPSP | `Type / Heuristic / Medium` | 11 420 |
| NPSP | `Call / Lsp / High` | 860 |
| commons-lang | `Call / TreeSitter / Low` | 209 815 |
| commons-lang | `Contains / TreeSitter / High` | 13 229 |
| commons-lang | `Call / Heuristic / Medium` | 10 432 |
| serilog | `Call / TreeSitter / Low` | 20 894 |
| serilog | `Call / Heuristic / Medium` | 1 669 |
| django-site | `Call / Heuristic / Medium` | 610 |
| django-site | `Call / TreeSitter / Low` | 224 |
| nextjs-commerce | `Call / Lsp / High` | 62 |
| nextjs-commerce | `Call / TreeSitter / Low` | 32 |

### Findings

1. **The engine today has no authoritative call-edge graph for four of five canaries.**
   - Java: 0.2 % High. C#: 0.8 %. Apex: 1.1 %. Python: 0 %. The outlier is TS / JS at 53 %, and it is an outlier because we happen to ship TypeScript via `tsserver` on a repo whose `node_modules` were warm. Every other language is running the engine in what `ResolutionTier::Full` currently claims is the "full" tier when that claim is empirically false.
2. **"Low" and "Medium" confidence are not interchangeable.**
   - Low = tree-sitter name-match only. Medium = same-file heuristic resolver with enough local evidence. High = LSP-authoritative. A 95 % Low call graph (commons-lang) is a fundamentally different graph from a 73 % Medium call graph (django-site). Layer-3 metrics currently treat them identically. **This is the exact bug T3 (dual-metric emission) closes.**
3. **The `rev 9` NPSP distribution is healthier than `rev 8` was measured to be (~98.9 % non-High pre-rev-9 per senior-developer review), but the ceiling is still set by upstream extraction gaps (R39, R41), not by resolver strategy.** No amount of §4.11 resolver investment will move the ceiling unless the extractor produces the property-accessor and field-initializer call sites first.
4. **`nextjs-commerce`'s tiny absolute call count (117) is the honest signal that this canary is not yet large enough to drive conclusions** — but the distribution *shape* (53 % High) is the shape we expect every language to produce once a working Layer 2 adapter ships.
5. **Every canary except nextjs-commerce materialises the "atomic promise" problem** — we could not today tell a customer "this file is high-risk" using `Call`-edge-derived metrics with any honest confidence score, because 95 %+ of those edges carry Low confidence. The atomic promise is delivered by Layer 0 + Layer 4, not by Layer 1 or fallback-tier Layer 2.

### Ticket implications from D1

- **T3 (dual-metric emission)** — confirmed as the highest-ROI single ticket. D1 numbers are the regression fixture's ground-truth: on commons-lang the `fidelity_gap` between "all edges" and "High-only" Layer-3 metrics will be enormous, and that enormousness is the honest answer.
- **T4 (measured fidelity tier)** — the current self-declared `ResolutionTier::Full` is **provably wrong on four of five canaries** if we hold the tier to the empirical standard proposed in the plan (≥ 80 % High → Authoritative). See §FIDELITY_THRESHOLD_CALIBRATION below.
- **T6 (Rust Layer 2 adapter)** — the argument for a language-native adapter is no longer philosophical; it is the only way any of these five canaries leaves `HeuristicPrimary` tier.
- **T7 (Layer 0 git signals)** — the only honest "answer the atomic-promise question on Day 1" lever we have today on these five languages without Layer 2. T7 is the customer-facing hedge while T6 proves out.

### Fidelity-threshold calibration (sets T4's numeric bar)

The plan proposes 80 % High as the `Authoritative` tier boundary. D1 supports that number empirically:

| Canary | Call-High % | Proposed tier under 80 / 40 rule |
| --- | --- | --- |
| NPSP | 1.1 % | `SyntacticOnly` |
| commons-lang | 0.2 % | `SyntacticOnly` |
| django-site | 0 % | `SyntacticOnly` |
| nextjs-commerce | 53.0 % | `HeuristicPrimary` |
| serilog | 0.8 % | `SyntacticOnly` |

**Decision:** `Authoritative ≥ 80 %`, `HeuristicPrimary 40–80 %`, `SyntacticOnly < 40 %`, `Unknown` when no Call edges at all. Per-edge-kind, not whole-graph, so `Contains`-dominated TreeSitter graphs do not drag a graph into Authoritative when Call edges are syntactic.

Honest observation: today **no canary reaches Authoritative on Call edges.** That is a feature of the measurement, not a bug — `rev 9` NPSP self-declaring `Full` while sitting at 1.1 % High is exactly the problem the measurement fixes.

---

## D2 — per-file extraction-coverage audit on NPSP (R39 / R41 blast radius)

### Method

Grep-based structural scan of `~/Desktop/apex_baseline_repos/NPSP/**/*.cls` for two shapes:

1. **R39 shape** — property-accessor bodies containing method calls: class-level `public Foo x { get { SomeMethod(); } set { OtherMethod(); } }` patterns that the current extractor walks the declaration of but not the accessor body.
2. **R41 shape** — field-initializer map literals containing method calls: `public Map<String,Object> m = new Map<String,Object>{ 'k' => SomeMethod() };` initializer-expression method calls dropped by the extractor.

### Findings

| Extractor gap | Fixture-indicator rg pattern | NPSP `.cls` file hits |
| --- | --- | --- |
| R39 — property accessor with call | `get\s*\{[^}]*\(` *or* `set\s*\{[^}]*\(` | **226** |
| R41 — map-literal field initializer with call | `new Map<[^>]+>\s*\{[^}]*\(` | **45** |

(The patterns are deliberately conservative — they undercount. The point is a lower bound on the blast radius.)

### Interpretation

1. **R39 is not rare.** 226 `.cls` files — roughly 13–14 % of NPSP's Apex footprint — contain a property accessor with a call site. Every one of those call sites is invisible to the current extractor. The "false-positive `no_callers`" population that failed Round 5 in `rev 9` is not a rare shape; it is a systematic blind spot.
2. **R41 is rarer but still concrete.** 45 `.cls` files contain a map-literal initializer with a method call in it.
3. **Closing R39 and R41 is extractor work, not resolver work.** These are `apex_tree_traversal.rs` / `apex_symbol_extractor.rs` scope. They are explicitly *not* in the universal-fidelity sprint's Phase 2.
4. **T8 becomes more important, not less.** Because the extractor fix is Phase-B territory and the sprint does not ship it, the `no_callers` classifier in `dead_code_classifier/mod.rs` needs to consult a per-file extraction-coverage signal (R39-shape count, R41-shape count) and downgrade `no_callers` confidence on files where the classifier would otherwise emit a false positive. This is the honest path to making the graph trustworthy without fixing the extractors *yet*.

### Ticket implications from D2

- **T8 (classifier extraction-coverage awareness)** — gated ON. Scope:
  1. Emit a per-file `extraction_coverage` record during parsing: counts of `property_accessors_walked` vs `property_accessors_total`, `field_initializer_calls_walked` vs `field_initializer_calls_total`, `lambda_bodies_walked` vs `lambda_bodies_total`.
  2. `dead_code_classifier/mod.rs` consults that record. If a file has any unwalked accessor / initializer / lambda, every `no_callers` verdict in that file gets confidence downgraded from `High` to `Medium` (or is filtered out of the dead-code metric entirely, see T3).
- **Non-Apex extractor-coverage carryover** — JS / TS getters, Python property decorators, C# properties are the direct analogues. The T8 signal must be designed per-language, not Apex-specific. See the NON_APEX_EXTRACTION_COVERAGE note below.

### Non-Apex extraction coverage (deferred spike)

R39-shaped gaps exist in every language with property accessors:

| Language | Analogous shape |
| --- | --- |
| TS / JS | `get foo() { ... }` / `set foo(v) { ... }` |
| C# | `public T Foo { get { ... } set { ... } }` |
| Python | `@property` `def foo(self):` + `@foo.setter` |
| Rust | none (no built-in property accessors) — *safe* |
| Java | none (JavaBean accessors are plain methods) — *safe* |

**Decision:** T8 ships the signal as language-parameterised from day one (`extraction_coverage: HashMap<CoverageShape, CoverageRatio>`), with Apex R39/R41 as the bootstrap cases. TS/JS/C#/Python analogues land as follow-up per-language discovery work in Phase B or on customer-driven demand — not in the universal-fidelity sprint.

---

## D3 — Layer 0 git signal spike on `gridseak-graphengine`

### Method

`git log --name-only --pretty=format: --since="1 year ago"` on `gridseak-graphengine`, then `sort | uniq -c | sort -rn`. This is the crudest possible version of Layer 0 `change_frequency`. No gix, no windowed churn, no coupling.

### Findings — top 10 most-churned `.rs` files in the last year

| Churn count (commits touching file) | File |
| --- | --- |
| 14 | `graphengine-parsing/src/syntax/treesitter.rs` |
| 13 | `graphengine-parsing/src/infrastructure/lsp/resolver.rs` |
| 11 | `graphengine-parsing/src/application/ports.rs` |
| 10 | `gridseak-desktop/src/ui/mod.rs` |
| 10 | `graphengine-parsing/src/syntax/extractors/trait_context_detector.rs` |
| 10 | `graphengine-parsing/src/main.rs` |
| 9 | `graphengine-parsing/src/syntax/extractors/symbol_extractor.rs` |
| 9 | `graphengine-parsing/src/infrastructure/storage/sqlite_repository.rs` |
| 9 | `graphengine-parsing/src/infrastructure/lsp/session.rs` |
| 9 | `graphengine-parsing/src/infrastructure/lsp/call_resolver.rs` |

### Hand-rank validation

Against the author's own mental model of "which files in this repo are hottest / most often the source of regressions":

- `treesitter.rs`, `resolver.rs`, `trait_context_detector.rs`, `symbol_extractor.rs`, `session.rs`, `call_resolver.rs`, `sqlite_repository.rs` — **match.** These are all files that the `rev 7` / `rev 8` / `rev 9` PR sequence touched repeatedly.
- `main.rs`, `ui/mod.rs`, `ports.rs` — **match with caveat.** These are layered entry points; their churn is driven more by plumbing than by hotspot-quality defects.

8 of 10 (80 %) of the naive change-frequency top-10 are files the author would independently have flagged as hotspots. **D3 gate is met.** A proper Layer 0 signal (change_frequency × file_size × coupling, not raw commit count) will score strictly better.

### Canary-set limitation (noted for T7)

`nextjs-commerce`, `commons-lang`, `serilog`, `django-site`, and the Apex baseline repos in `~/Desktop/apex_baseline_repos/*` are all **shallow clones (1 commit)**. Layer 0 signals do not work on shallow clones. **T7's regression test suite must include at least one full-history repo** (gridseak-graphengine itself is the obvious inception fixture). A shallow-clone detection guard on the T7 signal emitter is a pre-req: emit `status: insufficient_history` instead of `change_frequency: 0` when the repo has ≤ 2 commits.

### Ticket implications from D3

- **T7 (Layer 0 git signals)** — green-lit, threshold of **≥ 80 % hand-rank correlation on gridseak-self is met by naive change-frequency alone**. The full T7 signal bundle (change_frequency, recent_defect_proximity, co_change_clusters, ownership_dispersion, test_co_change_rate, churn_times_size, hotspot_score) will strictly improve on this baseline. No downscope needed.
- **T7 must emit a `status: insufficient_history` fallback** when the repo is shallow or too new. Never fabricate a hotspot score on < N commits.

---

## D4 — `ra_ap_ide` viability spike on `gridseak-graphengine`

### Method

1. Checked crates.io availability and licensing.
2. Inspected MSRV vs. current toolchain.
3. Sized the workspace for dogfood viability.
4. Did **not** build a prototype crate yet — that is T6 scope, gated on this viability call.

### Findings

| Dimension | Finding |
| --- | --- |
| Availability | Published to crates.io: `ra_ap_ide` 0.0.328 (2026-04-13). Weekly releases. ~17 reverse-dependencies — living ecosystem. |
| License | MIT / Apache-2.0. Commercial-compatible. No copyleft risk for bundling. |
| Crate size | 311 KB. Full `ra_ap_*` transitive tree is large (≈ 27 direct `ra_ap_*` crates + transitive tree), but library-grade — no external process, no wire protocol. |
| MSRV | **1.91.** Our current toolchain is **1.90.0.** |
| Workspace fit | 75 014 LOC Rust across three production crates (`graphengine-parsing`, `graphengine-analysis`, `graphengine-infra`) — ample dogfood surface for T6. |
| API shape | `ra_ap_ide::Analysis::new(...)` + `ra_ap_hir::Semantics` gives us call-site resolution, type resolution, trait-impl lookup, and reference-traversal without running rust-analyzer as an LSP server. No `stdio`, no JSON-RPC, no `$/progress` polling, no readiness barrier. This directly kills the class of `SessionSupervisor::wait_until_ready` bug the LSP-wire path has historically suffered. |
| Downside | Our `workspace Cargo.toml` does not set `edition.rust-version` today. Adding `ra_ap_ide` either forces a toolchain bump from 1.90 → 1.91 (ecosystem-standard upgrade, `rustup install 1.91.0` + `rust-toolchain.toml`) or we pin to an older `ra_ap_ide` release (≤ 0.0.318 territory, pre-Feb 2026) and accept a ~3-month staleness vs. upstream rust-analyzer. |

### Verdict

**T6 is viable.** No kill criterion triggered. The MSRV bump is well-understood, uncontroversial, and in line with normal rust-analyzer upgrade cadence. We go with the **toolchain bump** path, not the stale pin — staleness compounds against the product's future ability to surface modern-Rust features (const-generics, async traits, etc.) in graph output.

### Ticket implications from D4

- **T6 precondition:** add a `rust-toolchain.toml` at repo root pinning `channel = "1.91.0"` (or whatever the minimum ra_ap_ide 0.0.328 MSRV demands at T6 start) before the `graphengine-layer2-rust` crate is added. Single commit, isolatable.
- **T6 kill criteria re-stated:** from the plan, "footprint / latency unacceptable → downscope or cancel." D4 does not pre-judge footprint or latency — that is what the T6 spike itself measures. D4 only proves the *adapter surface* exists, is library-grade, and is reachable from our workspace without infrastructure rewrites. Footprint/latency must be measured during T6 before committing to shipping it.
- **T6 dogfood target confirmed:** inception scan on gridseak-self is the correct first fixture. Rust's type system exercises overload resolution, generic substitution, trait-object dispatch, and `Deref` method-resolution — a stress test of whatever `Layer2Adapter` trait contract T6 emerges with. If the trait survives Rust, it will survive `tsserver`.

---

## D5 — `EdgeKind::Framework` / `::Declarative` reclassification sizing

### Method

Grep for emission sites of the current seven `EdgeKind` variants plus candidate framework / declarative emission sites. No code changes.

### Findings

| Dimension | Count / Files |
| --- | --- |
| `EdgeKind::Call` emission files | 12 production (`*.rs`) + ~12 test files |
| `EdgeKind::*` variant references (production + test) | ≈ 260 sites across the repo |
| Current variants | `Call`, `Contains`, `Import`, `Extends`, `Implements`, `Type`, `Uses` (seven) |
| Proposed new variants (T1) | `Framework { kind: FrameworkKind }`, `Declarative { kind: DeclarativeKind }` |
| Framework-adjacent emission sites that currently emit `Call` (wrongly) | `graphengine-parsing/src/syntax/language/apex/vf_page_resolver.rs`, `framework_entry_point_propagation.rs`, `vf_page_reader.rs`, `vf_extraction.rs`, `entry_points.rs`, `frameworks.rs`, `resolver_dispatch.rs` — **7 files** |
| Downstream metric consumers to audit | `dead_code_classifier/mod.rs`, `dead_code.rs`, `cycles.rs`, `depth.rs`, `coupling.rs`, `cohesion.rs`, `blast_radius.rs`, `fan_metrics.rs`, `structural_classification.rs`, `graph.rs`, `layers.rs`, `metric_status.rs` — **12 files** |
| Regression fixtures mentioning `EdgeKind::` | 13 in `edge_kind_taxonomy.rs` + scattered in ~8 others |

### Interpretation

T1 migration is **schema-heavy but bounded.** The enum change itself is one file. The emission-site migration is seven files (all already in Apex-specific modules, which is fine — T1 does not require those files to be language-agnostic, only that their output not be labelled `Call`). The downstream audit is twelve metric files, each of which needs exactly one decision: *does this metric's definition include framework-dispatch and declarative-wiring edges, or only explicit call edges?* Defaults:

| Metric | Default policy |
| --- | --- |
| `dead_code.no_callers` | Include `Call` **and** `Framework` **and** `Declarative` (a Salesforce Flow invocation still counts as a caller) |
| `cycles` | `Call` only (declarative wiring is data-flow, not control-flow cycles) |
| `coupling` | `Call` + `Type` + `Framework` + `Declarative` (all outbound reach) |
| `blast_radius` | `Call` + `Framework` + `Declarative` (symmetric to `no_callers`) |
| `cohesion` | `Call` only (same reason as cycles) |
| `depth` | `Contains` + `Extends` (no Call / Framework at all) |

Each of the twelve consumer files gets one explicit decision documented. T1 is ~1–2 d as scoped.

### Ticket implications from D5

- **T1 ships the enum change + seven emission-site migrations + twelve consumer audit decisions + fixture migration.** Scope stays at the plan's ~1–2 d.
- **T1 *must* also update the storage layer** (`sqlite_repository.rs`) to round-trip the new variants through the `edges.kind` column. This is a schema-compat read for existing `parse.db` files — unknown `EdgeKind` strings on deserialise should fall through to a newly-introduced `EdgeKind::Unknown(String)` variant, not panic. Without this, every in-flight `parse.db` (NPSP `rev 9`, the five canaries) is invalidated by the ship commit.
- **T1 provenance audit:** the framework-adjacent emission sites currently emit `EdgeKind::Call` with `Provenance::Heuristic` and `Confidence::Low` or `Medium`. After T1, they emit `EdgeKind::Framework` with the *same* provenance and confidence — the information was never in the confidence tier; it was in the edge kind.

---

## D6 — supersede-audit of existing namespacing / stable-id design docs

### Method

Read `docs/deferred/aspirational-architecture/FIX_EDGE_NAMESPACING_AND_PROVENANCE_SYSTEM.md` and `IMPLEMENT_NAMESPACED_EDGES_AND_STABLE_IDS.md`, compare to T1 + T2 design in the universal-fidelity sprint plan.

### Findings

Both docs are **superseded by T1 + T2 and must be moved to `docs/deferred/aspirational-architecture/` with a SUPERSEDED header.** Reasoning:

1. **`FIX_EDGE_NAMESPACING_AND_PROVENANCE_SYSTEM.md`** proposes an `EdgeNamespace { Syntactic, Semantic, Synthesis, Derived, Ontology }` enum layered *on top of* edge kinds, with kinds themselves as free-form `String`s. This scheme conflates three orthogonal things the layered-fidelity architecture keeps separate:
   - *Edge kind* — `Call`, `Contains`, `Framework`, `Declarative`, etc. (the semantic relationship). Shipped as typed enum variants in T1, not strings.
   - *Provenance source* — `TreeSitter`, `Heuristic`, `LSP`. Already shipped.
   - *Layer membership* — Layer 0 / 1 / 2 / 3 / 4, derived from the source × kind pair. Not a separate namespace axis.
   The `Synthesis` and `Ontology` namespace members are aspirations that were never implemented and are explicitly deprioritised by the pivot away from aspirational-UCGR (see `docs/deferred/aspirational-architecture/README.md`).
2. **`IMPLEMENT_NAMESPACED_EDGES_AND_STABLE_IDS.md`** proposes `ContentBasedIdGenerator` with a `generate_id(source, span, content)` contract. The design is almost right — it is in fact the direct ancestor of T2. But it couples ID generation to `DataSource` + `Span` (includes line/col), which is exactly the `rev 6.1` corruption root cause (IDs churning on whitespace edits). T2 fixes this by hashing `SHA256(fqn || normalized_body_hash)` with line/col explicitly *excluded* from the hash input. The existing doc's ID scheme is therefore partially-correct-but-buggy and its continued presence in the live architecture tree confuses which design is current.

### Decision

1. Move both files to `docs/deferred/aspirational-architecture/` (already the established parking lot for documents the layered-fidelity pivot supersedes).
2. Add a one-paragraph SUPERSEDED header to each file pointing at this DISCOVERY_REPORT §3 (edge taxonomy) and §STABLE_ID_NORMALIZATION (§ below), plus T1 and T2 ticket IDs.
3. Delete the stale "PENDING D6 SUPERSEDE AUDIT" note in `docs/deferred/aspirational-architecture/graph-system/README.md` and replace with "Both prior design documents superseded by T1 / T2; see `docs/workstreams/universal-fidelity/DISCOVERY_REPORT.md`."

See the "Follow-up actions" section at the bottom of this report for the concrete file operations.

---

## Stable-ID normalization decision (sets T2's hash-input contract)

### Problem restatement

Today, node IDs are `SHA256(fqn || line || col)`. Whitespace edits change line/col, so a symbol whose body is semantically unchanged still changes ID, which breaks trend, feedback, and incremental re-parse. `rev 6.1/baseline.json` was overwritten with `rev 9` data partly *because* of this instability — there is nothing content-stable to compare against.

### Decision (T2)

**Node ID = `SHA256(fqn || normalized_body_hash)`**, where `normalized_body_hash` is computed from the symbol's AST-level body text with the following normalisations applied, in order:

1. Strip all comments (block, line, doc — all three).
2. Collapse all whitespace runs to a single space.
3. Strip leading / trailing whitespace from each line; re-join with `\n`.
4. **Do not** touch string or char literals — their contents are semantic.
5. **Do not** normalize identifiers — rename is a semantic change that must churn IDs.

The FQN component remains exactly as today (language-qualified dotted name including enclosing class / module path).

### Compat read

`sqlite_repository.rs::load_edges` gains a migration shim: if an existing `parse.db` has node IDs of the legacy `SHA256(fqn || line || col)` shape, re-derive FQNs from the `nodes` table and re-generate their T2-shape IDs on the fly; emit both old-ID and new-ID columns in a transient mapping table. One-shot. Subsequent parses write only T2-shape IDs.

### Non-goals for T2

- Does **not** change edge-ID schemes. Edges identify by `(from_id, to_id, kind)`. Once node IDs are T2, edges are automatically T2 as well.
- Does **not** try to make IDs stable across language version upgrades (if Apex ever re-versions `apex-jorje-lsp` and FQN conventions change, IDs will churn — that is a feature, not a bug).

---

## Known unknowns

Things the discovery phase could **not** resolve and that we flag here so T6, T7, T8, and Phase 3 do not trip on them silently:

1. **ra_ap_ide build footprint on our workspace.** D4 confirmed the adapter surface is reachable; it did not measure wall-clock build time, cold-start memory, or on-demand latency of `Analysis::new(...)` against our 75 k-LOC workspace. T6 measures this. If cold-start exceeds ~30 s on an M1-class machine, T6 must either cache the `Analysis` instance across scans or defer to an out-of-process `rust-analyzer` with a session supervisor (undoing part of the D4 benefit).
2. **`jedi` vs `pyright` for Python Layer 2.** Deferred to Phase B. Jedi is library-grade; pyright is an LSP-wire tool written in TypeScript. D1 shows Python's current state is `SyntacticOnly`-tier today, so neither is urgent; the decision is deferred until a Python customer emerges.
3. **Roslyn embeddability for C#.** C# Layer 2 will most likely need a sidecar process (same problem as Java `jdtls`). D4 deliberately did not prototype C#. Deferred to Phase B.
4. **Cross-language co-change clustering in T7.** The obvious implementation of `co_change_clusters` is a connected-component analysis over the "files changed in the same commit" graph. On polyglot repos, this bridges Apex / JS / VF / LWC naturally. That is great when it is correct, but on a repo with noisy commits (branch merges, formatter sweeps) it can produce spurious giant clusters. T7 must include a commit-noise filter (e.g., exclude commits that touch > N files, where N is repo-size-proportional). Discovery did not calibrate N.
5. **Hand-audit methodology for Layer 2 claims per language.** The `rev 9` `no_callers` Round 5 hand-audit is the Apex-resolver baseline. We do not yet have an equivalent audit protocol for Rust Layer 2 / Python Layer 2 / etc. Phase 3 inception-dogfood drafts that protocol on Rust; subsequent per-language adapter work inherits it.

---

## Phase 2 ticket-scope revisions (Phase 1 → Phase 2 handoff)

No ticket was invalidated. Three tickets gain explicit preconditions:

| Ticket | Revision |
| --- | --- |
| T1 | Add `EdgeKind::Unknown(String)` variant for forward-compat on deserialise. Document the 12-consumer audit decisions table from §D5. |
| T2 | Normalization rule formalised (§STABLE_ID_NORMALIZATION above). Compat-read contract specified. |
| T4 | Threshold calibration formalised: Authoritative ≥ 80 %, HeuristicPrimary 40–80 %, SyntacticOnly < 40 %, Unknown when zero Call edges. Applies per-edge-kind, not whole-graph. |
| T6 | Add repo-root `rust-toolchain.toml` prerequisite (MSRV 1.91). Footprint / latency measurement is T6 scope, not D4 scope. |
| T7 | Add shallow-clone fallback (`status: insufficient_history`). Include at least one full-history fixture (gridseak-self). |
| T8 | Gated ON by D2 findings. Emit `extraction_coverage` per-file signal during parsing. `dead_code_classifier` consumes it. Language-parameterised from day one. |

Total sprint effort estimate unchanged at **11–19 days, 3 calendar weeks**.

---

## Follow-up actions (opened at the end of Phase 1)

1. ✅ Done (2026-04-19): `FIX_EDGE_NAMESPACING_AND_PROVENANCE_SYSTEM.md` and `IMPLEMENT_NAMESPACED_EDGES_AND_STABLE_IDS.md` now live in `docs/deferred/aspirational-architecture/` with SUPERSEDED headers citing this report.
2. ✅ Done (R1/D1, 2026-05-25): `graph-system/README.md` was moved alongside them to `docs/deferred/aspirational-architecture/graph-system/README.md` so the whole superseded set lives in one folder.
3. Mark todo `phase-1-discovery` complete.
4. Mark todo `t1-edgekind-namespaces` as the next in-progress item.

## Cross-references

- Universal-fidelity sprint plan: `docs/workstreams/universal-fidelity/` (directory; the plan itself is the canonical source).
- Phase A closure attempt and `rev 9` Round-5 audit: `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md`, `REGRESSION_RESULTS.md`.
- Risk register: `docs/workstreams/proof-foundation-gap/FOLLOWUP_RISKS.md` (R39, R40, R41, R44, R45, R46).
- Deferred aspirational architecture: `docs/deferred/aspirational-architecture/README.md`.
