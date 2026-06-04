# T5 — orchestrator trait-method collapse

Authored against [`TEMPLATE.md`](TEMPLATE.md). Every section is answered, no skipped headings.

---

## 0. Post-implementation retrospective

T5 shipped as designed (see §5 acceptance criteria; all green). Two
items surfaced **during** implementation that are outside T5's scope
and are now tracked in [`../FOLLOWUPS.md`](../FOLLOWUPS.md):

| Follow-up | What surfaced | Why it is not a T5 defect |
| :--- | :--- | :--- |
| `UF-FU-001` | `deserialise_class_symbols` is duplicated across `vf_extraction_stage.rs` and `framework_entry_point_stage.rs`. | The duplication predates T5. T5 moved the two files to `syntax::language::apex/` but did not change their bodies — that was a deliberate non-goal (§2, "Rewriting the content of `vf_extraction` / `framework_entry_point_propagation`"). Extracting the shared helper is a mechanical DRY pass with zero behavioural change and gets its own PR. |
| `UF-FU-005` | `real_resolver.rs:183` hardcodes `EdgeKind::Call` instead of calling `UnresolvedReference::edge_kind()`. | This is a **P1.d regression the T1 rework should have caught**, not a T5 concern. `real_resolver.rs` lives under `infrastructure/lsp/`, was not touched by T5, and is dormant today because only `Call` variants reach the LSP path. Filed here (as a T5 artefact) only because the T5 audit of `UnresolvedReference` consumers is what surfaced it. |
| `UF-FU-006` | `real_resolver.rs:124` uses `"dummy_path"` as the LSP document URI. | Pre-existing TODO from before the sprint. Filed because it directly undermines the UF-FU-005 fix. |

---

## 1. Problem statement

The parse-repo orchestrator at `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/orchestrator.rs` calls two Apex-specific stages by name:

- `vf_extraction::run(&root, &mut syntax_results)` at `orchestrator.rs:151` — Visualforce-page extractor that synthesises container `Struct` + `__vf_page__` `Function` + `Contains` edge per `.page`, and pushes `UnresolvedReference::FrameworkBinding` entries for resolved `{!method}` bindings.
- `framework_entry_point_propagation::run(&mut syntax_results)` at `orchestrator.rs:187` — Apex interface-inheritance walker that propagates platform-interface tags (`batchable`, `schedulable`, `queueable`, `inbound_email_handler`) onto contract-method nodes on abstract parent classes (Round 5 R11 fix).

Both stages `use super::...` an Apex-only module; both no-op on non-Apex parses by internally short-circuiting on empty `class_symbols`. The cost: **the orchestrator grows with every language**. Adding LWC JavaScript bindings (T6 follow-on), Java Spring endpoint propagation, Go embed-struct field propagation, or any other language-specific post-syntax hook requires either (a) a new `use super::…` + hardcoded call in `orchestrator.rs`, or (b) accreted branching at the top of the existing Apex stages. That is the exact pattern NPSP Round 5 called out as the root cause of R13 / R23 / R25 / R26 / R27 / R28 — language-specific pipelines each growing their own stage, with the orchestrator as the universal merge point.

**Concrete symptom of the shape bug.** `graphengine-parsing/src/application/use_cases/parse_repo/pipeline/orchestrator.rs` imports Apex-specific modules at crate module resolution time (`use super::vf_extraction;`, `use super::framework_entry_point_propagation;`). Removing Apex support from the build — or compiling a version of the engine that ships only Go + Rust — is not a config flag; it is a source edit. That is the "one type doing two jobs" smell at module-graph scale: the orchestrator is doing both (1) pipeline stage composition and (2) language dispatch.

## 2. Non-goals

