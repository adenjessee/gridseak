# T7 — Layer 0 git signals

Authored against [`TEMPLATE.md`](TEMPLATE.md). Every section is answered, no skipped headings.

---

## 1. Problem statement

The Layered-Fidelity Architecture names Layer 0 ("always-available truth") as the floor every other layer sits on: git history of the repository. The engine today consults none of it. Nothing in `graphengine-analysis/src/` or `graphengine-parsing/src/` reads `.git/`. Every metric is therefore a function of the working-tree snapshot alone.

Concrete consequences:

- **`MeasuredFidelityTier::SyntacticOnly` has no floor.** When the semantic resolver is unavailable (every non-Apex non-TypeScript language today, by §1 of T6), the metrics fall to syntactic tree-sitter signals + heuristics. There is no "but we still know that this file has been churned 47 times in the last 90 days across 8 authors" signal to rescue any classifier. Customers whose language has no Layer 2 see nothing but a degraded headline.
- **Hotspot detection is absent.** Round 5 audit notes (R27) called out that a developer reading the dead-code report would trivially classify a method differently if they knew the enclosing file had 14 commits this quarter versus 0 commits in 3 years. We have no signal to surface that.
- **Co-change clusters are absent.** Edges the call resolver cannot draw (R45 chained calls, R41 map-literal initializers) are sometimes visible in the commit record: two files that always change together, in the same commit, by the same author, over > 10 commits are structurally coupled whether or not the graph reflects it.
- **Ownership dispersion is absent.** Dead-code classification cannot account for "this method was written by a contractor in 2022 and never touched since" versus "this method is owned by 3 current engineers who committed to it last week."

## 2. Non-goals

- **Using git signals to *override* call edges.** Layer 0 is an independent evidence stream; it feeds the classifier as an additional signal, but it never synthesises `Edge` objects. The graph shape is still owned by Layer 1 + Layer 2.
- **Cross-repo co-change analysis.** `gix` can traverse one repository; cross-repo signal aggregation is a separate dataplane concern.
- **Handling non-git version control.** Mercurial / Perforce / Fossil / Jujutsu are out. The engine emits a `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1` caveat when a scanned repo is not a git working tree.
- **Mining commit messages.** Message-text classification (fix / feat / chore) is downstream of this task; it requires an NLP or convention parser and is a separate ticket.
- **Rewriting any existing `metrics` output.** T7 adds signals to the health report and the classifier input; it does not rename or restructure the existing headline metrics surface.
- **Bypassing the confidence envelope.** Every signal emitted by T7 carries an explicit `Confidence` (see §4.3). Signals from a shallow clone are `Confidence::Low` regardless of how strong they would have been on a full clone.

## 3. Five diagnostic questions

1. **Are we forcing one type to do two jobs?** **Partial — No.** The new signals are a distinct evidence type (`GitSignal` struct) from code-graph edges. The classifier consumes both but their domain types stay separate. No conflation.
2. **Is the trade-off between "hardcoded list" and "brittle update"?** **Yes.** A naive design would hardcode "which metrics consume which git signal" in each classifier. The dissolving element (§4) is a predicate-style `GitSignal::applies_to_metric(&MetricKind)` registration, mirroring the `EdgeKind` predicate pattern introduced in P1.a. Adding a new signal type updates the predicate once; every metric that queries the predicate picks up the new signal automatically.
3. **Is the trade-off between "earlier stage knows X" and "later stage knows Y"?** **Yes.** The `gix` walker knows commit timestamps + authors + file paths; the classifier knows metric semantics. The dissolving element is a typed channel — `GitSignal` enum emitted per-file, consumed by classifiers — not a bag of `HashMap<String, serde_json::Value>` properties the classifier has to re-parse.
4. **Is the trade-off about serialization format?** **No.** Signals live in-memory and in the existing analysis SQLite. We use `serde` with the existing `{"kind":…, "sub":…}` pattern (same as `EdgeKind` post-P1.b).
5. **Is the trade-off between two modes of failure?** **Yes.** Shallow clones (our NPSP / django-site / commons-lang canaries are 1-commit clones) genuinely have insufficient history. The false choice is "error out" vs "silently emit garbage." The dissolving element is an explicit `CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1` + downgrade every signal to `Confidence::Low` + continue. Observable, not silent, not fatal.

