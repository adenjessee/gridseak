# T1 — rework (universal-fidelity sprint)

> Authored against [TEMPLATE.md](TEMPLATE.md).

## Retrospective header

**What shipped.** T1 landed as described in the pre-rework version of [NEW_ENGINEER_PRIMER.md](../NEW_ENGINEER_PRIMER.md) §8. Introduced `EdgeKind::Framework(FrameworkKind)` and `EdgeKind::Declarative(DeclarativeKind)`, added hand-rolled `to_stable_str` / `from_stable_str`, added `CallSite.edge_kind_hint: Option<EdgeKind>` to plumb framework context from extractor to resolver, added `is_call_like()` helper on `EdgeKind`.

**What the post-ship review found.** Five of six design decisions were false trade-offs. Specifically:

- **Decision 1** — `EdgeKind: Copy` vs `Unknown(String)` was one type (`EdgeKind`) being asked to do two jobs (in-memory domain + on-disk wire). The correct answer was a boundary type, not a choice.
- **Decision 3** — extract-time vs resolve-time emission was a missing typed channel. `Option<hint>` is the field-smell of a missing enum; silently-broken-by-default when consumers forget to check.
- **Decision 4** — hand-rolled `to_stable_str` ignored `serde`, which every Rust project uses for this. Composition with Decision 1 solves both problems.
- **Decision 5** — per-metric audit of `EdgeKind` usage baked in a hardcoded list pattern. Predicates on the enum invert the coupling.
- **Decision 6** — framed as a trade-off between completeness and discipline; it is neither. Code holds what runs, markdown holds what's planned.

**Rework ticket that owns the correction.** This ticket (P1). Lives after P2 (template), P3 (test-gap closure), P4 (primer rewrite) in the master plan because the rework needs a regression net (P3) and a codified discipline (P2, P4) before the code changes.

---

## 1. Problem statement

The shipped T1 bakes in four silent-failure modes that T6 (Rust Layer 2) would inherit and amplify:

1. **`CallSite.edge_kind_hint: Option<EdgeKind>` is silently-broken-by-default.** A new resolver path that forgets to check the hint produces `EdgeKind::Call` when `EdgeKind::Framework(VisualforcePage)` was intended. No compile error. No runtime warning unless a downstream metric notices. Concrete symptom: if T6's Rust adapter ships a new resolver before P1.d lands, it will silently produce wrong edges for any framework-emitted `CallSite`.
2. **Hand-rolled `to_stable_str` / `from_stable_str` will drift from any future serde addition.** `EdgeKind` already has `#[derive(serde::Serialize, Deserialize)]` today — two serialization systems coexist. Any consumer reading `edges.provenance` (JSON) vs `edges.kind` (hand-rolled) encounters a split-brain where the kind field uses a different format convention than every other column. Symptom: changing a `FrameworkKind` variant name silently changes the on-disk format with no compile-time warning.
3. **Hardcoded variant lists in metric consumers.** Every new variant added to `EdgeKind` forces an audit of every metric that filters on variant sets. Concrete symptom: if T8's classifier extends to recognize `Declarative::ProcessBuilder` bindings as incoming callers, `dead_code.fan_in` must be manually updated to count them, and forgetting silently produces false-positive dead-code findings.
4. **No `EdgeKind`-to-SQLite boundary type.** Reading a parse DB written by a newer engine version with an unknown edge kind string silently halts the load (`from_stable_str` returns `None`, caller skips). The edge is dropped with no count in the integrity caveats. Concrete symptom: post-T6, a gridseak-self scan with Rust Layer 2 edges loaded by a pre-T6 analysis binary would silently report zero Rust calls.

## 2. Non-goals