- **Rewriting the content of `vf_extraction` / `framework_entry_point_propagation`.** Those stages stay correct. T5 moves *where they are called from*; it does not change their effects on `SyntaxResults`. Behavioural parity is the central acceptance criterion — any behavioural change would blur the rework signal.
- **Introducing new language hooks.** T5 does not add Rust, Java, or LWC-specific stages. It only makes the existing Apex stages callable through a trait method so future additions do not re-grow the orchestrator.
- **Changing the stage ordering (syntax → VF extraction → entry-point propagation → semantic resolve).** The hook trait method runs at the existing insertion point; the sequence is unchanged.
- **Collapsing `class_symbols` persistence into the trait.** `class_symbols` is already produced by `LanguageSpecificExtractor::extract_class_symbol_tables`; the orchestrator only persists the payload (`upsert_apex_class_symbols` in `sqlite_repository`). Renaming `upsert_apex_class_symbols` to a polymorphic persister is a separate refactor — this task is about post-syntax *hooks*, not storage. Tracked as an **out-of-scope follow-up** in §8.
- **Touching the `SyntaxExtractor` port (the top-level dispatcher that runs tree-sitter + queries).** T5 works at the language-specific *post-syntax* hook layer. The port layer is stable.

## 3. Five diagnostic questions

Answered before writing code. See `NEW_ENGINEER_PRIMER.md` §5.5 ("the two-jobs rule") and §8.0 ("diagnostic questions").

1. **Are we forcing one type to do two jobs?** **Yes.** The orchestrator does *(a)* pipeline stage composition and *(b)* language dispatch. Splitting them is the dissolving element — §4 introduces a `post_syntax_hooks(&self, …)` trait method so dispatch lives on the language-specific extractor and composition stays on the orchestrator.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** **Yes, implicit.** Today the orchestrator holds a hardcoded list of Apex hook calls; adding a language means updating that list. The dissolving element (trait-method dispatch) inverts the direction: the language declares its own post-syntax behaviour; the orchestrator iterates.
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** **No.** VF extraction and entry-point propagation already run after syntax extraction and before semantic resolution, and they have access to the same `SyntaxResults` the call resolver does. The stage-ordering constraint is stable; only the dispatch is wrong.
4. **Is the trade-off about serialization format?** **No.** T5 is a call-site refactor, not a data-shape change. `SyntaxResults` keeps its current in-memory form.
5. **Is the trade-off between two modes of failure?** **Partial — no.** Today both hooks fail-open (`warn!` and continue). That stays. The trait method returns `Result<HookStats, HookError>`; an `Err` is logged and does not abort the parse. Explicit, matches the existing behaviour, not a new axis.

Interpretation: **questions 1 and 2 are `Yes`**. §4 must supply the dissolving element — a trait-method dispatch that removes the orchestrator's hardcoded language-specific imports.

## 4. Chosen shape

### Types introduced or changed

Add a single default-implemented method to the existing `LanguageSpecificExtractor` trait at `graphengine-parsing/src/syntax/language/extractor.rs`:

```rust
/// Run language-specific post-syntax hooks. Called by the parse-repo
/// orchestrator after the tree-sitter extraction stage and before
/// semantic resolution.
///
/// The default implementation is a no-op (returns `HookOutcome::default()`).
/// Languages override it to splice in stages that need access to the
/// already-populated `SyntaxResults` but are *before* semantic
/// resolution — e.g. Apex's Visualforce-page extractor (emits
/// framework bindings and synthetic container nodes) and interface-
/// inheritance propagator (tags contract-method symbols on abstract
/// parents). Non-Apex extractors do nothing here today; any language
/// that grows such a hook plugs it in at this seam without touching
/// the orchestrator.
///
/// Contract:
/// - Runs after `extract` has populated `symbols`, `references`,
///   `imports`, `type_refs`, `class_symbols`.
/// - Runs before semantic resolution (LSP + heuristic call
///   resolution).
/// - May mutate `SyntaxResults` in place; MUST NOT construct or
///   populate `Graph` / `Edge`s outside of what `SyntaxResults`
///   already supports via `add_framework_binding` /
///   `add_declarative_binding` / `add_symbol`.
/// - MUST be no-op-safe when called on a parse that did not
///   populate the structures this language's hook consumes (e.g.
///   Apex VF extraction no-ops on empty `class_symbols`).
/// - Errors are reported as `HookOutcome::Warning`; the caller
///   logs + continues. Panics are a programmer error.
fn post_syntax_hooks(
    &self,
    _workspace_root: &std::path::Path,
    _syntax_results: &mut crate::application::ports::SyntaxResults,
) -> crate::syntax::language::extractor::HookOutcome {
    HookOutcome::default()
}
```

Introduce a small accompanying enum in the same file:

