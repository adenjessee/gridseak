# Universal-fidelity sprint — new-engineer primer

**Audience.** You understand computer science fundamentals — compilers, graphs, static analysis at the "I know what an AST is" level — but you are new to this codebase and this sprint's vocabulary. You already know the product's goal: ship honest structural-health metrics for any codebase in any language. This document teaches you everything you need to understand the universal-fidelity sprint, read the `DISCOVERY_REPORT.md`, and take meaningful decisions on T1 through T8.

**How to read this.** Each section names a concept, gives you an analogy from something you probably already know, then grounds the analogy in a concrete piece of this codebase. If a section feels too obvious, skim it — it's there for someone more junior than you. If a section feels too dense, stop and read the linked source file — the code is the definitive spec.

---

## 1. The product in one minute

The shipping CLI is `ge-analyze`. It points at a codebase, builds a graph where every function is a node and every call is an edge, computes structural-health metrics from that graph (cycles, blast radius, coupling, dead code, etc.), and renders a report. A developer looks at the report and decides what to refactor.

The product only works if the graph is **truthful**. A false dead-code claim tells a developer to delete a function the framework secretly calls at runtime. A false cycle tells them there's an architectural sin where there isn't one. Every claim the report makes must be traceable back to evidence the engine can defend.

The whole sprint is about making that truthfulness **measurable** and **layered** — not self-declared.

---

## 2. The parsing pipeline in one picture

Follow a file from disk to report:

```
source file → tree-sitter parse → AST
              ↓
         extractors                    ← language-aware walk of the AST
         - symbol_extractor            ← produces Function / Class / Module nodes
         - call_site_extractor         ← produces CallSite records (unresolved)
         - type_reference_extractor    ← produces TypeReference records
              ↓
         SyntaxResults                 ← bundle of the above + `synthesized_edges` + `class_symbols`
              ↓
         resolvers                     ← turn name-references into real graph edges
         - lsp/call_resolver           ← uses LSP or heuristic to resolve target function node
         - type_resolver
         - import_resolver
              ↓
         Graph { nodes, edges }        ← every edge has a kind + provenance + confidence
              ↓
         SQLite (parse.db)             ← persisted form; `graphengine-analysis` reads this
              ↓
         AnalysisGraph                 ← in-memory representation, same shape
              ↓
         metrics                       ← cycles, coupling, dead_code, blast_radius, ...
              ↓
         MetricsReport (JSON)
```

Two crates, cleanly split:

- `graphengine-parsing` — everything from "file on disk" through "edge in SQLite."
- `graphengine-analysis` — everything from "read SQLite" through "JSON report."

Each crate has its own `EdgeKind` enum. **They are mirrors** by convention, not by compile-time relationship. Keep them in sync manually — if you add a variant to one you almost always add it to the other.

---

## 3. Nodes, edges, and their identities

### Nodes

A **node** is anything the graph can point at: a function, a class, a module, a file, a folder. Each node has a **FQN** (fully-qualified name — e.g. `foo::bar::Baz` in Rust, `com.example.Baz` in Java, `MyClass.myMethod` in Apex) and a **stable ID**.

Today, the ID is computed as `SHA256(fqn || line || col)`. That has a subtle bug: reformat the file, the line numbers shift, the ID churns, and every piece of evidence about that symbol — trend lines, user-submitted feedback, incremental scan caches — is orphaned. **T2 fixes this.**

### Edges

An edge is a relationship between two nodes. The **kind** of an edge says what relationship:

| `EdgeKind` | Meaning |
| --- | --- |
| `Contains` | Structural parenthood — module contains function, file contains class. Non-structural; you could delete every `Contains` edge and the *logic* of the program would be unchanged. |
| `Call` | "A calls B" at runtime. The headline edge type; most metrics run on it. |
| `Type` | "A's parameter/return/field type references B." |
| `Extends` | "A inherits from B" (class inheritance). |
| `Implements` | "A implements interface B." |
| `Import` | "A imports module B." |
| `Uses` | "A references identifier B" in a way that doesn't cleanly fit the other buckets. |

The reason these are distinct rather than one "relationship" enum: different metrics care about different subsets. `cycles` runs on "all non-Contains edges." `dead_code` uses fan-in, which is "incoming non-Contains edges." `depth` (call-depth) runs on `Call` only. The distinction is how the type system enforces "this metric knows which edges it's defined on."

### Provenance — where did this edge come from?

Every edge carries a `Provenance { source, confidence }` tag:

| `Source` | Meaning |
| --- | --- |
| `TreeSitter` | The edge was derived from tree-sitter syntax alone — name match, no type system consulted. |
| `Heuristic` | The engine's own resolver (per-language rules) produced the edge. No authoritative tool involved. |
| `Lsp` | The edge came from a language server's authoritative answer (rust-analyzer, tsserver, jdtls, ...). |

| `Confidence` | Meaning |
| --- | --- |
| `Low` | Name matched something that looked right. May be wrong. |
| `Medium` | Multiple signals agreed (same file, receiver type inferred, etc.). Usually right. |
| `High` | An authoritative tool said so. Only LSP produces High on calls. |

When the `DISCOVERY_REPORT` says NPSP has 1.1 % `Call` edges at `High`, what it literally means is: out of every 100 call edges in the NPSP graph, only ~1 came from an LSP. The other 99 came from the engine's heuristic resolver or from tree-sitter's name-match. That is the "fidelity gap" the sprint is measuring.

---

## 4. What "fidelity" really means here

Take this worked example. You have two repos the engine says have a cycle:

- **Repo A.** The cycle was found in a sub-graph where 95 % of call edges are `Lsp|High`. Rust-analyzer said: these functions really do call each other. The cycle is real.
- **Repo B.** The cycle was found in a sub-graph where 99 % of call edges are `TreeSitter|Low`. Tree-sitter saw `foo.bar()` on a receiver whose type we never resolved, so we guessed every method named `bar` might be the target. The cycle may well be an artifact of that ambiguity.

The engine today reports both as "a cycle." The number is identical. The honest reader should not trust them equally.

**Fidelity is the measured property of the evidence behind a metric.** It is not a philosophical property, not a configuration toggle, not a thing you declare. It is literally the distribution of `Source × Confidence` on the edges that feed the metric. T3 (dual-metric emission) exposes this.

This is the whole reason the sprint exists.

---

## 5. The layered-fidelity architecture

The sprint was triggered by the realization that one graph cannot honestly represent all five of these signal strengths at once. The architecture splits evidence into layers, each with its own authority claim:

| Layer | Signal | Always available? | Confidence ceiling |
| --- | --- | --- | --- |
| **0 — Git signals** | Change frequency, co-change, ownership, recent-defect proximity | Yes (any git repo) | Reliable from day 1 |
| **1 — Tree-sitter structural** | Classes, functions, modules, LOC, complexity, name-matched references | Yes (if we have a grammar for the language) | `Low` — these are syntactic only |
| **2 — Language-native semantic** | Authoritative call / type / import graphs from the language's own compiler or LSP | Only if the authoritative tool is installed or linkable | `High` — authoritative |
| **3 — Cross-language analysis** | Cycles, blast radius, coupling, cohesion, dead code | Depends on Layer 2 | Dual-emitted: all-edges vs High-only |
| **4 — Declarative / framework wiring** | Salesforce Flow XML, LWC templates, Spring XML, Angular templates, Django URLconf | Per-ecosystem opt-in | Authoritative *for that wiring*, not for Call |

**Key insight.** Not every layer answers every question. Layer 0 can tell you "this file is a hotspot" on any repo on Day 1 — no Layer 2 needed. Layer 3 *cannot* give you an honest coupling metric on a Layer-1-only graph; the best it can do is compute it anyway and flag `fidelity_gap = high` in the output.

Today the engine treats everything as one uniform graph. The sprint splits it.

---

## 5.5 The two-jobs rule

One sentence: **if a type, a prose section, or a code path is fighting you, it is probably carrying two responsibilities — split before you compromise.**

This is the meta-principle behind every architectural move the sprint makes. Named explicitly so you recognize it on sight in your own work.

Four live citations from this codebase:

1. **Domain `EdgeKind` vs wire `PersistedEdgeKind`.** `EdgeKind` was being asked to be both the in-memory closed taxonomy the engine reasons over *and* the open container for whatever a newer engine version might have persisted to SQLite. Those are different types. The split — `EdgeKind: Copy` in memory, `PersistedEdgeKind { Known(EdgeKind), Unknown(String) }` at the boundary — satisfies both properties with no trade-off. See §8 Decision 1.
2. **Layered fidelity (one graph per layer, not one graph for all).** The original architecture had one graph carrying tree-sitter name-matches, heuristic inferences, and LSP-authoritative resolutions as equals. That graph was being asked to deliver both "a structural skeleton of the code" (Layer 1's job) and "an authoritative call graph" (Layer 2's job), and the numbers it produced were dishonest because it could not tell which edges were which. The five-layer split (see §5) is the two-jobs rule applied to the whole architecture.
3. **T3 dual-metric emission (two numbers, not one).** Reporting `coupling = 12` to the user was forcing a single f64 to mean both "all the edges we have" and "the edges we can defend". Those are two numbers. T3 emits both and reports the gap. The fix is not a better averaging rule; it is refusing to collapse two quantities into one.
4. **T4 measured vs self-declared tier.** `ResolutionTier::Full` was being asked to be both "the LSP ran" (a configuration fact the resolver self-reports) and "the LSP actually resolved the calls" (an empirical property of the edges). Those are different questions. T4 leaves `resolution_tier` as the self-report and adds `measured_fidelity.tier` as the empirical measurement; consumers pick whichever they meant.