Interpretation: **questions 2, 3, 5 are `Yes`**. §4 supplies all three dissolving elements.

## 4. Chosen shape

### 4.1 New crate: `graphengine-git-signals`

Workspace member. Single dependency on `gix`. Exposes:

```rust
pub struct GitSignalExtractor { /* gix::Repository + parameters */ }

impl GitSignalExtractor {
    pub fn open(repo_root: &Path) -> Result<Self, OpenError>;
    pub fn extract(&self, window: HistoryWindow) -> Result<GitSignalReport, ExtractError>;
}

pub struct HistoryWindow {
    pub commits_back: usize,        // cap — primary bound
    pub days_back: Option<u32>,     // secondary bound, optional
    pub max_wall_clock: Duration,   // hard budget
}

pub struct GitSignalReport {
    pub repository_shape: RepoShape,     // Full / Shallow { depth } / Bare / NonGit
    pub per_file: HashMap<PathBuf, FileSignals>,
    pub co_change_clusters: Vec<CoChangeCluster>,
    pub integrity_caveats: Vec<&'static str>,
}

pub struct FileSignals {
    pub change_frequency: u32,              // commits touching this file in window
    pub distinct_authors: u32,
    pub last_touched_days: Option<u32>,
    pub ownership_dispersion: f32,          // 0.0 (one author) → 1.0 (uniform)
    pub hotspot_score: f32,                 // change_frequency * file_complexity_hint
    pub confidence: Confidence,             // Low on shallow clones, High otherwise
}

pub struct CoChangeCluster {
    pub files: Vec<PathBuf>,
    pub co_commit_count: u32,
    pub confidence: Confidence,
}
```

### 4.2 Shallow-clone guard

`RepoShape::detect(&gix::Repository) -> RepoShape` inspects:

1. `.git/shallow` existence → `Shallow { depth }`.
2. `cargo` / canary mode with `--depth 1` inputs → `Shallow { depth: 1 }`.
3. Bare repositories (no working tree) → `Bare`; current scan path does not support these anyway.
4. Absence of `.git/` → `NonGit`.

On any non-`Full` shape:

- Every `FileSignals.confidence` is downgraded to `Confidence::Low`.
- `GitSignalReport.integrity_caveats` gets `CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1` with the detected shape.
- `co_change_clusters` is emitted if at least two files share the single available commit; otherwise empty. `confidence = Low`.
- Classifiers that consume signals gated on `Confidence::High` therefore silently skip the signal on shallow clones — no behavioural surprise.

Our canary fixtures (NPSP rev 9, commons-lang, django-site, serilog) are all 1-commit clones, so this guard is **load-bearing from day 1**. T7 without the guard would poison every canary with false-negative "no churn signal" reads.

### 4.3 Classifier integration

Two touchpoints in `graphengine-analysis`:

```rust
// graphengine-analysis/src/health/report.rs
pub struct HealthReport {
    // … existing fields …
    pub git_signals: Option<GitSignalReport>,   // None when VCS unsupported
}

// graphengine-analysis/src/classifiers/dead_code/mod.rs
impl DeadCodeClassifier {
    pub fn classify(&self, graph: &AnalysisGraph, git: Option<&GitSignalReport>) -> Vec<DeadCodeCandidate> {
        for candidate in raw_candidates {
            if let Some(signals) = git.and_then(|g| g.per_file.get(&candidate.file)) {
                if signals.confidence == Confidence::High && signals.last_touched_days.unwrap_or(u32::MAX) < 30 {
                    candidate.confidence = candidate.confidence.downgrade();   // "recently touched ≠ dead"
                }
            }
        }
        // …
    }
}
```

The classifier **reads predicates**, not raw numeric fields. `GitSignalConsumer` trait provides `is_active_recent(&FileSignals) -> bool`, `is_high_churn(&FileSignals) -> bool`, etc. — same discipline as the `EdgeKind::is_call_like()` shape from P1.a.

### Compile-time guarantees

- `GitSignalExtractor::extract` returns `GitSignalReport` by value; no mutable shared state.
- `RepoShape` is a closed enum; adding a shape requires updating every classifier consumer (compiler-enforced).
- `FileSignals.confidence` is mandatory — no `Option<Confidence>`. A signal without confidence cannot be constructed.

### Predicate contracts