```rust
/// Outcome of a single language-specific `post_syntax_hooks` call.
///
/// Intentionally not a `Result` because the orchestrator always
/// continues: a failed hook degrades the parse (e.g. VF bindings
/// go missing) but never aborts it — pre-T5 behaviour. The enum
/// carries structured stats so the orchestrator's existing
/// "VF extraction: N pages parsed…" / "framework entry-point
/// propagation: M tagged…" log lines round-trip with no format
/// drift.
#[derive(Debug, Default)]
pub enum HookOutcome {
    /// Hook did nothing (default for non-participating languages).
    #[default]
    NoOp,
    /// Hook ran successfully. Optional human-readable summary line
    /// the orchestrator emits at `info!` level.
    Ok { summary: Option<String> },
    /// Hook failed; the error message is logged at `warn!` level
    /// and the parse continues.
    Warning { message: String },
}
```

### Data flow

```
extract (LanguageSpecificExtractor::extract)
   │  produces SyntaxResults{symbols, references, imports, type_refs, class_symbols}
   ▼
post_syntax_hooks (LanguageSpecificExtractor::post_syntax_hooks)          ← new seam
   │  may mutate SyntaxResults in place (add FrameworkBinding, tag symbols, …)
   ▼
semantic_resolve (LspResolver / fallback)
   │  consumes SyntaxResults.references, emits Edges
   ▼
graph build
```

The orchestrator's existing `Step 3.5` + `Step 3.6` blocks collapse into one dispatch:

```rust
let hook_outcome = language_extractor.post_syntax_hooks(&root, &mut syntax_results);
match hook_outcome {
    HookOutcome::NoOp => {}
    HookOutcome::Ok { summary: Some(s) } => info!("post-syntax hooks: {s}"),
    HookOutcome::Ok { summary: None } => {}
    HookOutcome::Warning { message } => warn!("post-syntax hooks failed ({message}); continuing"),
}
```

The Apex extractor (`graphengine-parsing/src/syntax/language/apex/extractor.rs`) provides the override, composing the two existing stages behind the trait method:

```rust
impl LanguageSpecificExtractor for ApexExtractor {
    // ... existing methods ...

    fn post_syntax_hooks(
        &self,
        workspace_root: &std::path::Path,
        syntax_results: &mut SyntaxResults,
    ) -> HookOutcome {
        // Retain existing fail-open semantics: stage 1 failing does
        // not prevent stage 2 from running. Preserves today's
        // orchestrator behaviour exactly.
        let vf_summary = match vf_extraction::run(workspace_root, syntax_results) {
            Ok(stats) if stats.pages_parsed > 0 || stats.pages_failed > 0 => {
                Some(format!("VF {} parsed / {} failed", stats.pages_parsed, stats.pages_failed))
            }
            Ok(_) => None,
            Err(e) => return HookOutcome::Warning { message: format!("VF extraction: {e}") },
        };
        let prop_summary = match framework_entry_point_propagation::run(syntax_results) {
            Ok(stats) if stats.function_nodes_tagged > 0 || stats.function_nodes_already_tagged > 0 => {
                Some(format!(
                    "entry-points tagged={} already={}",
                    stats.function_nodes_tagged, stats.function_nodes_already_tagged
                ))
            }
            Ok(_) => None,
            Err(e) => {
                return HookOutcome::Warning {
                    message: format!("entry-point propagation: {e}"),
                };
            }
        };
        let summary = match (vf_summary, prop_summary) {
            (None, None) => return HookOutcome::NoOp,
            (Some(a), Some(b)) => Some(format!("{a}; {b}")),
            (Some(s), None) | (None, Some(s)) => Some(s),
        };
        HookOutcome::Ok { summary }
    }
}
```

The orchestrator's `use super::vf_extraction;` and `use super::framework_entry_point_propagation;` imports are deleted. The two module paths now live behind the Apex extractor's own module tree (`syntax::language::apex::vf_extraction`, `syntax::language::apex::framework_entry_point_propagation`) — if they are not already, the refactor moves them there. The orchestrator is language-agnostic after this change.

### Compile-time guarantees

