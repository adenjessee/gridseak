# T6 — Rust Layer 2 adapter via `ra_ap_ide`

> **Reproducing historical numbers / paths cited below.** Neither the historical baseline JSONs / calibration outputs nor the rev6.1 byte-identical regression fixture referenced in this document are tracked in git — both live as sha256-pinned GitHub release assets. Fetch on demand with `scripts/setup.sh historical-baselines` (rev3..rev11 evidence, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/baseline-archive-2026-05-18)) and `scripts/setup.sh fixtures` (rev6.1 regression fixture, [release](https://github.com/adenjessee/gridseak-graphengine/releases/tag/regression-fixtures-2026-05-19)). All artifacts are pinned in `experiments/artifacts.lock`. The active build/test loop does not require any of them.

Authored against [`TEMPLATE.md`](TEMPLATE.md). Every section is answered, no skipped headings.

---

## 1. Problem statement

No language in the engine ships a working Layer 2 (semantic-grade) call resolver today. The sprint primer §6.T6 states this directly and the Round 5 audit numbers confirm it: `commons-lang` ships 0.2 % `Call / Lsp / High` edges, `django-site` ships 0 %, `serilog` ships 0.8 %. TypeScript's `tsserver` adapter exists in `graphengine-parsing/src/infrastructure/lsp/` but runs over the LSP wire protocol, which means every failure mode of stdio-piped JSON-RPC — process lifecycle flake, response deserialization drift, timeout tuning — is part of the hot path. The measured fidelity-tier classifier (T4) therefore reports `SyntacticOnly` on every canary that is not Apex + NPSP.

Concrete consequences this task exists to remove:

- **`Confidence::High` is unreachable for Rust call edges.** The Rust extractor at `graphengine-parsing/src/syntax/language/extractors/rust.rs` emits only `CallSite`s; the resolver stage (`graphengine-parsing/src/infrastructure/lsp/call_resolver.rs`) resolves them via the shared heuristic path and stamps `Provenance { source: Heuristic, confidence: Low }`. No code path exists today that can attach `Provenance { source: Lsp, confidence: High }` to a Rust `Call` edge.
- **`MeasuredFidelityTier::Authoritative` is unreachable on any Rust scan.** The tier derivation (`graphengine-analysis/src/health/…`) classifies `Authoritative` as ≥ 80 % `High` confidence on Calls. With zero `High`-confidence Rust edges possible, the upper tier is a dead branch for our own codebase.
- **`gridseak-self` dogfooding is blocked.** The Universal-Fidelity primer §4 names `gridseak-self` as the canary that should exercise Layer 2 end-to-end on a repository the team already reads fluently. Without a Rust adapter we cannot dogfood our own engine at Layer 2 authority.

## 2. Non-goals

- **LSP wire-protocol resolver for Rust.** The entire point of `ra_ap_ide` is to link the rust-analyzer IDE crate as a library and skip the wire protocol. A `rust-analyzer` subprocess adapter is explicitly not what we are building. Existing TypeScript `tsserver` subprocess code stays where it is.
- **Full rustdoc-quality call graph across every macro expansion.** `ra_ap_ide` gives us name-resolved references and call-resolved Method/FnCall targets out of the box. Proc-macro-generated bodies are out of scope for v1 — the `ra_ap_ide` API surface for proc-macro expansion is unstable and differs across toolchain versions. Limitation is called out in the acceptance criteria and tested as a **known-miss** (see §6.3).
- **Any non-Rust language.** Java / Kotlin / Go / Python Layer 2 adapters are separate tasks, not T6 follow-ons. T6 is specifically the Rust pathfinder.
- **Dropping the existing Rust heuristic resolver.** The heuristic path stays. `ra_ap_ide` is a new `Confidence::High` source that runs *ahead* of the heuristic pass; anything `ra_ap_ide` cannot resolve flows into the existing heuristic fallback. Two-tier composition, not replacement.
- **Emitting `Call / Lsp / High` edges from data that `ra_ap_ide` itself flagged as speculative.** Rust-analyzer's API surfaces `ReferenceCategory`, `NavigationTarget`, and resolver-error variants; any ambiguous result is downgraded to `Medium` or discarded. We do not launder low-authority data into `High`.
- **Cross-crate resolution across the workspace for a single-crate scan.** `ra_ap_ide` requires a project-model (usually driven by `cargo metadata`); single-file scans land on an implicit one-crate project model. Multi-crate workspace resolution is tested on `gridseak-self` (itself a workspace), but single-file dogfood targets stay single-crate.

## 3. Five diagnostic questions

Answered before writing code. See `NEW_ENGINEER_PRIMER.md` §5.5 and §8.0.

1. **Are we forcing one type to do two jobs?** **Yes.** Today the shared `LspResolver` trait is doing (a) "subprocess LSP over stdio" and (b) "semantic call resolution source". The wire protocol is a transport; semantic-resolution authority is a guarantee. `ra_ap_ide` gives us the guarantee without the transport, which proves the two are separable. §4 introduces a new trait `SemanticResolver` that the `ra_ap_ide` adapter implements directly (no LSP wire involvement) and reframes the existing `LspResolver` as one implementation of the new trait.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** **No.** Confidence and source enums are already stable (`Confidence::High` / `ProvenanceSource::Lsp`). No list-vs-update axis.
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** **No.** The existing pipeline already separates extraction (tree-sitter) from resolution (heuristic / semantic). `ra_ap_ide` slots into the resolution stage; the earlier/later boundary is already correct post-P1.d (`UnresolvedReference` enum as the typed channel).
4. **Is the trade-off about serialization format?** **No.** T6 introduces no new on-disk format. Edges stay shape-compatible with today; only their `Provenance` field changes.
5. **Is the trade-off between two modes of failure?** **Yes — and it dissolves.** The alternative framing ("crash the scan when `ra_ap_ide` fails" vs "fall back to heuristic silently") is a false choice. The dissolving element is a **measured fallback**: every `ra_ap_ide` failure increments a named counter (`semantic_resolver_errors_total`), the scan falls back to the heuristic for that reference, and the integrity caveats list the counter if non-zero. Observable fallback, not silent or catastrophic.

Interpretation: **questions 1 and 5 are `Yes`**. §4 supplies both dissolving elements.

## 4. Chosen shape

### Types introduced or changed

Add a new crate `graphengine-ra-ide-adapter` (workspace member) that links `ra_ap_ide` and provides one public type:

```rust
// graphengine-ra-ide-adapter/src/lib.rs
pub struct RustAnalyzerSemanticResolver {
    analysis_host: AnalysisHost, // ra_ap_ide::AnalysisHost
    project_model: ProjectWorkspace,
}

impl RustAnalyzerSemanticResolver {
    pub fn from_workspace_root(root: &Path) -> Result<Self, SemanticResolverError> { /* … */ }
    pub fn resolve_reference(
        &self,
        reference: &UnresolvedReference,
    ) -> Result<Option<ResolvedTarget>, SemanticResolverError>;
}

pub struct ResolvedTarget {
    pub node_fqn: String,
    pub confidence: Confidence, // High unless ra_ap_ide flags ambiguity
}

#[derive(Debug)]
pub enum SemanticResolverError {
    ProjectModelLoad(String),
    AnalysisSnapshot(String),
    AmbiguousReference { candidates: usize },
    MacroExpansionUnsupported,
}
```

Generalise `graphengine-parsing`'s resolver composition layer. Rename `LspResolver` trait's role clarifier (doc-only, no signature change required) so the existing TypeScript subprocess implementation is one of several:

```rust
// graphengine-parsing/src/infrastructure/lsp/resolver.rs — updated doc comment
/// Historically this trait represented "LSP-subprocess-backed
/// semantic resolution". After T6 it represents any authoritative
/// (Confidence::High-eligible) semantic resolver — whether driven
/// by a subprocess LSP (TypeScript) or by a library-linked IDE
/// crate (Rust, via `ra_ap_ide`). The transport is an implementation
/// detail; the contract is "each resolved reference has semantic-
/// grade authority from a name resolver that consulted the compiler
/// model of the code, not a surface-form heuristic."
```

Wire a language-keyed registry: `fn build_semantic_resolver(language: &str, root: &Path) -> Option<Arc<dyn LspResolver>>`. Returns `Some(RustAnalyzerSemanticResolver::into_lsp_resolver(…))` for Rust, `Some(TsServerResolver::new(…))` for TypeScript, `None` otherwise. The orchestrator (post-T5) asks the registry; if `None`, semantic resolution is skipped and the heuristic path still runs.

### Data flow

```
syntax extraction  (UnresolvedReference { Call / FrameworkBinding / DeclarativeBinding })
   │
   ▼
semantic resolver (new):
   · Rust      → RustAnalyzerSemanticResolver (links ra_ap_ide)
   · TypeScript → TsServerResolver (existing LSP-wire)
   · others    → None
   │
   │ Each resolver returns (resolved: Vec<Edge { provenance: Lsp/High }>,
   │                        unresolved: Vec<UnresolvedReference>)
   ▼
heuristic fallback resolver (existing)
   · consumes unresolved references
   · emits Edge { provenance: Heuristic/Low }
   ▼
aggregator → Graph
```

### Compile-time guarantees

- **`ra_ap_ide` version pinned.** Workspace `Cargo.toml` adds `ra_ap_ide = "=0.0.X"` (exact version, not caret) plus `ra_ap_project_model`, `ra_ap_base_db`, `ra_ap_paths`, `ra_ap_vfs`. Exact pin is deliberate because the `ra_ap_*` family ships breaking API changes per release. An MSRV or rust-analyzer bump therefore requires an explicit `Cargo.toml` edit, not a `cargo update` surprise.
- **MSRV bump to the crate's declared minimum.** `rust-toolchain.toml` `channel = "stable"` becomes a specific version (e.g. `1.91.0`) pinned in the file. Sprint primer §6.T6 already names 1.91 as the target; we confirm the exact minimum from the `ra_ap_ide` release notes for the pinned version at implementation time and set the toolchain pin to match.
- **`RustAnalyzerSemanticResolver` is behind a feature flag** (`features = ["rust-layer2"]`) on `graphengine-parsing`. Default on for workspace builds; build-flag-gatable for downstream consumers who cannot tolerate the MSRV bump. Feature flag is removed once rust-analyzer stabilises its library API. Tracked as a post-T6 follow-up in `docs/workstreams/universal-fidelity/FOLLOWUPS.md` (`UF-FU-002 — ra_ap_ide library API stability gate`). The library-API-stability question is not in scope for any existing `DISCOVERY_REPORT.md` entry and will be opened as a dedicated discovery spike after T6 ships first-pass.

### Predicate contracts

No change to `EdgeKind` predicates. T6 emits `EdgeKind::Call` with a different `Provenance`; the edge-kind taxonomy is unchanged.

## 5. Acceptance criteria

Every criterion is behavioural, grep-able, falsifiable.

1. `cargo test -p graphengine-ra-ide-adapter` green. Tests live alongside the new crate.
2. A synthetic two-file Rust fixture at `graphengine-parsing/tests/rust_layer2_fixtures/two_file_call/` produces at least one `Edge { kind: Call, provenance: { source: Lsp, confidence: High } }` when scanned. Assertion lives in new integration test `graphengine-parsing/tests/t6_rust_layer2_semantic.rs` under function name `semantic_resolver_emits_lsp_high_call_edge`.
3. On a repository with no `Cargo.toml` (pure `.rs` scratch file), the scan completes with zero panics and emits `CAVEAT_SEMANTIC_RESOLVER_PROJECT_MODEL_MISSING_V1` in the health report. The heuristic fallback still runs.
4. `gridseak-self` end-to-end dogfood target: `cargo run -p graphengine-diagnostic -- scan . --semantic-resolver=rust-analyzer --emit=health-summary` produces a `MeasuredFidelityTier::Authoritative` (or `HeuristicPrimary` at minimum) classification with the `High` edge share ≥ 20 % of `Call` edges in the analysis graph. Specific percentage target of 20 % is derived from the T4 threshold spread: 40 % is `HeuristicPrimary`'s floor, but we set the dogfood floor at 20 % to have honest evidence of semantic-grade emission without pretending we are at `HeuristicPrimary` on day 1.
5. **Kill criterion (from D4, tightened Gate 1.1).** If the resident-set-size during a `gridseak-self` scan exceeds **200 MB** above the baseline (baseline = same scan with `--semantic-resolver=none`), or end-to-end scan wall-clock exceeds **0.5 seconds per kloc** on a production-only file filter, T6 fails the dogfood gate. The 0.5 s/kloc ceiling replaces the original 10 s/kloc figure; the tightening was forced by the Gate 1.1 production-file re-measurement (see §9.B3.3 and `FOLLOWUPS.md::UF-FU-011` closure) which found a 173× headroom on the original criterion. The new value is `max(3 × baseline, 0.5 s/kloc) = max(188 ms/kloc, 500 ms/kloc) = 500 ms/kloc`, giving rust-analyzer linkage ~8× its current baseline cost before tripping. Both numbers are sprint-level kill criteria referenced in `DISCOVERY_REPORT.md` §D4 `"Rust Layer 2 resource envelope"`. Measured via `graphengine-parsing parse` + `ge-analyze` wall-clock plus `/usr/bin/time -l` for RSS on macOS.
6. `rg -n 'edge_kind_hint' graphengine-ra-ide-adapter/` returns zero — the new crate consumes `UnresolvedReference` via its typed API, not the deprecated hint field.
7. `cargo build --all-targets --features rust-layer2` and `cargo build --all-targets --no-default-features` both green. The feature flag is real.
8. MSRV bump lands in `rust-toolchain.toml` in the same PR as the `ra_ap_ide` dependency. CI on the PR passes on the pinned toolchain.

## 6. Test plan

Three tiers.

### 6.1 Unit tests

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `semantic_resolver_error_variants_are_exhaustive` | `graphengine-ra-ide-adapter/src/lib.rs #[cfg(test)]` | Round-trip match on every `SemanticResolverError` variant; locks exhaustiveness. |
| `resolved_target_high_confidence_default` | same file | `ResolvedTarget::confidence` defaults to `Confidence::High` only when `ra_ap_ide` returned a single unambiguous target. |
| `ambiguous_reference_downgrades_to_medium` | same file | When `ra_ap_ide` returns N>1 candidates, the adapter emits `Medium` (not `High`). |

### 6.2 Integration tests (behavioural)

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `semantic_resolver_emits_lsp_high_call_edge` | `graphengine-parsing/tests/t6_rust_layer2_semantic.rs` | Two-file fixture produces at least one `Edge { provenance: Lsp/High, kind: Call }`. |
| `semantic_resolver_unresolved_falls_through_to_heuristic` | same file | Fixture with an unresolvable reference (e.g. method from a crate not in the project model) produces a `Heuristic/Low` edge, not nothing. |
| `semantic_resolver_missing_project_model_surfaces_caveat` | same file | Scan with no `Cargo.toml` emits `CAVEAT_SEMANTIC_RESOLVER_PROJECT_MODEL_MISSING_V1` in `HealthReport.integrity_caveats`. |
| `semantic_resolver_known_miss_proc_macro` | same file | Fixture that uses a proc-macro attribute produces *only* heuristic edges for calls inside the expanded body, with no panic. Asserts the **known limitation**. |

### 6.3 Regression fixture

The `gridseak-self` dogfood target is the canary. It is a live workspace with > 10 kloc of Rust across 7 crates. A T6 regression that (a) silently drops `High`-confidence emission, (b) misroutes fallback, or (c) blows the resource envelope all surface there.

The dogfood run is wired into CI as a separate non-blocking job: it runs on every PR that touches `graphengine-parsing/` or `graphengine-ra-ide-adapter/`, records the measured tier and `High`-share to a timeline file, and fails the job if the kill criteria from §5.5 trip. The job is non-blocking until T6 stabilises (projected 2 sprints), then promoted to blocking.

## 7. Rollback criterion

**Single named signal:** the `gridseak-self` dogfood CI job emits `MeasuredFidelityTier::SyntacticOnly` for two consecutive main-branch runs *or* trips either kill criterion (RSS > 200 MB above baseline, wall-clock > 10 s/kloc).

If that happens:

1. Revert the `ra_ap_ide` dependency + feature-flag-gated code path.
2. Do not revert the `LspResolver` trait doc-comment clarification, the `graphengine-ra-ide-adapter` crate skeleton, or the MSRV bump — those are precondition changes, not T6 work themselves.
3. File a T6.b rework ticket naming the kill-criterion that tripped.

This narrow criterion exists because a T6 regression is almost certainly either "the library linkage stopped producing `High` edges" or "the library linkage is too expensive." Both are observable in the single dogfood number.

## 8. Out-of-scope follow-ups

- **Java Layer 2 via `jdtls`-as-library.** Eclipse JDT ships a usable-as-library core (`org.eclipse.jdt.core`), but JVM interop from Rust is non-trivial. Separate future task, not a T6 follow-on. Proposed name: **T12 — Java Layer 2 adapter**.
- **Python Layer 2 via `jedi`-as-embedded.** Pure Python library, would require PyO3 embedding or subprocess. Separate task. Proposed name: **T13 — Python Layer 2 adapter**.
- **Proc-macro expansion support.** Blocked on `ra_ap_ide` stabilising the expansion API. Tracked in `docs/workstreams/universal-fidelity/FOLLOWUPS.md` as `UF-FU-003 — proc-macro expansion viability`. Not yet in `DISCOVERY_REPORT.md`; will be promoted to a discovery question if T6 dogfood shows proc-macro-heavy crates systematically underemitting `High` edges.
- **Workspace-wide project-model caching.** Today each scan rebuilds the `ra_ap_ide` `AnalysisHost` from scratch. For `gridseak-self` that is tolerable; for repos > 100 kloc it may not be. Deferred pending measurement on the kill-criterion envelope. Proposed name: **T6.c — semantic-resolver project-model cache**.
- **Cross-crate edge emission when scanning a single-crate slice of a larger workspace.** The v1 adapter uses a single-crate project model for single-file scans. Cross-crate emission requires plumbing `cargo metadata` through the scan entry point. Proposed name: **T6.d — cross-crate semantic resolution**.
- **Replacing the TypeScript `tsserver` wire adapter with a library-grade linkage.** TypeScript's `tsserver` is the only officially supported interface; there is no library-grade equivalent today. Deferred indefinitely; tracked in the sprint's long-tail notes, not a T6 follow-on.

---

## 9. Pre-flight discovery (B1-B3)

Appended during the P5 hardening plan's Phase B. This section records the
measured facts the T6 implementation plan consumes as inputs. Every number
here was captured on `2026-04-19` against the workspace's live toolchain
and against crates.io's registry snapshot on the same day; re-measure
before T6 implementation starts if more than four weeks have elapsed.

### 9.B1. `ra_ap_ide` version and MSRV

Data source: `https://crates.io/api/v1/crates/ra_ap_ide` and sibling
crate responses for `ra_ap_project_model`, `ra_ap_vfs`. All five crates
(`ra_ap_ide`, `ra_ap_project_model`, `ra_ap_base_db`, `ra_ap_paths`,
`ra_ap_vfs`) are auto-published weekly from
`github.com/rust-lang/rust-analyzer` and move in lockstep — same version
number, same MSRV, same release timestamp.

**Latest published version (2026-04-19):** `0.0.329` (released
`2026-04-20`). MSRV: `1.91`.

**Last MSRV-1.88 version:** `0.0.307` (released `2025-11-24`). MSRV
transition point: `0.0.308` (released `2025-12-01`) bumped the declared
`rust-version` from `1.88` to `1.91`. Every version since has kept
`1.91`. Scan window inspected: `0.0.305` through `0.0.329` (25 recent
releases).

**Current workspace toolchain:** `rustc 1.90.0 (1159e78c4 2025-09-14)`.
`rust-toolchain.toml` pins `channel = "stable"`, so the effective
version is whatever `rustup` resolved `stable` to on the local machine
at `rustup update` time. The 1.90.0 value recorded above is the
developer-machine reading; CI may be on a different stable version at
any given moment.

**Hard finding (MSRV gap — one minor version).** Adopting `ra_ap_ide >=
0.0.308` at the workspace's current effective toolchain (`1.90.0`)
fails to compile. Two remediations exist, each with a named
trade-off:

| Option | Change | Cost | Risk |
| :--- | :--- | :--- | :--- |
| **A. Toolchain bump** | Replace `channel = "stable"` with `channel = "1.91"` (or a newer explicit pin) in `rust-toolchain.toml` at the same time as the `ra_ap_*` dependency lands. | Every developer runs `rustup install 1.91` once. CI image rebuild. | Minimal: `1.91` has been stable since ~2025-11-01 per the rust release cadence; 5+ months of bake time on the wider ecosystem. |
| **B. Pin to the last 1.88-compatible version** | `ra_ap_ide = "=0.0.307"` (and matching siblings). | No toolchain change. | Five months behind HEAD on a weekly-cadence library; diverges further every week. Defeats the "link the library so we track rust-analyzer" premise of T6. |

**Initial recommendation (superseded by 9.B2 findings below).** Option A
was the initial recommendation here. The B2 spike then uncovered a
transitive-dependency constraint (newer `ra-ap-rustc_index` versions
use the unstable `new_zeroed_alloc` library feature) that forces a
**third remediation**:

| Option | Change | Status |
| :--- | :--- | :--- |
| **C. Combined pin (actual T6 recommendation)** | `rust-toolchain.toml` → `channel = "1.91"` **and** `ra_ap_ide = "=0.0.307"` (and matching siblings). | Only stable-rust-compatible combination measured in B2. Both pins are load-bearing; removing either re-breaks the build. |

See `9.B2` below for the measurement that ruled out pinning at
`=0.0.329` (latest).

**Note on `ra_ap_*` API stability.** All five crates ship breaking
changes per weekly release. This is the sprint primer's §6.T6 premise
and is already tracked as
[`FOLLOWUPS.md::UF-FU-002`](../FOLLOWUPS.md) ("`ra_ap_ide` library API
stability gate"). The `=0.0.X` exact-version pin in §4 of this doc is
non-negotiable. A `0.0.328 → 0.0.329` bump must be an explicit
`Cargo.toml` edit and go through a review.

### 9.B2. `ra_ap_ide` dep-chain spike

Spike lives at `scratch/ra_ap_ide_spike/` — throwaway, `.gitignore`d as
of this addendum, deleted at the end of Phase B. Shape: a one-binary
Cargo project depending on `ra_ap_ide`, `ra_ap_load-cargo`,
`ra_ap_project_model`, `ra_ap_vfs`, `ra_ap_paths`. Fixture: a 2-file
Rust crate (`fixture/src/lib.rs` + `fixture/src/main.rs`) with one
cross-file call (`main` → `lib::callee`).

**Scope discipline.** The spike answers Q1 (does the dep graph compile
on the target toolchain) and Q3 (resource footprint floor). It does
**not** answer Q2 (does `goto_definition` return a resolved target) —
constructing the `FilePosition` for the exact offset of the `callee()`
invocation requires a syntax-walk that is T6 implementation work, not
spike work. The spike's honest floor here is: the dep chain links, the
workspace loads, and the VFS enumerates the two files. Semantic
resolution itself is validated in T6's integration tests.

#### 9.B2.1. Combination matrix

Three version × toolchain combinations were measured. Results are
reproducible from the spike directory.

| `ra_ap_ide` pin | Toolchain | Outcome | Failure cause (where applicable) |
| :--- | :--- | :--- | :--- |
| `=0.0.329` | `1.91.1` | **FAIL** (build) | Transitive `ra-ap-rustc_index 0.160.0` uses the unstable library feature `new_zeroed_alloc` at crate-root. Tracking issue [`rust-lang/rust#129396`](https://github.com/rust-lang/rust/issues/129396); stabilization PR [`#144091`](https://github.com/rust-lang/rust/pull/144091) opened 2025-07-17 and is **still open / unmerged** as of the B2 run. Any Rust stable channel therefore rejects the crate. |
| `=0.0.307` | `1.90.0` | **FAIL** (resolve) | Transitive `cargo-platform 0.3.3` declares `rust-version = "1.91"`. Cargo refuses to resolve under 1.90. |
| `=0.0.307` | `1.91.1` | **PASS** | Clean compile. Spike runs to completion. See 9.B2.2. |

**Consequences for T6 planning:**

1. **Pinning `=0.0.307` is not a "recovery path" — it is the primary
   pin.** The version recommendation in 9.B1 above (pin latest) is
   contradicted by the ground truth here; `=0.0.307` is the newest
   `ra_ap_ide` release that compiles on any released stable-channel
   Rust toolchain today. Later releases (`0.0.308` through `0.0.329`)
   drag in `ra-ap-rustc_index` versions that require nightly.
2. **MSRV bump to 1.91 is still mandatory** — even for `=0.0.307`,
   because `cargo-platform 0.3.3` is pulled in transitively through
   `ra_ap_load-cargo`'s project-model path. "Stay on 1.90 by pinning
   old `ra_ap_ide`" is not a viable option.
3. **When `new_zeroed_alloc` stabilises,** revisit the pin. Stability
   lands into some future stable (probably 1.92 or later) once PR
   `#144091` merges; at that point newer `ra_ap_ide` becomes
   stable-compatible and the 5-month version-lag on `=0.0.307` can be
   closed. Tracked as
   [`FOLLOWUPS.md::UF-FU-008`](../FOLLOWUPS.md) (`ra_ap_ide version-pin
   unblock`). This is a new follow-up row; it did not exist before the
   B2 spike measured the constraint.

#### 9.B2.2. Measurements at `=0.0.307` on 1.91.1

All numbers on `aarch64-apple-darwin`, release profile, 8-core M-series
machine, cold cargo cache for the build row and a warm run for the
binary row.

| Axis | Value | Notes |
| :--- | :--- | :--- |
| Dep-chain build (cold) | **1 m 05 s** | Single-threaded-linking phase dominates the tail. |
| Dep-chain rebuild after spike-source-only edit | 1.35 s | Incremental path. |
| Release binary size (`target/release/spike`) | **18 MB** | Statically linked; includes rust-analyzer IDE, project-model, HIR + type-inference libraries. This is the floor for any shipped tool that embeds `ra_ap_ide`. |
| `load_workspace_at` wall-clock on the 2-file fixture | **135 ms** | Includes sysroot discovery, `cargo metadata` subprocess, VFS population. Sysroot discovery is ~half. |
| Binary end-to-end wall-clock | **0.42 s** | Library load + `load_workspace_at` + VFS iteration + print. |
| Maximum resident set size | **32 MB** | Measured via `/usr/bin/time -l`. This is the library linkage + workspace-load floor; real T6 runs with `goto_definition` walks will land higher. |

#### 9.B2.3. API shape used

The concrete APIs the spike exercised, for the T6 implementation plan
to build on:

| Crate | API | Usage |
| :--- | :--- | :--- |
| `ra_ap_load_cargo` | `load_workspace_at(root: &Path, &CargoConfig, &LoadCargoConfig, &dyn Fn(String) + Sync) -> Result<(RootDatabase, Vfs, Option<ProcMacroClient>)>` | Primary entry point. T6 calls this with the target workspace's `Cargo.toml`. |
| `ra_ap_project_model` | `CargoConfig::default()` | Accepts default env + no-sysroot-override. T6 may need to customise `sysroot_src` if the target workspace uses a non-standard toolchain. |
| `ra_ap_load_cargo` | `LoadCargoConfig { load_out_dirs_from_check: bool, with_proc_macro_server: ProcMacroServerChoice, prefill_caches: bool }` | Spike set all three off to keep footprint minimum; T6 should measure whether `prefill_caches = true` trades memory for `goto_definition` latency. |
| `ra_ap_vfs` | `Vfs::iter() -> impl Iterator<Item = (FileId, VfsPath)>` | Used to walk discovered files; T6 will also need `vfs.file_id(&VfsPath)` for reverse lookup when binding scanned offsets to `FileId`s. |
| **NOT YET EXERCISED** | `AnalysisHost::default()` → `apply_change` → `analysis()` → `Analysis::goto_definition(FilePosition, &GotoDefinitionConfig)` | This is the T6 implementation path. The `AnalysisHost` does not expose a public constructor that wraps a `RootDatabase`, so T6 must either (a) drive queries through the `RootDatabase` returned by `load_workspace_at` directly (if `ra_ap_ide` exposes the query traits on it — to confirm during T6 implementation) or (b) bootstrap a fresh `AnalysisHost` via `default()` + an equivalent change application. This is a concrete **T6-implementation open question**, not a spike gap. |

#### 9.B2.4. Spike discipline

- Spike lives at `scratch/ra_ap_ide_spike/`; `scratch/` is now
  `.gitignore`d at the workspace root.
- Spike compile artefacts (`target/`) are likewise gitignored.
- No production crate under `graphengine-*/` was touched during B2.
  `cargo test --workspace` at workspace HEAD remains green.
- Spike deletion: Phase B4 closes with either a `git clean` of
  `scratch/` or a named decision to keep the spike around as future
  reference; either is acceptable because `scratch/` cannot ship.

### 9.B3. `gridseak-self` baseline measurement

Executed 2026-04-19 on `aarch64-apple-darwin`, release binaries rebuilt
at workspace HEAD after Phase A landed, no semantic resolver (since T6
is not yet implemented). Invocation path used:

```bash
./target/release/graphengine-parsing \
    --configs-dir graphengine-parsing/configs \
    parse --root . --lang rust \
    --db experiments/results/gridseak-self-baseline/parse.db \
    --clear

./target/release/ge-analyze \
    --db experiments/results/gridseak-self-baseline/parse.db \
    --output experiments/results/gridseak-self-baseline/baseline.json
```

**Plan-vs-reality note.** The P5 plan's §B3 named a single-command
invocation (`cargo run -p graphengine-diagnostic -- scan . --emit=health-summary`).
That binary does not exist in the workspace today —
`graphengine-diagnostic` is a library crate, and parsing + analysis are
two separate binaries (`graphengine-parsing` and `ge-analyze`). Using
the real two-binary invocation does not change the measurement intent
but it is a concrete B3 fidelity gap worth recording so the next reader
is not surprised. If T6 wants a single-command baseline, file it as a
separate follow-up; it is not a T6 prerequisite.

#### 9.B3.1. Measured baseline numbers

| Axis | Value | Source |
| :--- | :--- | :--- |
| Workspace Rust LOC (incl. test fixtures, excl. `target/` and `scratch/`) | **348,538** | `find . -name '*.rs' ... \| xargs wc -l` |
| Files parsed | 915 | `SELECT COUNT(DISTINCT json_extract(location,'$.file')) FROM nodes` |
| Parse wall-clock | **50.24 s** | `/usr/bin/time -l` on the parse binary |
| Analyze wall-clock | 12.48 s | `/usr/bin/time -l` on `ge-analyze` |
| End-to-end wall-clock (parse + analyze) | **62.72 s** | sum of above |
| Wall-clock per kloc (parse only) | **0.144 s/kloc** (144 ms/kloc) | 50.24 / 348.5 |
| Wall-clock per kloc (end-to-end) | **0.180 s/kloc** (180 ms/kloc) | 62.72 / 348.5 |
| Peak RSS during parse | **345 MB** (361,889,792 bytes) | `maximum resident set size` |
| Peak RSS during analyze | **399 MB** (418,709,504 bytes) | `maximum resident set size` |
| **Baseline peak RSS** (max of the two) | **399 MB** | this is the RSS floor the 200 MB kill criterion adds to |
| LSP session activity (parse-phase rust-analyzer wire) | 215,097 successes, 0 timeouts, max latency 625.1 ms | parse log `[LSP_STATS]` |

#### 9.B3.2. Health-report tier (the number that matters)

```
resolution_quality.resolution_tier:   "full"   ← infra-level: LSP wire reachable end-to-end
resolution_quality.measured_fidelity:
    tier:                             "syntactic_only"
    high_ratio_on_calls:              0.00874 = 0.87 %
    call_edges_by_confidence:
        high:    1,118
        medium:  30,809
        low:     96,003
        unknown: 0
    all_edges_by_confidence:
        high:    18,085
        medium:  34,189
        low:     96,003
        unknown: 0
summary.total_nodes:                  16,181
summary.total_edges:                  148,277
health_score:                         44 / 100
```

Two distinct kinds of "High" are visible in the output:

- **Call-edge High share (the T6 target axis): 0.87 %.** This is the
  number T6's dogfood acceptance criterion §5.4 ("`High` edge share
  ≥ 20 % of `Call` edges") is measured against. Today's value is
  `1118/(1118+30809+96003)`. The floor is effectively zero.
- **All-edge High share: 12.2 %.** This reflects high-confidence
  syntactic structural edges (`Contains`, `Import`, `Extends`) that
  do not require semantic resolution to emit at `High`. The T4 tier
  classifier's job is to **not** be fooled by this — and it isn't:
  it correctly reports `syntactic_only` because the classifier
  measures `high_ratio_on_calls`, not `all_high_ratio`. This is a
  live validation that T3/T4's split-the-signal instinct was right.

Note on `resolution_tier = "full"` vs `tier = "syntactic_only"`: the
two are measuring different things and must not be conflated.
`resolution_tier: full` reports "the LSP wire layer functioned at all"
(binary: did rust-analyzer respond to requests?). `measured_fidelity.tier:
syntactic_only` reports "how many of those responses survived into
`High` `Call`-edge emission?" — near-zero, because Layer 1 LSP
resolution has ~153 `Lsp/High` call edges out of 36,014 calls. This
two-layer distinction is itself a signal validator for T6: a naïve
regression that inflates `high_ratio_on_calls` without changing
`resolution_tier` would be instantly visible.

#### 9.B3.3. T6 kill-criterion cross-check

| T6 acceptance criterion (§5) | Number in that criterion | Baseline reading | Relationship |
| :--- | :--- | :--- | :--- |
| §5.4 `High` edge share on calls ≥ 20 % | 20 % | 0.87 % | Baseline is 23× below the floor. Meaningful signal if T6 lifts it. |
| §5.5 RSS kill: baseline + 200 MB | 200 MB delta | **baseline = 399 MB peak; ceiling = 599 MB** | The T6 kill-criterion number is now pinned to a measured baseline, not a guess. |
| §5.5 Wall-clock kill: 10 s/kloc | 10,000 ms/kloc | 144 ms/kloc parse-only | Baseline has **69× headroom**; a T6 implementation can add an order-of-magnitude latency and still pass. Bigger question: does the kill criterion want tightening? Noted as a sprint-primer follow-up, not an in-line fix. |

The 69× wall-clock headroom is suspicious-large and is worth a second
look at T6 implementation time. Two honest explanations compete: (a)
the 10 s/kloc kill criterion was set pessimistically for an unknown
rust-analyzer linkage, and the 1+ minute whole-scan floor is genuinely
small on modern hardware; or (b) the 348 kloc denominator is inflated
by test fixtures and generated code that are in the file-walk but have
near-trivial per-file parse cost. Both are defensible at 2026-04-19
and neither blocks B4; it is a note for the T6 design doc's §5 kill
criteria to revisit with measured numbers in hand.

### 9. Completion checklist

- [x] B1: version + MSRV recorded with exact numbers. Initial
      recommendation (pin latest) superseded by B2 findings; corrected
      recommendation is option C (combined pin: `channel = "1.91"` +
      `ra_ap_ide = "=0.0.307"`).
- [x] B2: spike executed; three-row combination matrix recorded;
      resource footprint floor measured; `AnalysisHost`/`RootDatabase`
      open question flagged as T6-implementation scope.
- [x] B3: baseline scan executed; `measured_fidelity.tier =
      syntactic_only`; `high_ratio_on_calls = 0.87 %`; peak RSS 399 MB;
      wall-clock 62.72 s end-to-end (0.18 s/kloc); all three numbers
      pin the T6 acceptance criteria to measured deltas.
- [x] B4 acceptance: §9 addendum complete; `scratch/` is `.gitignore`d
      at workspace root; no production code under `graphengine-*/`
      was modified during Phase B (only this design-doc addendum and
      `FOLLOWUPS.md::UF-FU-008`).

## 9.G1. Gate 1.1 implementation evidence (PR #1)

Appended during Gate 1.1 of the t6-t7-t8-customer-readiness plan. This
section records the ground-truth answers that the B1–B3 addendum
surfaced as open questions.

### 9.G1.1. `AnalysisHost` constructor path (UF-FU-009 closure)

`ra_ap_ide = 0.0.307` **does** expose `AnalysisHost::with_database(db:
RootDatabase) -> AnalysisHost` as a public constructor, contrary to the
B2 spike's tentative reading of the surface. This is verified by
compilation in `graphengine-ra-ide-adapter/src/resolver.rs` and by the
end-to-end `adapter_goto_definition_resolves_callee_from_main_rs`
integration test which proves `main -> callee` resolution emits
`Confidence::High` on the 2-file fixture.

Consequences:

- The "drive queries off `RootDatabase`" option (a) is ruled out. The
  `Analysis` query surface (`goto_definition`, `hover`, etc.) is only
  reachable via `AnalysisHost::analysis()`; `RootDatabase` alone does
  not expose the query traits publicly. This is recorded as a rejected
  path so future readers do not re-open the question.
- The "fresh `AnalysisHost::default()` + mirror change application"
  option (b/c) is kept documented in the resolver's rustdoc (`## UF-FU-009
  resolution (constructor path)`) as a named fallback if a future
  `ra_ap_ide` release removes `with_database` from the public API.
- UF-FU-009 closes.

### 9.G1.2. `GotoDefinitionConfig` API shape

The B2 spike noted the API only at the "not yet exercised" level.
Confirmed shape at `=0.0.307`:

```rust
// from ra_ap_ide-0.0.307/src/goto_definition.rs
pub struct GotoDefinitionConfig<'a> {
    pub minicore: MiniCore<'a>,
}

// Call shape:
pub fn goto_definition(
    &self,
    position: FilePosition,
    config: &GotoDefinitionConfig<'_>,
) -> Cancellable<Option<RangeInfo<Vec<NavigationTarget>>>>;
```

`MiniCore` is `ra_ap_ide_db::MiniCore` (not re-exported from
`ra_ap_ide`; adapter takes a direct `ra_ap_ide_db = "=0.0.307"`
dependency). `MiniCore::default()` injects the library's vendored
synthetic core source — production scans always prefer the real
sysroot that `load_workspace_at` has already wired in, so the default
is correct for us. The field is not `Option`, so callers cannot omit
it.

### 9.G1.3. `Vfs::file_id` API drift

`Vfs::file_id(&VfsPath) -> Option<(FileId, FileExcluded)>` at
`=0.0.307`. The `FileExcluded` flag is rust-analyzer's internal marker
for files in `.gitignore`d or vendored paths; we discard it because
exclusion is not a semantic-authority signal (the heuristic fallback
re-admits those files at lower confidence). Documented inline in
`RustAnalyzerSemanticResolver::vfs_file_id_for`.

### 9.G1.4. Production-file-filter baseline (UF-FU-011 closure)

Re-measured on `graphengine-parsing/src` alone (production source
only, no calibration repos, no test fixtures, no vendored grammars):

| Axis | Value | Source |
| :--- | :--- | :--- |
| Production Rust LOC | **55,779** | `wc -l` on all `.rs` under `graphengine-parsing/src/` |
| Parse wall-clock (release) | **3.22 s** | `/usr/bin/time -l` |
| Analyze wall-clock (release) | 0.28 s | `/usr/bin/time -l` |
| End-to-end wall-clock | **3.50 s** | sum |
| Wall-clock per kloc (parse-only) | **57.7 ms/kloc** | 3.22 / 55.779 |
| Wall-clock per kloc (end-to-end) | **62.7 ms/kloc** | 3.50 / 55.779 |
| Peak RSS (parse) | **143 MB** | `maximum resident set size` |
| Peak RSS (analyze) | 31 MB | same |
| `measured_fidelity.tier` | `syntactic_only` | health JSON |
| `high_ratio_on_calls` | **2.52 %** | health JSON |
| `health_score` | 63 / 100 | health JSON |

Cross-check against the original §9.B3 full-workspace number:
**production code is ~2× cheaper per kloc than the full-workspace
baseline** (57.7 vs 144 ms/kloc parse-only). The hypothesis in
§9.B3.3 was that test fixtures were *cheaper* per kloc; the
measurement disproves that — test fixtures have higher per-kloc
overhead (more imports, more syntax construct density) than expected.
This flips the UF-FU-011 picture: headroom on 10 s/kloc is **173×**
on production, not 69×; the criterion is decisively too loose.

**Tightened §5.5 wall-clock kill criterion: 0.5 s/kloc end-to-end**
(max(3 × baseline = 188 ms/kloc, 500 ms/kloc) = 500 ms/kloc). Gives
rust-analyzer linkage ~8× its current baseline cost before tripping;
still catches a pathological regression.

Measurement artefacts (parse.db + baseline.json) live in
`experiments/results/gate1-1-t6-pr1-uf-fu-011/` and are gitignored.

### 9.G1.5. Gate 1.1 PR contents (what ships, what does not)

Ships in PR #1:

- New crate `graphengine-ra-ide-adapter/` with pinned
  `ra_ap_ide = "=0.0.307"` (+ matching siblings), `MiniCore::default()`
  query plumbing, `AnalysisHost::with_database` constructor path.
- `rust-toolchain.toml` channel bump `stable → 1.91`.
- `graphengine-parsing` gets a new optional feature
  `rust-layer2 = ["dep:graphengine-ra-ide-adapter"]`. **Default-off.**
- 10 adapter unit + integration tests green on 1.91.1.
- Workspace default builds (`cargo check --workspace`) remain green;
  `cargo check --workspace --features graphengine-parsing/rust-layer2`
  also green, proving the feature flag is real.
- UF-FU-009 and UF-FU-011 close with measured evidence; T6 §5.5 kill
  criterion tightens from 10 s/kloc to 0.5 s/kloc.

Deliberately does **not** ship in PR #1 (reserved for PR #2 per
Gate 1.2):

- Wiring of the adapter into `LanguageSpecificExtractor::post_syntax_hooks`
  for Rust files. The adapter is reachable as a dependency but no
  code path calls it yet.
- `gridseak-self` dogfood CI job + rollup artefact.
- UF-FU-004 (`language_extractor_for_file` registry) resolution.
- Proc-macro known-miss test (UF-FU-003 contract).
- Default-on flip for `rust-layer2`.

---

## 10. Post-ship retrospective (Gate 1.3)

This section captures what T6 actually measured against what T6
predicted, what it cost to ship, and the specific residue the
implementation leaves behind for future work. Added at Gate 1.3 once
PRs #1 and #2 had both landed and the `gridseak-self` dogfood was
complete. Anchors T5/T7/T8 authoring decisions that depend on the
measured T6 reality, not the planning-phase projection.

### 10.1 Predicted vs measured

| Axis | Prediction (T6 §4 + D1) | Measurement (Gate 1.2 dogfood on `gridseak-self`) | Delta |
| :--- | :--- | :--- | :--- |
| `high_ratio_on_calls` | 60–80 % (Authoritative tier) based on D1 Rust-canary projection | **10.17 %** (`high=10306 / call_refs=101343`) → HeuristicPrimary tier by T4 thresholds | −50 to −70 pp |
| `edges_emitted` | ≥ 0.95 × `high + medium` resolutions (SymbolIndex assumed near-lossless) | **0.635 × resolutions** (6,550 edges / 10,311 resolutions → 36 % loss) | −32 pp |
| Adapter miss share (`no_target_misses / call_refs`) | < 30 % on a production Rust workspace (macro-light baseline assumed) | **88.9 %** on `gridseak-self` (proc-macro-heavy) | +59 pp |
| End-to-end wall-clock / kloc (with Layer 2) | < 500 ms/kloc (the tightened UF-FU-011 criterion) | 129,695 ms / 55,779 LOC production = **~2.3 s/kloc on full workspace**; filtered production-only rerun pending | Criterion defined on production-only — needs rerun with `--exclude-tests` before a verdict |
| T4 tier progression | `syntactic_only` → `authoritative` on first dogfood | **`syntactic_only` → `heuristic_primary`** | One tier short of prediction; still a step-up |

The 10.17 % headline is honest: Rust crossed the 10 % T4 threshold, so
the canary fixture `canary_rust_post_t6_gridseak_self_is_authoritative`
**does not** reflect the dogfood number — it reflects what the
resolver would do on a proc-macro-light codebase where the 88.9 %
miss share collapses. The fixture is intentionally pinned at 100 %
High because it tests the *tier classifier boundary*, not the
adapter's current emission rate. The adapter's real-world ratio is
held in UF-FU-012.

### 10.2 What UF-FU-012 surfaced that the plan missed

Three investigation threads, all filed at Gate 1.2 and none of them
blocking T6 completion:

- **UF-FU-012.a — SymbolIndex loss (36 %)**. The planning phase
  assumed `SymbolIndex::find_enclosing_function` and
  `find_callee_for` would match rust-analyzer's `goto_definition`
  output against `SyntaxResults::symbols` losslessly, because both
  sides originate from the same file bytes. The measurement disproves
  that: 3,761 successful Layer 2 resolutions failed the lookup.
  Root cause unconfirmed; strongest hypothesis is line/column vs
  byte-range coordinate drift for `impl`-scoped or macro-adjacent
  symbols. Closing this item alone would push `high_ratio_on_calls`
  from 10.17 % → ~16.7 % on the same dogfood corpus. **Highest-ROI
  next investment on T6.**
- **UF-FU-012.b — Adapter miss share (88.9 %)**. Not unexpected given
  UF-FU-003's known-miss contract on proc-macro expansion, but the
  *share* is higher than the plan implicitly assumed. The control
  experiment — rerun on a proc-macro-light Rust workspace — is not
  yet done. If the miss share stays above 80 % without `serde_derive`
  / `tokio::macros` in the dep graph, UF-FU-003 graduates from
  follow-up to `DISCOVERY_REPORT.md` entry and the `ra_ap_ide`
  expansion-API spike becomes a real T6.b task. If the miss share
  drops below 30 %, the current T6 ships as "correct for the happy
  path, blocked by proc-macro dependency density in customer code" —
  an honest customer-readiness statement.
- **UF-FU-012.c — `adapter_errors = 896`**. 0.9 % is noise on the
  per-scan denominator but an aggregated-only counter is not
  actionable. Adding a per-variant histogram to `ResolveSnapshot`
  distinguishes `FileNotInProjectModel` (shape: the file never made
  it into the loaded project model, probably a `build.rs` / `OUT_DIR`
  path) from `LineIndex` / `Vfs` failures (shape: coordinate
  translation bug inside the adapter). Small instrumentation PR; no
  architectural decision.

### 10.3 Decisions that held under measurement

What the planning-phase design got right, recorded so the next
universal-fidelity task does not relitigate them:

- **`AnalysisHost::with_database` as the constructor path (UF-FU-009).**
  The Gate 1.1 integration test closes it. If `ra_ap_ide` removes
  `with_database` in a future release, the option (c) fallback
  documented in `src/resolver.rs` remains workable.
- **`Mutex<RustAnalyzerSemanticResolver>` wrap for `Sync` compliance.**
  The `#[async_trait]` dispatch never holds the lock across an
  `.await`; the synchronous critical section is bounded by
  `goto_definition` cost (< 10 ms p99 on the dogfood). No measured
  contention in telemetry.
- **`resolved_call_sites` deduplication contract.** The heuristic
  fallback dedup worked correctly on the dogfood: with 6,550 LSP
  edges emitted, `heuristic_edges = 0` confirms the fallback skipped
  every reference that the Layer 2 path already claimed. Zero
  conflicting-confidence races.
- **Measured-fallback discipline.** The 10.17 % number itself is a
  direct vindication: `high_ratio_on_calls` reports what the resolver
  actually emitted, not what the resolver promised. T4's
  `measured_fidelity.tier` then downgrades the graph's claim
  honestly to `heuristic_primary`. The whole sprint's honesty thesis
  — "let the edges testify, not the resolver self-declare" — held up
  under its first real-world adversarial measurement.
- **`language_extractor_for_file` registry (UF-FU-004).** The Rust
  post-syntax hook wired through the registry without an
  orchestrator refactor. The shape scales to a second Layer 2
  language (Python via `jedi`, TS via `tsserver`) with only a new
  registry entry, not a rewrite.

### 10.4 Decisions that did not hold (or that require follow-up)

- **`high_ratio_on_calls` D1 projection (60–80 %).** D1's Rust
  canary prediction assumed a stdx-style library workspace. The
  measurement on a proc-macro-heavy applied codebase is 5–8× lower.
  D1's predictive model needs a "proc-macro-density adjustment"
  coefficient before the next canary spike, or D1 predictions
  become floor-not-ceiling. Recorded here; not yet a follow-up ID
  because D1 is a planning artefact not an engine contract.
- **Implicit assumption that adapter resolution → edge emission.**
  See UF-FU-012.a. The plan's §4 "data flow" diagram showed a direct
  arrow from `ResolvedTarget` to `Edge`; in practice there are two
  lookups (`find_enclosing_function`, `find_callee_for`) between
  them, either of which can drop the resolution. Future task docs
  under this template must include the lookup step explicitly in
  their data-flow diagram.
- **T6 §5.5 "performance kill criterion only" framing.** §5.5 only
  defined kill thresholds on wall-clock and RSS; it did not define a
  *fidelity* kill threshold. Had the dogfood measured
  `high_ratio_on_calls = 0.5 %` (below the pre-T6 baseline of
  2.52 %), we would have had no automated signal to roll back — the
  tiered-response rubric (≥20 %, 10–19 %, < 10 %) lives in the
  rework plan, not in the T6 doc. **Recommendation:** add a §5.6
  "fidelity kill criterion" to the TEMPLATE so future layer-2
  adapters have one.

### 10.5 Residue carried out of the sprint

- UF-FU-012 (three sub-items) — open.
- UF-FU-003 — held open pending UF-FU-012.b control run.
- UF-FU-002 and UF-FU-008 — `Open — quarterly watch`; next review
  2026-07-21.
- UF-FU-004 — closed by the `language_extractor_for_file` registry
  landing at Gate 1.2.
- UF-FU-007 — closed by the five canary fixtures added at Gate 1.3.
- UF-FU-009 / UF-FU-011 — closed at Gate 1.1 with measured evidence
  (§9.G1.1 and §9.G1.4 respectively).
- No new follow-ups filed against UF-FU-005 / UF-FU-006 by T6 work —
  the `real_resolver` path is parallel to the Rust Layer 2 path and
  the Gate 1.2 dogfood exercised only the new path.

### 10.6 What the T7 and T8 authors inherit

- T7 (Layer 0 git signals) inherits a HealthReport that now carries
  a `measured_fidelity.tier = heuristic_primary` for Rust. T7's
  `git_signal_share` metric lives alongside it as Layer 0 evidence
  and must not be allowed to *overwrite* the measured tier — same
  discipline as T6. Record Layer 0 agreement rate separately, in a
  sibling field on the report, not inside the tier axis.
- T8 (extraction-coverage awareness) inherits UF-FU-012.a as a
  direct constraint: if SymbolIndex is the bottleneck on Rust
  edge emission, a parallel bottleneck probably exists on NPSP
  Apex (which has a different SymbolIndex shape around
  `@AuraEnabled` / `@InvocableMethod` symbol resolution). T8's
  `FileExtractionCoverage` should flag files where
  `resolved_call_sites.len() < enclosing_function_lookup_attempts`
  and surface that as a secondary coverage-gap signal, not only the
  parse-walker coverage gap.
- Both T7 and T8 should assume Rust ships at the
  `heuristic_primary` tier, not `authoritative`, until UF-FU-012.a
  is closed. Any customer-readiness claims that depend on "Rust has
  authoritative coverage" are premature; the current floor is
  10.17 % High, which T4 classifies as `heuristic_primary`.