New predicate set on `GitSignal` consumers. No change to `EdgeKind` predicates.

## 5. Acceptance criteria

1. `cargo test -p graphengine-git-signals` green.
2. Synthetic git repo fixture at `graphengine-git-signals/tests/fixtures/three_commits_one_author/` produces `FileSignals { change_frequency: 3, distinct_authors: 1, ownership_dispersion: 0.0, confidence: High }` for the single changed file. Test: `t7_three_commits_one_author.rs` → `single_author_ownership_is_zero_dispersion`.
3. Synthetic shallow-clone fixture at `graphengine-git-signals/tests/fixtures/shallow_one_commit/` produces `RepoShape::Shallow { depth: 1 }`, emits `CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1`, and every `FileSignals.confidence == Confidence::Low`. Test: `t7_shallow_clone_guard.rs` → `shallow_clone_downgrades_all_signals_to_low`.
4. Non-git directory fixture produces `RepoShape::NonGit`, emits `CAVEAT_LAYER0_UNSUPPORTED_VCS_V1`, and `GitSignalReport.per_file.is_empty()`. Test: `t7_non_git_repo.rs` → `non_git_directory_emits_caveat_and_no_signals`.
5. `DeadCodeClassifier` regression fixture `graphengine-analysis/tests/t7_dead_code_respects_recent_churn.rs` — a synthetic graph where node X would be classified as dead, plus a git signal showing X was touched 5 days ago, produces classification confidence `Medium` (downgraded from `High`). Same fixture without the git signal produces `High`. Named behavioural difference.
6. NPSP canary (rev 9, 1-commit clone) scan emits a non-empty `git_signals` field in the health report with every `FileSignals.confidence == Low` and `CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1` present. Verified by `graphengine-diagnostic/tests/npsp_layer0_shallow_caveat.rs`.
7. Resource envelope: extracting signals on `gridseak-self` (full clone) completes in < 2 s wall-clock and adds < 50 MB RSS over baseline. Kill criterion: > 5 s or > 200 MB fails CI for that PR.
8. `rg -n 'HashMap<String, serde_json::Value>' graphengine-git-signals/src/` returns zero — no stringly-typed property bags escape the crate.
9. `cargo build --all-targets` green with the new crate as a workspace member.

## 6. Test plan

### 6.1 Unit tests

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `repo_shape_detects_full` | `graphengine-git-signals/src/repo_shape.rs #[cfg(test)]` | Synthetic full-clone fixture → `RepoShape::Full`. |
| `repo_shape_detects_shallow_via_shallow_file` | same file | `.git/shallow` present → `Shallow { depth }` with parsed depth. |
| `repo_shape_detects_non_git` | same file | No `.git/` → `RepoShape::NonGit`. |
| `ownership_dispersion_single_author_is_zero` | `graphengine-git-signals/src/ownership.rs #[cfg(test)]` | Single-author file yields `0.0` dispersion. |
| `ownership_dispersion_uniform_three_authors_is_near_one` | same file | Three authors with equal commit counts yields ≥ 0.95. |
| `confidence_is_low_on_shallow_regardless_of_signal_strength` | `graphengine-git-signals/src/confidence.rs #[cfg(test)]` | `FileSignals` constructed with high change_frequency but `RepoShape::Shallow` → `confidence == Low`. Locks the guard. |

### 6.2 Integration tests

| Test | Location | Asserts |
| :--- | :--- | :--- |
| `extract_three_commits_one_author` | `graphengine-git-signals/tests/t7_three_commits_one_author.rs` | See acceptance criterion #2. |
| `shallow_clone_emits_caveat_and_downgrades` | `graphengine-git-signals/tests/t7_shallow_clone_guard.rs` | See criterion #3. |
| `non_git_directory_returns_non_git_shape` | `graphengine-git-signals/tests/t7_non_git_repo.rs` | See criterion #4. |
| `dead_code_classifier_respects_recent_churn` | `graphengine-analysis/tests/t7_dead_code_respects_recent_churn.rs` | See criterion #5. |
| `co_change_clusters_emit_two_file_pair` | `graphengine-git-signals/tests/t7_co_change_two_file_pair.rs` | Fixture where files A and B are edited in the same commit 5 times produces a `CoChangeCluster { files: [A, B], co_commit_count: 5 }`. |

### 6.3 Regression fixture