- **Adding a language never grows the orchestrator.** A new `LanguageSpecificExtractor` implementation may override `post_syntax_hooks` if it needs a hook, or inherit the no-op default if it does not. `orchestrator.rs` is touched exactly zero times.
- **Removing a language from the build is not a source edit of the orchestrator.** The Apex hook modules live under `syntax::language::apex::*`; compiling without that module (a hypothetical `#[cfg(feature = "apex")]` split) simply means the Apex extractor is absent from the language registry — the orchestrator still builds.
- **Trait-method signature pins the hook contract.** The `&mut SyntaxResults` argument and `HookOutcome` return type are compiler-enforced. A language that wants to emit edges directly would have to change the trait — a visible, reviewable action, not an accreted side effect.

### Predicate contracts

T5 does not touch `EdgeKind` or any taxonomy enum. `is_call_like`, `is_structural`, etc. are untouched. No predicate changes.

## 5. Acceptance criteria

Every criterion is behavioural, grep-able, falsifiable. Each maps to a §6 test or to an `rg` command a reviewer can run.

1. `rg -n "use super::vf_extraction|use super::framework_entry_point_propagation" graphengine-parsing/src/application/use_cases/parse_repo/pipeline/orchestrator.rs` returns **zero matches** after the rework.
2. `rg -n "vf_extraction::run|framework_entry_point_propagation::run" graphengine-parsing/src/application/use_cases/parse_repo` returns matches **only** under `syntax/language/apex/…` or from the new Apex override of `post_syntax_hooks`, never from `pipeline/orchestrator.rs`.
3. The `LanguageSpecificExtractor` trait has a `post_syntax_hooks` method with a no-op default implementation. `cargo doc --no-deps -p graphengine-parsing` lists the method with its doc comment. A new extractor that compiles without overriding it *runs* (proven by the Python / Go / JavaScript / TypeScript extractors, which do not override).
4. The existing R23 / A.5 VF binding fixture — `graphengine-parsing/tests/apex_resolver_r23_a5_vf_fixtures.rs` — passes with zero assertion edits. Same three tests (`r23_a5_util_jobprogress_binds_refresh_jobs`, `r23_a5_minimal_action_binds_save`, `r23_a5_multi_extension_falls_through_to_ext_b`) stay green.
5. The Apex heuristic corpus fixture — `graphengine-parsing/tests/apex_heuristic_corpus.rs` — passes with zero assertion edits, proving framework entry-point propagation still tags `batchable` / `schedulable` / `queueable` / `inbound_email_handler` contract methods on the ancestor classes it used to tag.
6. New unit test `graphengine-parsing/src/syntax/language/extractor.rs` `#[cfg(test)] mod post_syntax_hooks_default` asserts the default implementation returns `HookOutcome::NoOp` for every non-overriding language (parameterised over `PythonExtractor`, `GoExtractor`, `JavaExtractor`, `JavaScriptExtractor`, `TypeScriptExtractor`, `RustExtractor`, `CSharpExtractor`, `GenericExtractor`).
7. New behavioural test `graphengine-parsing/tests/t5_orchestrator_language_agnosticism.rs` parses a synthetic repo with only a `.py` file and confirms the orchestrator runs end-to-end without error and without emitting any Apex-specific log lines (no `VF extraction:` prefix, no `framework entry-point propagation:` prefix).
8. `cargo build --all-targets` and `cargo test --workspace` both green.

## 6. Test plan

Three tiers.

### 6.1 Unit tests

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `post_syntax_hooks_default_is_noop` | `graphengine-parsing/src/syntax/language/extractor.rs #[cfg(test)]` | Every non-Apex `LanguageSpecificExtractor` impl returns `HookOutcome::NoOp` when `post_syntax_hooks` is called on an empty `SyntaxResults`. |
| `apex_post_syntax_hooks_short_circuits_on_empty_class_symbols` | `graphengine-parsing/src/syntax/language/apex/extractor.rs #[cfg(test)]` | `ApexExtractor::post_syntax_hooks` returns `HookOutcome::NoOp` when `class_symbols` is empty (no `.cls` files parsed). |
| `hook_outcome_default_is_noop` | same file | `HookOutcome::default() == HookOutcome::NoOp`. Locks the derive. |