Each of those was presented at design time as a forced trade-off. Each dissolved once someone pointed at the missing second type, predicate, channel, or measurement.

When you next feel a type fighting you, run through §8.0's diagnostic questions before accepting the trade-off framing. The right move is usually to name a third thing, not to pick a loser.

---

## 6. The ticket map: what T1 through T8 really do

Each ticket exists because the current system violates one specific piece of the layered-fidelity discipline.

### T1 — typed `EdgeKind::Framework` and `::Declarative` variants

**Problem it solves.** Today, a Visualforce page binding `{! save}` to an Apex method `Controller.save()` ends up as `EdgeKind::Call` with `Provenance::Heuristic|Medium`. It is not a call in the normal sense — it is a framework dispatch, a declarative binding resolved by the Salesforce runtime, not by Apex-the-language. The engine has been coercing every ecosystem-specific wiring into `Call` because `Call` was the only variant that existed.

**Why this matters for metrics.** When a user asks "what's my call-cycle count?" they probably don't want VF framework dispatches counted as cycles. When they ask "what's dead code?" they probably *do* want a VF binding to count as a caller (the VF page invokes the method at runtime). Today we can't make those decisions per-metric because we can't tell framework edges apart from normal calls.

**Shape of the fix.** Add two new variants:

```rust
pub enum EdgeKind {
    // existing
    Call, Contains, Import, Extends, Implements, Type, Uses,
    // new
    Framework(FrameworkKind),
    Declarative(DeclarativeKind),
}

pub enum FrameworkKind { VisualforcePage, /* + LwcTemplate, AuraComponent, Trigger as emitters land */ }
pub enum DeclarativeKind { Flow, /* + ProcessBuilder, WorkflowRule, ... */ }
```

Each downstream metric then makes an explicit decision: "does my definition include Framework and Declarative, or just Call?" The decision is *documented*, not implicit. See §8 below for the decision table.

### T2 — content-based stable IDs

**Problem.** `SHA256(fqn || line || col)` churns on reformat. Trend, feedback, incremental parse all break.
**Fix.** `SHA256(fqn || normalized_body_hash)`. Normalize = strip comments, collapse whitespace, keep identifiers and string literals untouched. Rename = ID change (semantic). Reformat = ID stable.

### T3 — dual-metric emission

**Problem.** `coupling = 12` on a 95 %-Low graph and `coupling = 12` on a 95 %-High graph are reported identically. The reader cannot distinguish.
**Fix.** Every Layer-3 metric emits three numbers: `all_edges`, `high_only`, `fidelity_gap = |all - high| / all`. The UI shows all three. The reader sees the honest picture.

### T4 — measured fidelity tier

**Problem.** The engine today declares `ResolutionTier::Full` on every scan where the LSP ran, regardless of whether the LSP actually resolved anything. On NPSP `rev 9`, `Full` is claimed while `Call / Lsp / High` edges are 1.1 % of total.
**Fix.** Replace self-declared tier with *measured*: `Authoritative ≥ 80 % High on Calls`, `HeuristicPrimary 40–80 %`, `SyntacticOnly < 40 %`, `Unknown = 0 Call edges`. Derived from `edges_by_confidence`, not from a flag the resolver sets.

### T5 — orchestrator trait-method collapse

**Problem.** The parsing pipeline has Apex-specific hook stages (`vf_extraction`, `framework_entry_point_propagation`, Apex `class_symbols` phase) hardcoded into the orchestrator. Adding a language grows the orchestrator. That is the pattern that produced R13/R23/R25/R26/R27/R28 — language-specific pipelines each growing their own stage.
**Fix.** Move these into `LanguageSpecificExtractor` trait methods: `post_syntax_hooks(...)`, `class_symbols_phase(...)`. Next language does not grow the orchestrator.

### T6 — Rust Layer 2 adapter via `ra_ap_ide`