NPSP rev 9 canary is the primary regression canary: it is a shallow clone, so any regression in the shallow-clone guard immediately shows up as the caveat being absent or a `High`-confidence signal escaping.

`gridseak-self` (a full clone, by virtue of being our own repo) is the secondary canary: regression in the full-clone signal path shows up as empty `co_change_clusters` or missing `hotspot_score`.

## 7. Rollback criterion

**Single named signal:** `cargo test -p graphengine-git-signals --test t7_shallow_clone_guard shallow_clone_emits_caveat_and_downgrades` fails *or* the NPSP canary CI job regresses to a `MeasuredFidelityTier` classification that depends on a `High`-confidence git signal escaping the shallow-clone guard.

Broader definition: any scenario where a `FileSignals { confidence: High }` is constructed from a shallow clone. That is the single load-bearing invariant the crate protects, and its violation invalidates every downstream classifier.

## 8. Out-of-scope follow-ups

- **Commit-message classification.** NLP / convention parsers for commit subjects are a separate research investment. Proposed: **T7.b — commit message semantics**.
- **Hg / P4 / Fossil / Jujutsu adapters.** Only after customer demand. Proposed: **T7.c — non-git VCS adapters**.
- **Cross-repo co-change clusters.** Requires a repo registry and trust boundary. Proposed: **T7.d — federated co-change**.
- **Commit-signing / GPG verification as a signal.** Useful for supply-chain work but orthogonal to fidelity. Proposed: **T7.e — commit-signing signal**.
- **Per-author hotspot attribution in the headline metric.** Exposing authorship in the customer-visible metric is a PII / legal review. Defer until a compliance review is scoped.

## 9. Post-ship retrospective (Gate 2)

Captured during Gate 2 close-out. This section is **append-only**: subsequent T7.b / T7.c / T7.d work must add its own dated subsection rather than editing the one below.

### 9.1 Correlation measurement on `gridseak-self`

| Field | Value |
| :--- | :--- |
| Run date | 2026-04-18 |
| Workspace | `gridseak-graphengine` (self-hosting) |
| Git HEAD | `5a851ae` ("T6 PR #1: graphengine-ra-ide-adapter crate (Gate 1.1)") |
| Toolchain | `rustc 1.91.1 (ed61e7d7e 2025-11-07)` |
| Platform | aarch64-apple-darwin |
| Window | `HistoryWindow::default_ci()` — 500 commits, 365 days, 2 s wall-clock |
| Repo shape | `Full` |
| Commits walked | 47 (entire visible history — the repo is newer than the 365-day bound) |
| Files touched | 2,143 (leaf blobs; directories excluded after the fix in 9.2) |
| Wall-clock (`extract()` only) | ~400 ms first run, ~100 ms warm-cache |
| End-to-end wall-clock (`dump_report` harness) | 0.38–0.85 s |
| Peak RSS (`dump_report` harness, JSON-on) | 349 MB |
| Peak RSS (`dump_report` harness, JSON-off) | 334 MB |
| Top-10 ordering (after 9.2 fix) | `graphengine-parsing/src/syntax/treesitter.rs` (15 changes / 2 authors), `Cargo.lock` (14 / 1), `graphengine-parsing/src/infrastructure/lsp/resolver.rs` (14 / 2), `gridseak-desktop/graphengine5.db` (14 / 2), `graphengine-parsing/src/application/ports.rs` (12 / 1), `graphengine-parsing/src/main.rs` (11 / 1), `graphengine-parsing/src/syntax/extractors/trait_context_detector.rs` (11 / 2), `gridseak-desktop/graphengine5.db-wal` (11 / 2), `graphengine-parsing/Cargo.toml` (10 / 1), `graphengine-parsing/src/infrastructure/lsp/call_resolver.rs` (10 / 1) |
| Confidence distribution | all 2,143 per-file entries are `Confidence::High` (repository is `Full`-shape; no shallow-clone guard triggered) |
| Co-change clusters emitted | 130 (threshold `CO_CHANGE_MIN_FULL = 3` with `MAX_FILES_PER_COMMIT_FOR_CLUSTERING = 50`) |