### 6.2 Integration tests (behavioural)

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `vf_bindings_reach_graph_via_trait_hook` | `graphengine-parsing/tests/apex_resolver_r23_a5_vf_fixtures.rs` (existing) | Passes with zero assertion edits. VF bindings still reach the graph; the three existing `r23_a5_*` tests stay green. |
| `framework_entry_points_tagged_on_ancestors` | `graphengine-parsing/tests/apex_heuristic_corpus.rs` (existing) | Passes with zero assertion edits. `batchable` / `schedulable` / `queueable` / `inbound_email_handler` tags still land on contract methods on abstract ancestors. |
| `non_apex_parse_emits_no_apex_hook_log_lines` (new) | `graphengine-parsing/tests/t5_orchestrator_language_agnosticism.rs` | Parses a single-file Python repo end-to-end through `ParseRepoUseCase`. Asserts the captured log buffer contains no substring `"VF extraction:"` or `"framework entry-point propagation:"`. |
| `apex_parse_still_emits_hook_log_lines` (new) | same file | Parses a minimal Apex + `.page` repo. Asserts the captured log buffer still contains the Apex hook summary strings. Pinning the log format across the rework. |

### 6.3 Regression fixture

The R23 / A.5 fixture is the canary. It is behavioural, its assertions are variant-pattern matches (post-P1.d), and its inputs are the on-disk fixture repos already in `test-repos/`. Any regression in the Apex hook plumbing surfaces there.

If a reviewer wants a stronger canary, the end-to-end analysis determinism test (`graphengine-analysis/tests/determinism_integration.rs`) already compares two runs of the full pipeline byte-for-byte. A T5 rework that silently changed hook ordering would break it.

## 7. Rollback criterion

**Single named signal:** `cargo test -p graphengine-parsing --test apex_resolver_r23_a5_vf_fixtures r23_a5_util_jobprogress_binds_refresh_jobs` fails after the T5 merge.

This fixture depends on the entire Apex VF pipeline: tree-sitter extraction, VF stage, semantic resolution, `UnresolvedReference::FrameworkBinding` dispatch, edge emission. If it fails, the trait-method refactor has either mis-ordered the hook relative to semantic resolution, passed the wrong `workspace_root`, or dropped the mutable `SyntaxResults` reference on the way through. Any of those is a revert signal. Every other T5 failure mode (new trait method compilation error, non-Apex regression) is a pre-merge build break and cannot ship.

## 8. Out-of-scope follow-ups

- **Polymorphic `upsert_apex_class_symbols`.** The persistence step at `orchestrator.rs:408` is Apex-named. Generalising to `upsert_class_symbols_for(language: &str, payload: &[(String, String)])` is the mirror of T5 on the storage layer. Deferred — no ticket yet. Proposed name: **T5.b — class-symbols persistence dispatch**. Rationale for deferral: blocked on a second non-Apex language producing class_symbols (today only Apex does); until then the generalisation has exactly one implementor and cannot be validated by a polymorphism test.
- **Splitting the Apex hook modules into `syntax::language::apex::*` if not already there.** If `vf_extraction` / `framework_entry_point_propagation` live at `application/use_cases/parse_repo/pipeline/*` today (orchestrator siblings), they must move under `syntax::language::apex::*` as part of T5. If they already live there, no move is required. Confirm during implementation; if the move is needed, it lands in the T5 PR (no separate ticket), because the acceptance criterion ("orchestrator imports nothing Apex-specific") makes the move load-bearing.
- **A registry-based `language_extractor_for_repo(&ScanContext) -> Arc<dyn LanguageSpecificExtractor>` function.** Today the orchestrator is passed the extractor; the registry is upstream. T5 does not touch that layer. If a future task (post-T6, when Rust has its own post-syntax hook for proc-macro expansion counts) wants to unify extractor selection across languages, that is a separate design decision. Tracked in `docs/workstreams/universal-fidelity/FOLLOWUPS.md` as `UF-FU-004 — multi-language scan dispatch shape`. Not yet a `DISCOVERY_REPORT.md` entry; will be promoted to a discovery question once a second non-Apex language needs a post-syntax hook.
- **Moving T5's hook model into the sibling `LspResolver` trait so semantic resolution also gets a typed per-language hook.** Not in scope. `LspResolver` has its own concerns (fall-back ordering, receiver-type detector injection); mixing them with post-syntax hooks would re-couple the two. If a semantic-resolution hook becomes necessary (e.g., Rust wants to consult `ra_ap_ide` before our call resolver), that is a T6 follow-on, not a T5 widening.