- **Not changing the `Provenance` shape.** `Provenance { source, confidence }` is load-bearing for T3 and T4 and already behaves correctly.
- **Not re-scoping `FrameworkKind` or `DeclarativeKind`.** Only variants with live emitters ship (plus `DeclarativeKind::Flow` as the already-documented placeholder). Planned-but-unemitted variants live in [`docs/04-architecture/EDGE_TAXONOMY.md`](../../../04-architecture/EDGE_TAXONOMY.md).
- **Not refactoring the resolver architecture.** The resolver still owns target-node-ID resolution. P1.d introduces a typed channel *between* extractor and resolver but does not move the resolver boundary.
- **Not extracting a new crate.** All rework lives inside existing crates (`graphengine-parsing`, `graphengine-analysis`).
- **Not touching T2's stable-ID work.** Content-based IDs are unrelated to edge kind taxonomy.

## 3. Five diagnostic questions

1. **Are we forcing one type to do two jobs?** Yes, twice. `EdgeKind` is currently doing in-memory domain and on-disk wire (→ P1.c splits into `EdgeKind` + `PersistedEdgeKind`). `CallSite` is doing both "a normal call" and "a framework-bound reference" via `Option<hint>` (→ P1.d splits into `UnresolvedReference { Call, FrameworkBinding, DeclarativeBinding }`).
2. **Is the trade-off between "hardcoded list" and "brittle update"?** Yes. Every metric today filters on a variant list. (→ P1.a inverts via predicates on `EdgeKind`.)
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** Yes. Extractor knows framework context; resolver knows target ID. (→ P1.d introduces the typed channel.)
4. **Is the trade-off about serialization format?** Yes. Hand-rolled `to_stable_str` vs `Debug`. (→ P1.b adopts `#[serde(tag, content)]`.)
5. **Is the trade-off between two modes of failure?** Yes, specifically at the SQLite load boundary: silently-drop-unknown vs halt-the-load. (→ P1.c's `PersistedEdgeKind::Unknown(String)` plus an `unknown_edges` caveat.)

All five fire. Four distinct dissolving moves correspond to four sub-reworks P1.a–P1.d.

## 4. Chosen shape

Four sub-reworks, sequenced predicate-first so every later sub-rework consumes predicates and not variant lists.

### 4.1 P1.a — predicate suite (dissolves Decision 5)

Add predicates on `EdgeKind` on the parsing side (`graphengine-parsing/src/domain/edge.rs`) and mirror on the analysis side (`graphengine-analysis/src/health/graph.rs`):

- `is_structural()` — existing, confirmed. Returns `!is_containment()`.
- `is_call_like()` — existing. Returns `matches!(self, Call | Framework(_) | Declarative(_))`.
- `is_containment()` — existing. Returns `matches!(self, Contains)`.
- `is_dependency()` — **new**. Returns `matches!(self, Import | Type | Uses)`.
- `is_inheritance()` — **new** (on parsing side; analysis-side mirror already has it in a subtly-different form — unify).

Audit every metric consumer and migrate to predicate-driven filtering:

- `graphengine-analysis/src/health/coupling.rs`
- `graphengine-analysis/src/health/cohesion.rs`
- `graphengine-analysis/src/health/blast_radius.rs`
- `graphengine-analysis/src/health/dead_code.rs`
- `graphengine-analysis/src/health/fan_metrics.rs`
- `graphengine-analysis/src/health/depth.rs` (already uses `is_call_like` — confirm)
- `graphengine-analysis/src/health/layers.rs` (already uses `is_call_like` — confirm)
- `graphengine-analysis/src/health/metric_status.rs` (already uses `is_call_like` — confirm)
- `graphengine-analysis/src/health/structural_classification.rs`
- `graphengine-analysis/src/health/graph.rs` (the `structural_edge_indices` / `clean_structural_edge_indices` builders)

Every `match kind { ... }` or `kind == EdgeKind::X` that implicitly assumes a variant set becomes a predicate call. Audit result is captured inline in the PR description — the diff itself is the audit trail.

### 4.2 P1.b — serde-tagged serialization (dissolves Decision 4)

In `graphengine-parsing/src/domain/edge.rs`:

```rust
#[derive(Serialize, Deserialize, ...)]
#[serde(tag = "kind", content = "sub")]
pub enum EdgeKind {
    Call, Contains, Import, Extends, Implements, Type, Uses,
    Framework(FrameworkKind),
    Declarative(DeclarativeKind),
}
```

Delete `EdgeKind::to_stable_str` and `EdgeKind::from_stable_str` (and the equivalent methods on `FrameworkKind` / `DeclarativeKind`).

Update `graphengine-parsing/src/infrastructure/storage/sqlite_repository.rs` to use `serde_json::to_string(&kind)` and `serde_json::from_str::<PersistedEdgeKind>(&s)` at the boundary (the latter introduced in P1.c).

Update `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` (created pre-rework in P3) to assert the new wire format with **literal pinned-wire-string assertions** per variant. Example:

```rust
assert_eq!(
    serde_json::to_string(&EdgeKind::Framework(FrameworkKind::VisualforcePage)).unwrap(),
    r#"{"kind":"Framework","sub":"VisualforcePage"}"#,
);
```

The diff in this file is the audit trail for the format migration.

### 4.3 P1.c — split domain type from wire type (dissolves Decision 1)

Introduce in `graphengine-parsing/src/domain/edge.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PersistedEdgeKind {
    Known(EdgeKind),
    Unknown(String),
}

impl From<EdgeKind> for PersistedEdgeKind { fn from(k: EdgeKind) -> Self { Self::Known(k) } }
impl TryFrom<PersistedEdgeKind> for EdgeKind {
    type Error = String;
    fn try_from(p: PersistedEdgeKind) -> Result<Self, Self::Error> {
        match p { PersistedEdgeKind::Known(k) => Ok(k), PersistedEdgeKind::Unknown(s) => Err(s) }
    }
}
```

Compile-time `Copy` assertion:

```rust
#[cfg(test)]
mod copy_invariant {
    use super::*;
    fn _assert_copy<T: Copy>() {}
    #[test]
    fn edge_kind_is_copy() { _assert_copy::<EdgeKind>() }
}
```

`PersistedEdgeKind` is **only** used at the SQLite boundary (`sqlite_repository.rs`). In-memory code everywhere else works on `EdgeKind`. Load path:

```rust
let persisted: PersistedEdgeKind = serde_json::from_str(kind_str)?;
match EdgeKind::try_from(persisted) {
    Ok(k) => /* insert into edges */,
    Err(unknown_str) => {
        warn!("dropped unknown edge kind {unknown_str}");
        unknown_edges_count += 1;
    }
}
```

Analysis side: the SQLite load in `graphengine-analysis/src/health/graph.rs` already skips unknown kinds (via the current `from_stable_str` returning `None`). Extend to **count** them and surface `unknown_edges_count` as `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1` (new constant in `report.rs`) in the integrity status when non-zero.

### 4.4 P1.d — typed UnresolvedReference channel (dissolves Decision 3)

In `graphengine-parsing/src/application/ports.rs`:

```rust
pub enum UnresolvedReference {
    Call(CallSite),
    FrameworkBinding(FrameworkBinding),
    DeclarativeBinding(DeclarativeBinding),
}

pub struct FrameworkBinding {
    pub framework: FrameworkKind,
    pub callee_name: String,
    pub from_node_id: String,
    pub location: Location,
    /// Free-text context: VF page name, LWC template path, etc.
    pub binding_source: String,
}

pub struct DeclarativeBinding { /* parallel structure */ }
```

Remove `edge_kind_hint: Option<EdgeKind>` from `CallSite`. The field is absent; `CallSite` becomes "a plain call site that does not carry framework context". Framework-aware emitters produce the matching enum variant.

Touched files (emitters and consumers):

- `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/vf_extraction.rs` — emits `UnresolvedReference::FrameworkBinding(FrameworkBinding { framework: FrameworkKind::VisualforcePage, ... })` instead of a hinted `CallSite`.
- `graphengine-parsing/src/syntax/language/apex/heuristic_resolver.rs` — `match`-dispatches on `UnresolvedReference` variant. The compiler enforces an arm per variant.
- `graphengine-parsing/src/syntax/language/apex/resolver_dispatch.rs` — same.
- `graphengine-parsing/src/syntax/language/apex/field_type_resolver.rs`, `field_type_resolver`-touching callers — update signature or rely on the new dispatch.
- `graphengine-parsing/src/infrastructure/lsp/resolvers/call_resolver_lsp.rs` — same.
- `graphengine-parsing/src/infrastructure/lsp/resolvers/type_resolver.rs` and `import_resolver.rs` — only touch if they consume `CallSite.edge_kind_hint`; audit confirms if yes.
- `SyntaxResults.call_sites: Vec<CallSite>` becomes `SyntaxResults.references: Vec<UnresolvedReference>`. Every consumer updates. The rename is deliberate — the old name is factually wrong post-rework.
- `graphengine-parsing/tests/apex_resolver_r23_a5_vf_fixtures.rs` — assertions move from "hint field is `Some(Framework(VisualforcePage))`" to "the emitted `UnresolvedReference` is the `FrameworkBinding` variant with `FrameworkKind::VisualforcePage`".

Exhaustiveness proof step (documented, not shipped): temporarily delete one arm in the resolver's `UnresolvedReference` match. Observe the compile error. Revert. Evidence is a paste of the compiler diagnostic in the PR description.

**Exhaustiveness-proof execution log (P1.d)**. Performed during P1.d landing. The canonical dispatch is `UnresolvedReference::edge_kind()` in `graphengine-parsing/src/application/ports.rs`. Commenting out the `Self::DeclarativeBinding(db) => EdgeKind::Declarative(db.declarative)` arm and rebuilding yielded:

```text
error[E0004]: non-exhaustive patterns: `&UnresolvedReference::DeclarativeBinding(_)` not covered
For more information about this error, try `rustc --explain E0004`.
error: could not compile `graphengine-parsing` (lib) due to 1 previous error
```

The arm was then restored. This demonstrates the dissolving-element property: the variant is the typed channel, not a hint — any future reader that fails to handle a variant is a compile-time failure, not a silent runtime regression.

## 5. Acceptance criteria

All criteria gated on the **single atomic PR** merge (see §7).

1. `cargo build --all-targets` is green across the workspace.
2. `cargo test --workspace` is green. **P3 fixtures pass without assertion edits.** If any P3 assertion needs to change to survive P1, that signals the invariant was tested at the wrong level; the rework does not land until the underlying behavioural invariant is re-pinned.
3. `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` asserts the serde wire format with literal-string pins for every shipping variant; a naive derive-order change or `#[serde(rename)]` addition trips the test.
4. `graphengine-parsing/tests/apex_resolver_r23_a5_vf_fixtures.rs` asserts `Edge.kind == EdgeKind::Framework(FrameworkKind::VisualforcePage)` by **enum pattern match**, not `to_stable_str` comparison.
5. A synthesized parse DB with an unknown edge kind string (e.g., `{"kind":"FutureLayer6","sub":"Whatever"}`) round-trips: the load skips the edge, `unknown_edges_count` is non-zero, and the resulting `HealthReport.integrity_status.schema_caveats` contains `CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1`. New integration test: `t1c_unknown_edge_kind_surfaces_caveat.rs`.
6. Compile-time test `fn _assert_copy<T: Copy>() {}` proves `EdgeKind` remains `Copy`.
7. Compile-time exhaustiveness on `UnresolvedReference`: deleting a resolver arm fails to compile. Evidence: the PR description includes the compiler diagnostic text from the delete-and-revert step.
8. Every metric consumer listed in P1.a uses predicates; `rg "== EdgeKind::" graphengine-analysis/src/health` returns zero matches after the migration (the PR description includes this grep as audit evidence).

## 6. Test plan

| Test | Location | Owns |
| --- | --- | --- |
| P3 divergent-fixture gap (existing, extended) | `graphengine-analysis/tests/t3f_fidelity_gap_regression.rs` | Dual-metric plumbing on all 8 metrics. Must pass unchanged. |
| P3 uniform-High zero-gap (existing) | `graphengine-analysis/tests/t3_uniform_high_zero_gap.rs` | Zero-gap invariant. Must pass unchanged. |
| P3 T4 canary tier (existing, 6 fixtures) | `graphengine-analysis/tests/t4_canary_tier_classification.rs` | Tier classification + boundary directions. Must pass unchanged. |
| P3 T1 round-trip pin (existing, updated in P1.b) | `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` | Wire format literal strings. **This file is edited in P1.b** to swap pre-rework hand-rolled strings for post-rework serde-tagged strings. |
| P1.a predicate unit tests | in-module `#[cfg(test)]` inside `domain/edge.rs` and mirror in `health/graph.rs` | Per-predicate variant membership. |
| P1.b round-trip property test | `graphengine-parsing/tests/t1_edgekind_roundtrip.rs` (same file, after update) | Every variant round-trips via `serde_json`. |
| P1.c unknown-edge behavioural (new) | `graphengine-parsing/tests/t1c_unknown_edge_kind_surfaces_caveat.rs` | Forward-compat path + caveat surfaces. Synthesize a DB with unknown kind, load via `run_analysis`, assert caveat + count. |
| P1.d `UnresolvedReference` exhaustiveness (new) | in-module `#[cfg(test)]` inside `application/ports.rs` | Compile-time: a helper `fn` does a `match` on every variant and returns a `&'static str`. Adding a variant without updating the helper fails to compile. |
| VF binding E2E (existing, rewired) | `graphengine-parsing/tests/apex_resolver_r23_a5_vf_fixtures.rs` | Framework edge variant is matched by enum pattern, not string. |

All tests run in the same `cargo test --workspace` invocation. No separate run.

## 7. Rollback criterion

Single named signal: **any P3 fixture assertion requires edit to pass post-P1**, AND the failure is behavioural (invariant broken) rather than implementation-shape (assertion needs migration).

If this signal fires, revert to the tag taken immediately before P1's merge. The revert is clean because P1 ships as one atomic PR (§7 "Shipping discipline" in the master plan). Nothing downstream has built on top of the rework yet.

If the failure is clearly implementation-shape and the invariant is still satisfied by the new code, migrate the P3 assertion, document the migration in the PR description under a "P3 assertion shape migration" heading, and land. This escape hatch is deliberately narrow; the default reading of a P3 assertion failure is "the rework broke something".

## 8. Out-of-scope follow-ups

- **Analysis-side `EdgeKind` type unification.** The analysis crate has its own `EdgeKind` mirror (`graphengine-analysis/src/health/graph.rs`) kept in sync by convention. P1 does not unify the two types; they remain manual mirrors, with P1.a ensuring the predicate surface is consistent. A future ticket extracts `EdgeKind` to a shared crate (`graphengine-domain-common` or similar) and has both `parsing` and `analysis` depend on it; that change is a workspace-layout concern, not a T1 concern.
- **`UnresolvedReference` for non-Apex framework dispatch.** LWC, Aura, Trigger, InboundEmail all sit in `docs/04-architecture/EDGE_TAXONOMY.md` as planned-but-unemitted. Each future emitter adds its variant to `FrameworkKind` AND produces `UnresolvedReference::FrameworkBinding` with the matching variant. That follow-up is per-emitter and not blocked by P1.
- **Rust Layer 2 consumption of `UnresolvedReference`.** T6 will define a `Layer2Adapter` trait that emits `UnresolvedReference::Call` with `Provenance { source: Layer2, confidence: High }`. Trait sketch lives in T6's design doc; no code here.
- **Pre-P1 parse.db compat read.** A database written by pre-P1 engines contains hand-rolled strings (`"Framework:VisualforcePage"`). The SQLite load path in P1.c may need a compat-read arm that recognises both old and new formats for a grace period. Decision: **omit compat-read**. P1 ships with a hard schema-version bump in SQLite metadata; pre-P1 DBs force a re-parse. Rationale: the sprint has zero durable parse DBs in production (canaries regenerate on every run), so compat-read is pure cost. If a production customer arrives with old DBs before P1 ships, re-open this decision.