**Problem.** Every language currently runs on Layer 1 + our own heuristic resolver. No language has a working Layer 2 today (TypeScript's `tsserver` sometimes works but is LSP-wire-protocol, with all the fragility that implies).
**Fix.** Link `ra_ap_ide` — the rust-analyzer IDE API crate, library-grade, no LSP wire protocol — and emit `Call / Lsp / High` edges for Rust. Dogfood on `gridseak-self`. Rust is the best first target because (a) library-grade, (b) our team already reads Rust fluently, (c) our existing Rust heuristic resolver gives us ground truth.
**Precondition.** Bumps workspace MSRV to 1.91 via `rust-toolchain.toml`. Low-risk, well-understood.

### T7 — Layer 0 git signals

**Problem.** The engine today ignores git history. Layer 0 is the "always-available truth" floor — and we're not using it. Customers who don't have a working Layer 2 for their language (most of them, by D1's numbers) see degraded metrics with no fallback.
**Fix.** Add `graphengine-git-signals` crate using `gix`. Emit change_frequency, co_change_clusters, ownership_dispersion, hotspot_score, etc. Guard with a `insufficient_history` fallback for shallow clones (our canaries are 1-commit clones, so the guard is load-bearing from day 1).

### T8 — classifier extraction-coverage awareness

**Problem.** The Round 5 audit failed because extraction gaps (R39, R41) produced systematic false positives in `dead_code.no_callers`. A method that's called from inside a property accessor or a map-literal initializer is invisible to our extractor, so `fan_in` = 0, so it looks dead. 226 `.cls` files have R39 shapes; 45 have R41 shapes. Fixing the extractors is Phase-B extractor work we're not doing this sprint.
**Fix.** Per-file `extraction_coverage` signal. `dead_code_classifier` consults it: if this file has unwalked property accessors, downgrade `no_callers` confidence to Medium or filter out of the headline metric. Honest workaround for the extractor gap without fixing it yet.

---

## 7. Where the reality-check pressure came from

The Round 5 audit on NPSP `rev 9` drew 10 random `no_callers` samples, hand-verified each one, and found 4 were wrong — three classes of upstream bug:

- **R39** (2 wrong) — property accessor bodies (`get { ... }`) aren't walked by the extractor, so calls inside them are invisible.
- **R41** (1 wrong) — map-literal field initializers aren't walked.
- **R45** (1 wrong) — chained call on the return value of another call (`obj.first().second()`) wasn't resolved.

None of these are bugs in the Apex resolver §4.11 investment. They are all *upstream of the resolver*. No amount of resolver investment fixes them.

Round 5's verdict made clear that continuing to invest in Apex-specific resolver depth — Phase B as originally planned — would not move the audit numbers. Meanwhile, every other language tested (commons-lang at 0.2 % High, django-site at 0 %, serilog at 0.8 %) has essentially no Layer 2 at all. The sprint's thesis is that **one Layer 2 adapter shipped end-to-end (T6) plus the measurement discipline (T3/T4) is worth more than another two weeks of Apex-resolver depth (Phase B)**.

Phase A stays formally OPEN with the `rev 9` FAIL on record. Phase B is deliberately deferred until after the universal-fidelity sprint lands and a real Phase-B decision point can be taken with Rust Layer 2 data in hand.

---

## 8. T1 — the actual decisions you'd need to make

> **Reader's note.** The original §8 presented T1 as six A-vs-B trade-offs. Senior review after T1 shipped found that five of the six were false trade-offs hiding a missing type, predicate, or channel. §8 has been restructured. Each decision now leads with an **"earlier framing (preserved)"** block — the original A/B discussion — so commit archaeology pointing at "Decision 3" still lands on the right content. After the preserved block, each section names the root cause, the dissolving element, and why the choice evaporates once the third element is named. §8.0 below lists the diagnostic questions you should run through **before** accepting an A/B framing yourself.

### 8.0 Diagnostic questions before accepting a trade-off

When you are tempted to frame a design question as "Option A vs Option B, pick your pain", pause. Run these five questions:

1. **Are we forcing one type to do two jobs?** If yes, split the type. Domain vs wire. Taxonomy vs metadata. Intent vs enactment. Most "choose A or B" framings are a single type being asked to do two incompatible jobs simultaneously, and the answer is not to pick a loser but to give each job its own type.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** If yes, invert. Variants declare their properties; consumers query predicates. (See Decision 5 for the canonical example: metrics that used to enumerate `EdgeKind` variants became predicate calls after the inversion.)
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** If yes, introduce a typed channel between the stages. Do not force one stage to guess the other's knowledge (an `Option<hint>` is the smell). A variant-typed enum carrying the distinguishing information makes the downstream exhaustiveness check compiler-enforced.
4. **Is the trade-off about serialization format?** If yes, use `serde` with explicit tags. Rust already solved this. Hand-rolled `to_stable_str` and `Debug` format are both wrong answers to a solved problem.
5. **Is the trade-off between two modes of failure?** If yes, the missing element is usually an explicit error type or a `Result` boundary, not a coin flip between which failure mode to prefer.

If none of those five apply, it may be a real trade-off. Most of the time, one applies and the framing was incomplete. See §5.5 for the meta-principle behind all five.

This same discipline is embedded in [`docs/workstreams/universal-fidelity/tasks/TEMPLATE.md`](tasks/TEMPLATE.md) §3 — every task design doc runs the five questions at authoring time, so the diagnostic becomes a habit, not a review finding.

### Decision 1 — `EdgeKind` identity at the SQLite boundary

#### Earlier framing (preserved)

`EdgeKind` today is `#[derive(Copy)]`. Copy lets you pass it by value everywhere without lifetime bookkeeping — nice. But `Copy` forbids variants that carry heap data like `Unknown(String)`, which we'd want to gracefully accept unknown edge kinds from future versions of `parse.db`.

- **Option A.** Drop `Copy`. Add `Unknown(String)`. Every site that currently takes `EdgeKind` by value needs to take it by reference or clone it. Touches a lot of code.
- **Option B.** Keep `Copy`. Handle unknown edge-kind strings at the SQLite deserialization boundary by logging and skipping the edge. No in-memory `Unknown` variant.

**Earlier decision: B.** The forward-compat scenario (reading a newer DB with an older binary) is rare in practice and better solved by bumping `schema_version` in the SQLite metadata and forcing a re-parse. Keeping `Copy` preserves ergonomics in all 260-odd `EdgeKind` use sites. Logged+skipped edges leave a warning in the parse log; they're not silently dropped.

#### Root cause

`EdgeKind` was being asked to be two different types at once: the compile-time closed taxonomy the engine reasons over **and** the runtime-open container for whatever a newer engine version might have written to SQLite. Those are fundamentally different types. Diagnostic question 1 fires (one type, two jobs).

#### Dissolving shape

```rust
// In-memory, closed, Copy. Used everywhere inside parsing & analysis.
#[derive(Copy, Clone, ...)]
pub enum EdgeKind {
    Call, Contains, Import, Extends, Implements, Type, Uses,
    Framework(FrameworkKind),
    Declarative(DeclarativeKind),
}

// At the SQLite boundary only. Open, not Copy.
pub enum PersistedEdgeKind {
    Known(EdgeKind),
    Unknown(String),
}

impl From<EdgeKind> for PersistedEdgeKind { /* ... */ }
impl TryFrom<&PersistedEdgeKind> for EdgeKind { /* ... */ }
```

The SQLite repository converts at the boundary. Analysis receives `EdgeKind` and stays `Copy` everywhere. A newer DB with an unknown variant round-trips through `PersistedEdgeKind::Unknown(String)`, surfaces as an integrity-caveat entry ("N edges dropped due to version skew"), and never silently corrupts the graph.

#### Why this is not actually a choice

You get both properties (ergonomic `Copy` everywhere + forward-compat at the wire layer) with zero trade-off once the two types are separated. The earlier framing forced a "pick one property" decision that only existed because one type was doing two jobs.

### Decision 2 — `FrameworkKind` / `DeclarativeKind` shape

#### Earlier framing (preserved)

- **Option A.** `FrameworkKind` variants carry data (`VisualforcePage { page_name: String }`). Forces `EdgeKind` to drop `Copy`.
- **Option B.** `FrameworkKind` variants are unit variants (`VisualforcePage`, `LwcTemplate`). Any per-instance data goes on the edge's `provenance` or on the node.

**Earlier decision: B.** The edge kind is a *taxonomy* question — which family does this edge belong to — not a "carry arbitrary metadata" question. Metadata belongs in `provenance` or node properties. This also keeps `EdgeKind: Copy`.

#### Reframe as consequence of §5.5

This is not a decision; it is a consequence of separating concerns. Taxonomic variance ("which kind of framework binding") and instance metadata ("which page name") live on different axes. The first belongs in `EdgeKind`; the second belongs in `Provenance` or node properties. Treating them as A/B was a framing error. Each has exactly one correct home — no choice to make.

### Decision 3 — where the framework-kind information lives between stages

#### Earlier framing (preserved)

Today, VF binding emission produces a `CallSite` record that the resolver later binds to a real Apex Function node and stamps as `EdgeKind::Call`. To migrate the final edge to `EdgeKind::Framework(VisualforcePage)`, we need *somewhere* to carry "this call should become a Framework edge."

- **Option A.** Extract-time emission. VF extraction creates a fully-formed `Edge::framework_visualforce(...)` directly in `synthesized_edges`, bypassing the resolver. Problem: the resolver is what knows the target node's ID; VF extraction only knows `"Controller::save"` as a string.
- **Option B.** Resolve-time emission with a hint. Add `edge_kind_hint: Option<EdgeKind>` to `CallSite`. VF extraction sets the hint. The resolver honors it when emitting.

**Earlier decision: B.** The resolver is the only point where we know the target node ID. Tearing out that architectural invariant to emit edges earlier would be a Phase-B-scale change. The hint is four lines in `CallSite` plus two lines in the resolver.

#### Root cause

The extractor knows the framework context (this is a VF binding). The resolver knows the target node ID (this binds to `Controller.save`). Neither alone can emit the right edge, and the earlier framing asked us to pick a losing side. Diagnostic question 3 fires: two stages, different knowledge, no typed channel between them. An `Option<hint>` is the field-smell of a missing enum.

#### Dissolving shape

```rust
// graphengine-parsing/src/application/ports.rs
pub enum UnresolvedReference {
    Call(CallSite),
    FrameworkBinding(FrameworkBinding),   // carries FrameworkKind + source binding location
    DeclarativeBinding(DeclarativeBinding), // for Flow/ProcessBuilder/...
}
```

Extractors emit the typed variant that matches what they saw. The resolver matches on the enum and dispatches to the correct resolution path. The compiler enforces "every resolution-consuming code path handles every variant" — deleting a resolver arm fails to compile.

#### Why this is not actually a choice

Option B's `edge_kind_hint: Option<EdgeKind>` is silently-broken-by-default: any downstream consumer that forgets to check the hint quietly produces `Call` edges where `Framework` was meant. The typed-variant approach makes that failure mode impossible to express at the type level. Correctness by construction replaces correctness-by-grep.

### Decision 4 — SQLite serialization format

#### Earlier framing (preserved)

Today: `format!("{:?}", edge.kind)` produces `"Call"`, `"Contains"`, etc. For `Framework(VisualforcePage)`, `{:?}` produces `"Framework(VisualforcePage)"` — parseable but ugly. Also, `{:?}` is the Debug format, which is explicitly not stable under `derive` — a future Rust compiler could change the format.

- **Option A.** Keep `{:?}` and live with the ugliness.
- **Option B.** Add explicit `EdgeKind::to_stable_str()` and `EdgeKind::from_stable_str(&str)`, replace all `format!("{:?}", ...)` with `.to_stable_str()`, use colon-delimited form: `"Framework:VisualforcePage"`.

**Earlier decision: B.** This is the right moment — we're touching the schema anyway. Colon-delimited is grep-friendly, future-grammar-additions-friendly, and explicit. Stable-string contract is documented at the method site.

#### Root cause

Both options ignore the serialization crate every Rust project already uses. Diagnostic question 4 fires.

#### Dissolving shape

```rust
#[derive(Serialize, Deserialize, ...)]
#[serde(tag = "kind", content = "sub")]
pub enum EdgeKind {
    Call, Contains, Import, Extends, Implements, Type, Uses,
    Framework(FrameworkKind),
    Declarative(DeclarativeKind),
}
```

Round-trip is compiler-checked. Adding a variant requires zero parser updates. `serde_json::to_string(&EdgeKind::Framework(FrameworkKind::VisualforcePage))` produces `{"kind":"Framework","sub":"VisualforcePage"}` — stable, versioned, documented, grep-friendly. Composes with Decision 1: `PersistedEdgeKind` gets `#[serde(untagged)]` and unknown strings deserialize into the `Unknown(String)` arm automatically.

The wire format pins as literal strings in `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` so future derive-changes or `#[serde(rename)]` additions trip a test.

#### Why this is not actually a choice

Hand-rolled `to_stable_str` / `from_stable_str` is work that `serde` already does for free, with stronger guarantees and with fewer moving parts.

### Decision 5 — which metrics include Framework / Declarative edges

#### Earlier framing (preserved)

Audit the 12 downstream metric consumers. From the code:

| Metric | Current filter | Decision for Framework / Declarative |
| --- | --- | --- |
| `dead_code.no_callers` (via `fan_in`) | "incoming non-Contains" | **Include.** A VF binding is a real caller. Don't false-positive dead code on VF-invoked methods. |
| `cycles` | all structural | **Include.** Already in — the new variants are structural, not Contains. |
| `coupling` | all structural | **Include.** Framework dispatch counts as coupling. |
| `blast_radius` | structural outgoing from a node | **Include.** If you break a VF-invoked method, downstream damage propagates. |
| `cohesion` | all structural | **Include.** Same reasoning. |
| `fan_metrics` | fan-in / fan-out | **Include.** See `dead_code`. |
| `depth` | `== Call` | **Include** — switch to `is_call_like()`. Call-depth includes framework dispatch. |
| `layers` | `== Call` | **Include** — switch to `is_call_like()`. Layer topology is about call-reach. |
| `metric_status` | `== Call` in the "Call edge count" health metric | **Include** — switch to `is_call_like()`. The metric measures resolution health across all call-family edges. |

**Earlier decision:** a per-metric audit with `is_call_like()` introduced as a single helper.

#### Root cause

The earlier audit locked in a one-shot snapshot of "which variants each metric includes". Every time a new variant lands, the audit would have to repeat. Diagnostic question 2 fires: hardcoded lists (implicit in each metric's filter) versus brittle audit updates.

#### Dissolving shape

Invert the direction. Variants declare their properties; metrics query predicates:

```rust
impl EdgeKind {
    pub fn is_structural(self) -> bool   { !matches!(self, EdgeKind::Contains) }
    pub fn is_call_like(self) -> bool    { matches!(self, EdgeKind::Call | EdgeKind::Framework(_) | EdgeKind::Declarative(_)) }
    pub fn is_dependency(self) -> bool   { matches!(self, EdgeKind::Import | EdgeKind::Type | EdgeKind::Uses) }
    pub fn is_inheritance(self) -> bool  { matches!(self, EdgeKind::Extends | EdgeKind::Implements) }
}
```

Metrics consume predicates:

- `dead_code.fan_in` filters on `is_call_like() || is_dependency()` (anything that creates an incoming reference).
- `cycles` / `coupling` / `cohesion` / `blast_radius` filter on `is_structural()`.
- `depth` / `layers` / `metric_status` filter on `is_call_like()`.

Adding a variant forces one question at variant-introduction time — "what role does this play?" — and every metric automatically picks up the answer. The audit becomes a single-file edit, permanently.

#### Why this is not actually a choice

The earlier per-metric audit was correct in outcome for today's variants, but it enshrined the coupling (metric-knows-variant-list) rather than eliminating it. Predicates replace the per-metric audit with a permanent discipline the compiler enforces: if a variant's role is left unclassified when it's introduced, the metric-side filter that calls `is_call_like()` evaluates the variant membership by the rules declared at the type — no silent drift.

### Decision 6 — which variants ship in T1

#### Earlier framing (preserved)

Temptation: put every Salesforce framework down on paper now (`VisualforcePage`, `LwcTemplate`, `AuraComponent`, `Trigger`, `FrameworkEntryPoint`, `InboundEmail`, ...) because we *will* need them.

**Earlier decision: only ship variants that have an emitter today.** That is `FrameworkKind::VisualforcePage`. Everything else is a variant I'd add without any call site producing it — aspirational code, the pattern I'm criticizing in the sprint. Add variants when emitters land. `DeclarativeKind::Flow` is the exception — it's reserved as the canonical placeholder for when the Salesforce-Flow-XML reader ships (already scoped in `FRAMEWORK_RESOLVER_PLAN.md`), and having one variant on `DeclarativeKind` keeps the type non-uninhabited.

#### Reframe as consequence of §5.5

Code holds what runs; markdown holds what is planned. Those are two different artifacts, not two options on one axis. Shipping only variants with emitters is the discipline; enumerating the full planned taxonomy is the completeness. Both are satisfied — just in different files.

The code answer: only variants with live emitters (plus `DeclarativeKind::Flow` as a placeholder for the already-scoped Flow XML reader).

The markdown answer: [`docs/04-architecture/EDGE_TAXONOMY.md`](../../04-architecture/EDGE_TAXONOMY.md). Every planned-but-unemitted variant sits in that file with columns for "emitter exists?", "tests exist?", "ships in code?". A variant graduates from markdown to code in the same PR as its emitter; the taxonomy file is the source of truth for "what's on the roadmap" and the enum is the source of truth for "what runs".

### Decision 7 — fixtures and tests

`graphengine-parsing/tests/edge_kind_taxonomy.rs` asserts the shape of the `EdgeKind` enum. It needs to grow an assertion pair for `Framework` and `Declarative`. The VF-binding integration test (`apex_resolver_r23_a5_vf_fixtures.rs`) needs to assert the resulting edge kind is `Framework(VisualforcePage)`, not `Call`.

Post-rework, the wire format is additionally pinned by `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` with literal-string assertions per variant. A future serde-rename or derive-reorder change trips this test — the wire format becomes a first-class contract, not an emergent property.

Everywhere else the existing fixtures stay valid — they all use `EdgeKind::Call` for test edges explicitly, and that semantic is unchanged.

### 8.7 Further reading

- [`docs/workstreams/universal-fidelity/tasks/TEMPLATE.md`](tasks/TEMPLATE.md) — the design-doc template every future task uses. §3 embeds the five diagnostic questions so they're answered at authoring time, not discovered in post-ship review.
- [`docs/04-architecture/EDGE_TAXONOMY.md`](../../04-architecture/EDGE_TAXONOMY.md) — the planned-but-unemitted variant list referenced from Decision 6.

---

## 9. Short glossary for cross-reference

| Term | Meaning here |
| --- | --- |
| **FQN** | Fully-qualified name. Language-specific dotted form that uniquely identifies a symbol in its codebase. |
| **CallSite** | A still-unresolved call: location + function name string. Produced by extractors, consumed by resolvers. |
| **resolver** | Code that takes a name reference and produces an edge with a concrete target node ID. Can be LSP-backed (authoritative) or heuristic (per-language rules). |
| **extractor** | Code that walks the tree-sitter AST and produces nodes and unresolved references. No resolution happens here. |
| **LSP** | Language Server Protocol. Wire-protocol interface to a language's compiler, usually over JSON-RPC on stdio. Authoritative, but fragile (slow to start, sometimes crashes). |
| **`ra_ap_ide`** | Rust-analyzer's IDE API crate. Library-grade API — you link it and call functions. Same authority as rust-analyzer the LSP but without the wire protocol. |
| **Apex** | Salesforce's Java-like language. The engine invested heavily in it because of the Salesforce customer wedge. |
| **Visualforce / VF** | Salesforce's pre-LWC templating framework. `.page` files with `{!expr}` bindings that resolve at runtime to Apex methods. The ecosystem case for `EdgeKind::Framework`. |
| **LWC** | Lightning Web Components. Modern Salesforce component framework. Templates bind to JS, JS calls Apex via `@wire`. Another `EdgeKind::Framework` case (not yet emitted). |
| **Flow** | Salesforce's no-code automation tool. XML files that invoke Apex. `EdgeKind::Declarative` case (not yet emitted). |
| **rev N** | Engine revision N. Each revision corresponds to a PR landing and a fresh NPSP parse run. `rev 9` is the current head. |
| **canary** | A small repo used to test the engine in isolation on one language. NPSP (Apex), commons-lang (Java), serilog (C#), django-site (Python), nextjs-commerce (TS/JS). |
| **fidelity gap** | The numerical difference between a Layer-3 metric computed on all edges and on High-only edges. First-class output of T3. |
| **Layer 0 / 1 / 2 / 3 / 4** | The five layers of the layered-fidelity architecture. See §5. |
| **atomic promise** | Product language for "which 3 files are riskiest?" — the Day-1 question the engine must answer credibly on any repo. Layer 0 + Layer 4 is what delivers this today; Layer 2 sharpens it over time. |

---

## 10. What to read next

In order:

1. `docs/workstreams/universal-fidelity/DISCOVERY_REPORT.md` — the evidence base for every decision this sprint takes.
2. `docs/workstreams/proof-foundation-gap/HAND_AUDIT_LOG.md` — Round 5 `rev 9` section. The Phase-A failure that triggered the pivot.
3. `docs/01-status/CURRENT_STATE.md` — what is actually shipped today. Specifically the "Sprint I" section.
4. `graphengine-parsing/src/domain/edge.rs` — 120 lines. Read it cold. It's the contract every downstream piece is written against.
5. `graphengine-analysis/src/health/graph.rs` lines 1–120 — the mirror enum + `AnalysisGraph`. Note the mirror.
6. `graphengine-parsing/src/application/ports.rs` around line 14 — `CallSite` struct. The thing T1 plumbs a hint onto.
7. `graphengine-parsing/src/syntax/language/apex/vf_page_resolver.rs` — VF extraction today. Read to understand why hinting the CallSite is the right place for T1's framework-kind plumbing.

Once you have those open in tabs, the `DISCOVERY_REPORT` and the sprint plan will read as specifications, not as sales pitches.