**Hand-rank agreement.** A post-hoc spot-check of the top-10 list against "what has actually changed a lot in this repo" (using `git log --format=%H --first-parent -- <path> | wc -l` as the ground truth) confirmed every top-10 entry is genuinely among the busiest 15 files in the repository. The D3 discovery bar ("≥ 80 % hand-rank agreement on 3 repos") is met on this canary; the two other canaries (`commons-lang`, `django-site`) remain as follow-up D3 corroborations once their working trees are staged locally — see `UF-FU-015`.

**Wall-clock versus kill criterion.** The measured extract time (0.4 s cold, 0.1 s warm) is well inside the §5.7 "< 2 s" budget.

**Peak RSS versus kill criterion.** §5.7 states "adds < 50 MB RSS over baseline … > 200 MB fails CI". On `gridseak-self` the absolute peak is 334–349 MB, but the repository's own `.git/objects` directory is 185 MB (27.5 MB pack plus a large loose-object tail from ~260 cross-branch merges). `gix` mmaps pack files; on macOS the mmapped pages count toward RSS. Without a bare-baseline measurement (a `gix::open` + no-op that also mmaps the same bytes) the 50 MB-over-baseline criterion cannot be cleanly evaluated from this single-run data. The extractor's own object-cache cap was set to 16 MiB explicitly so the *in-process allocator* footprint stays bounded regardless of repository size; this covers the "don't unbounded-grow on large repos" concern even if the mmap-counted RSS headline number is harder to interpret. **Follow-up filed as `UF-FU-016`** to instrument the baseline comparison in CI rather than argue about it textually.

### 9.2 Bugs and fixes surfaced during the measurement

**Bug 1: directory entries appeared as "files" in `GitSignalReport::per_file`.**  
The initial `collect_changed_files` callback pushed every `change.location()` regardless of `change.entry_mode()`, so tree-kind entries (intermediate directories whose recursive content changed) inflated `change_frequency`, `ownership_dispersion`, and `hotspot_score`. The first `gridseak-self` dump showed the top-10 hotspot scores were all directory paths — `graphengine-parsing`, `graphengine-parsing/src`, `docs`, ... — with change counts that were recursive-sum artefacts.  
Fix: filter on `change.entry_mode().is_blob()` in the callback; rewrites apply the same filter to both sides.  
Regression lock: `graphengine-git-signals/tests/t7_tree_diff_excludes_directories.rs::per_file_contains_only_blob_paths_not_directory_paths`.

**Bug 2 (pre-fix, surfaced during fixture authoring): root commits were diffed against themselves.**  
Noted in the Gate 2 working notes and the `collect_changed_files` in-function comment block; fixed by falling back to `repo.empty_tree()` when the commit has no parent. Locked by `tests/t7_three_commits_one_author.rs`.

### 9.3 Two-jobs / measured-fallback audit

The T7 design split `graphengine-git-signals` (measures the working tree) from `graphengine-analysis::health::git_signals_attach` (composes the measurement onto a pre-built `HealthReport`) and from `graphengine-analysis::health::dead_code_classifier::apply_git_signal_churn_downgrade` (consumes the signal predicate). All three modules pass their own test suites in isolation. `attach_git_signals` never returns `Err`: extractor failure is logged as `Skipped(OpenError)` and leaves `report.git_signals` as `None`. Consumers that see `None` must therefore treat it as "no measurement" rather than "no churn" — the `CAVEAT_LAYER0_GIT_SIGNALS_V1` marker on `report.integrity_status.schema_caveats` is the signal that the attach path ran.

### 9.4 Inherited-from-T6, passed-to-T8

- **From T6.** The dead-code classifier's new `Confidence` field is the integration point T8 extraction-coverage awareness will also need: a `no_callers` verdict with insufficient walked files (T8) is logically identical to a `no_callers` verdict on a recently-churned file (T7). Both are `High -> Medium` downgrades with different predicates. When T8 lands, keep its classifier-integration surface symmetric with `apply_git_signal_churn_downgrade`: one classifier-level function, one report-level function, one regression fixture.
- **To T8.** The `--no-git-signals` flag now gates the attach step on `ge-analyze`. T8 should land a sibling `--no-coverage-gap` flag on the same binary so operators can reproduce pre-T7 / pre-T8 reports byte-for-byte when debugging regressions. The `UF-FU-010` path A ("single-command `graphengine-diagnostic` CLI") would make both flags first-class customer-visible options; path B would document them in the two-binary plan. Decision deferred to Gate 4.

