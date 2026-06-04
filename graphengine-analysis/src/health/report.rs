//! JSON output contract for ge-analyze health reports.
//!
//! Every struct here maps 1:1 to the JSON schema defined in the specification (Section 6).
//! Deterministic serialization: all maps use BTreeMap (sorted keys), all vecs are pre-sorted
//! before serialization.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Schema caveat constants (locked — never rename or repurpose).
//
// Each string identifies a distinct change to the meaning of the numeric
// values in a HealthReport. Downstream tools match these as opaque tokens
// to distinguish "this report was produced by version X of the pipeline"
// without having to parse `generated_at` or the engine version.
//
// RULES for evolving this list:
//   - Never rename a constant. Downstream consumers match on the literal.
//   - Never remove a constant. Add new ones; old reports still reference
//     the old ones.
//   - Adding a constant is a minor, additive schema change and does NOT
//     require a HealthReport version bump.
// ---------------------------------------------------------------------------

/// Stamped on every report produced by an engine that includes the
/// cycle-detection ordering fix. Reports missing this caveat (including
/// any future default constructor path) must be treated as
/// `legacy_pre_orderfix` by downstream trend-tracking.
pub const CAVEAT_CYCLES_ORDERFIX_APPLIED: &str = "cycles_orderfix_applied";

/// Stamped on every report emitted after the per-metric `MetricStatus`
/// contract was introduced. Older reports (pre-contract) have no
/// `integrity_status` field; downstream tools should infer an empty
/// caveat list plus `legacy_pre_orderfix` in that case.
pub const CAVEAT_METRIC_STATUS_CONTRACT: &str = "metric_status_contract_v1";

/// Implicit caveat inserted by downstream tooling for reports that
/// predate `CAVEAT_CYCLES_ORDERFIX_APPLIED`. Not emitted by this engine.
/// Defined here so there is a single authoritative string for the concept.
pub const CAVEAT_LEGACY_PRE_ORDERFIX: &str = "legacy_pre_orderfix";

/// Stamped on every report produced by an engine that populates the
/// dead-code reason classifier (`dead_code_classifier` module). Reports
/// missing this caveat have only an aggregate dead-code count — no
/// per-function reason, no `reason_breakdown` — and trend lines of
/// `no_callers` vs `framework_annotation_unresolved` cannot be computed
/// across the boundary. Downstream tools should treat missing breakdowns
/// in such reports as `{DeadCodeReason::Unclassified: total}`.
pub const CAVEAT_DEAD_CODE_REASONS_V1: &str = "dead_code_reasons_v1";

/// Stamped on every report produced by an engine that runs T3's dual-
/// metric emission — every Layer-3 metric detail carries a
/// `fidelity` block showing the metric computed once over all
/// production structural edges and once over just the High-confidence
/// subset, plus the absolute/relative gap between them. Reports
/// missing this caveat either (a) predate T3 or (b) were produced
/// against a parse.db whose edges lack `provenance.confidence` rows,
/// in which case every edge defaults to `Confidence::Unknown` and
/// the high-only graph is empty — the fidelity block will still
/// render but its numbers reduce to "this scan cannot be measured".
pub const CAVEAT_DUAL_METRIC_EMISSION_V1: &str = "dual_metric_emission_v1";

/// Emitted when `ge-analyze` loads a parse DB whose persisted schema
/// version is **older** than the engine's current
/// `APEX_CLASS_SYMBOLS_SCHEMA_VERSION`. Such a DB was produced by a
/// pre-TR-A.0 parse run and therefore lacks the `apex_class_symbols`
/// rows the Phase-A resolver consumes. Downstream numbers in the
/// report are still computable, but Apex no_callers / dead-code
/// counts are guaranteed to be pessimistic because constructor /
/// field-type / overload / inner-class dispatch all silently degrade
/// to the pre-rev-7 heuristic. Consumers MUST surface this caveat as
/// "re-parse your repository" rather than trust the numbers for
/// trend-tracking.
///
/// Value string is versioned (`_v1`) so future schema bumps can
/// introduce `CAVEAT_STALE_PARSE_DB_V2` etc. without renaming the
/// existing caveat.
pub const CAVEAT_STALE_PARSE_DB_V1: &str = "stale_parse_db_v1";

/// Emitted when `ge-analyze` loads a parse DB containing edge rows
/// whose `kind` column did not deserialise into a known `EdgeKind`
/// variant. Universal-fidelity sprint P1.c introduced
/// `PersistedEdgeKind { Known(EdgeKind), Unknown(String) }` at the
/// SQLite read boundary specifically so forward-compat with future
/// engine versions (Layer-6 edges, new framework / declarative
/// variants) does not silently drop edges without an audit trail.
///
/// When this caveat is present, the analysis has measured a strict
/// subset of the graph the parser emitted. Metrics computed over the
/// loaded subset are correct **for that subset**; the fidelity
/// comparison between two reports is not apples-to-apples unless
/// both reports were produced against DBs whose engine version
/// emitted a superset of this engine's `EdgeKind`. Downstream tools
/// should surface the count (carried on `AnalysisGraph::unknown_edge_kind_count`)
/// as "parse DB written by a newer engine; re-run with the newer
/// binary for full coverage" rather than treat the report as a
/// regression.
///
/// Value string is versioned (`_v1`) so a future change to the
/// wire-format (e.g., renaming `tag` / `content` keys in
/// `#[serde(tag, content)]`) can introduce `_V2` without breaking
/// this caveat's meaning.
pub const CAVEAT_UNKNOWN_EDGE_KIND_SKIP_V1: &str = "unknown_edge_kind_skip_v1";

/// Stamped on every report produced by an engine that bundled a
/// Layer 0 git-signal pass via `graphengine-git-signals`. Reports
/// missing this caveat either (a) predate T7 or (b) were produced
/// with `--no-git-signals`. The `git_signals` block in the JSON
/// payload will be absent in both cases; consumers MUST NOT infer
/// "no churn" from absence. See T7 in the universal-fidelity sprint.
///
/// Re-exported from `graphengine_git_signals::CAVEAT_LAYER0_GIT_SIGNALS_V1`
/// so both crates stamp the identical string without accidental
/// drift. Any change to the value there must be mirrored here —
/// both constants carry the same stability contract.
pub use graphengine_git_signals::CAVEAT_LAYER0_GIT_SIGNALS_V1;

/// Stamped when the repository being scanned classified as
/// [`graphengine_git_signals::RepoShape::Shallow`] or `Bare`. Every
/// [`graphengine_git_signals::FileSignals::confidence`] in the
/// attached `git_signals` block will be `Low`; churn numbers are
/// present but not trustworthy as a ranking signal. Downstream
/// tools must surface this as "clone is shallow — churn ranking
/// disabled" rather than render the top-hotspot list as if the
/// repo had full history.
pub use graphengine_git_signals::CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1;

/// Stamped when the scan target is not a git working tree at all
/// (tarball extract, non-git VCS, plain directory). The
/// `git_signals` block will still be emitted — the report carries
/// [`graphengine_git_signals::RepoShape::NonGit`] and an empty
/// `per_file` map — so downstream consumers have a single shape
/// contract to render against rather than a missing field.
pub use graphengine_git_signals::CAVEAT_LAYER0_UNSUPPORTED_VCS_V1;

/// Stamped on every report whose attached `file_extraction_coverage`
/// map contains at least one file with an invalidating
/// [`graphengine_parsing::application::ports::CoverageGap`] (R39 or
/// R41 on Apex today). The caveat tells downstream consumers that
/// `dead_code.no_callers` confidence was downgraded from `High` to
/// `Medium` on at least one candidate because the extractor did
/// not walk the full AST of that file — consumers must treat the
/// non-downgraded companion metric
/// (`no_callers_high_confidence`) as the authoritative headline for
/// "dead code we are confident about" rather than the legacy
/// `no_callers_total`. See T8 in the universal-fidelity sprint
/// (`docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`).
pub const CAVEAT_EXTRACTION_COVERAGE_GAPS_V1: &str = "extraction_coverage_gaps_v1";

/// S2 v1 warm-rescan fast path reused a prior full analysis report
/// because the parse-layer delta was below the configured threshold.
/// Metrics may be stale for symbols in `changed_paths` / removed files.
/// Re-run `gridseak scan --no-incremental` (full reparse + full analysis)
/// when you need headline numbers you can treat as fresh.
pub const CAVEAT_INCREMENTAL_ANALYSIS_REUSED_V1: &str = "incremental_analysis_reused_v1";

/// S2-γ L1: global segments reused from cache; call-graph topology unchanged.
/// Symbols in `changed_paths` may have stale complexity/dead-code findings.
pub const CAVEAT_INCREMENTAL_ANALYSIS_SEGMENTS_MERGED_V1: &str =
    "incremental_analysis_segments_merged_v1";

/// S2-γ L2: call-graph topology changed on a small delta; wiring detectors rerun.
pub const CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1: &str =
    "incremental_analysis_structure_changed_v1";

// ---------------------------------------------------------------------------
// Top-level report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub version: String,
    pub generated_at: String,
    pub analysis_duration_ms: u64,
    pub db_path: String,

    /// Backward-compat: when norms are available this equals composite_percentile,
    /// otherwise derived from the legacy weighted formula. Omitted (null) when neither
    /// is computable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_score: Option<u32>,

    /// Legacy weighted sub-scores. Populated from per-metric percentiles when norms
    /// are available, otherwise from the formula-based fallback during transition.
    pub health_score_components: HealthScoreComponents,

    /// Layer A — absolute, transparent, coverage-style metrics.
    pub metrics: MetricsReport,

    /// Layer B — comparative percentile rank against the population database.
    /// Present only when `--norms` is provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentiles: Option<PercentilesReport>,

    pub summary: Summary,

    pub findings: Vec<Finding>,

    pub node_annotations: BTreeMap<String, NodeAnnotation>,

    pub module_annotations: BTreeMap<String, ModuleAnnotation>,

    pub classifications: BTreeMap<String, ModuleClassification>,

    pub boundary_violations: Vec<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_quality: Option<ResolutionQuality>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub analysis_errors: Vec<AnalysisError>,

    /// Self-describing provenance / correctness metadata for this report.
    /// Downstream consumers should check `schema_caveats` before comparing
    /// numeric values across reports (e.g., trend lines). Reports missing
    /// this field entirely (old data) should be treated as having
    /// `schema_caveats = [CAVEAT_LEGACY_PRE_ORDERFIX]`.
    #[serde(default)]
    pub integrity_status: IntegrityStatus,

    /// S2-γ incremental analysis provenance (trust ladder L0–L3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_provenance: Option<AnalysisProvenance>,

    /// T7 Layer 0 git-signal payload. `Some` when a git-signal pass
    /// ran on the scan target (the engine's default behaviour);
    /// `None` when the operator passed `--no-git-signals` or when
    /// the build did not bundle the extractor. Absence MUST NOT be
    /// interpreted as "no churn" — it means "no measurement was
    /// taken". Presence with `repository_shape = NonGit` means the
    /// scan target has no VCS history to measure.
    ///
    /// Downstream predicates should consume this via the
    /// `GitSignalConsumer` trait
    /// (`graphengine_git_signals::predicates::GitSignalConsumer`)
    /// rather than reading `change_frequency` / `distinct_authors`
    /// directly, so the `Confidence::Low` gate on shallow clones
    /// is enforced by the type system rather than remembered by
    /// every consumer.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub git_signals: Option<graphengine_git_signals::GitSignalReport>,

    /// T8 extraction-coverage payload: one entry per parsed file
    /// whose language ran an extraction-coverage pass. Keyed by the
    /// file path reported in `FileExtractionCoverage::file_path`.
    ///
    /// Empty vec MUST be interpreted as "no coverage pass ran" (e.g.,
    /// language has no D2-validated gap shapes yet). A non-empty vec
    /// with no `coverage_gaps` anywhere means "coverage pass ran,
    /// nothing metric-sensitive was missed". Consumers that want a
    /// boolean "any invalidating gap on this scan" should iterate
    /// and call
    /// [`graphengine_parsing::application::ports::FileExtractionCoverage::has_invalidating_no_callers_gap`].
    ///
    /// See `docs/workstreams/universal-fidelity/tasks/T8-coverage-awareness.md`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub file_extraction_coverage:
        Vec<graphengine_parsing::application::ports::FileExtractionCoverage>,

    /// Canonical primary language for this scan, computed by the
    /// analyzer from File-node majority (see
    /// [`crate::health::graph::detect_primary_language`]). Polyglot-
    /// follow-up A3: this replaces the unreliable
    /// `Project.properties.language` value previously set by the
    /// parser, which was clobbered by the last
    /// `INSERT OR REPLACE` from the polyglot orchestrator.
    ///
    /// `None` means no File nodes had a `language` property — typically
    /// an empty repo or one where every extractor failed. Consumers
    /// (CLI hero badge, `scan_runs.primary_language`, AI summary)
    /// should treat `None` as "unknown" rather than guessing.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub primary_language: Option<String>,
}

// ---------------------------------------------------------------------------
// Integrity status
// ---------------------------------------------------------------------------

/// Provenance + correctness metadata stamped onto every report produced by
/// this engine. Consumers use it to distinguish reports emitted by
/// different pipeline generations and to decide whether historical values
/// can be trusted for trend comparisons.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrityStatus {
    /// Version of the `graphengine-analysis` crate that produced this
    /// report. Populated from `env!("CARGO_PKG_VERSION")` at runtime.
    pub engine_version: String,

    /// Optional engine commit SHA. Populated when the build pipeline
    /// injects `GE_COMMIT_SHA` at compile time; otherwise None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_commit: Option<String>,

    /// Opaque tokens stamping this report as having had specific
    /// pipeline fixes/features applied. See the `CAVEAT_*` constants at
    /// the top of this module for the authoritative list of values and
    /// their meanings. Downstream code should match on the constants,
    /// never on the raw string.
    #[serde(default)]
    pub schema_caveats: Vec<String>,

    /// True if `AnalysisGraph::validate_invariants()` found any violations
    /// during this run. When true, the numeric values below should be
    /// treated as suspect. Each violation is also appended to
    /// `HealthReport.analysis_errors`.
    #[serde(default)]
    pub invariant_violations: bool,
}

/// S2-γ trust-ladder metadata for agents and UI.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalysisProvenance {
    pub analysis_mode: String,
    pub trust_level: String,
    pub structure_fingerprint: String,
    pub structure_changed: bool,
    #[serde(default)]
    pub segments_reused: Vec<String>,
    #[serde(default)]
    pub segments_rerun: Vec<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub delta_fingerprint: String,
    /// Set when L1/L2 serves findings about symbols in the delta set.
    #[serde(default)]
    pub query_trust_note: Option<String>,
}

// ---------------------------------------------------------------------------
// Layer A — Absolute Metrics (coverage-style, always present)
// ---------------------------------------------------------------------------

/// Per-metric confidence signal. Downstream consumers (desktop UI, CI action,
/// cloud dashboards) MUST consult this before rendering the raw numeric
/// value of a metric. Rendering `0.000` as a trustworthy "zero cycles"
/// signal when `status != Ok` is a UX defect.
///
/// Ordering matters for this enum: deserializers that don't yet know about
/// newer variants will fail; keep additions at the end and always add
/// `#[serde(other)]` support in consumers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    /// The metric was computed normally from a graph that had the edges it
    /// needed. Safe to render the raw numeric value.
    #[default]
    Ok,
    /// The production edge count is below the per-metric threshold needed
    /// for the result to be meaningful. The raw number may be technically
    /// correct but is not interpretable as "low" health — it's "we have no
    /// data". Downstream UIs must replace the number with a state badge.
    InsufficientEdges,
    /// The codebase belongs to an ecosystem known to use declarative wiring
    /// (Django URLconf, Salesforce Apex metadata, Spring XML, Rails routes,
    /// etc.) and import-resolution quality is low — meaning the parsed call
    /// graph is almost certainly missing the dispatch edges that would
    /// produce meaningful values for this metric. Raw numeric value is
    /// likely an underestimate.
    FrameworkInvisible,
    /// The metric does not apply to this repository (e.g., distance-from-
    /// main-sequence on a single-module repo). Numeric value should be
    /// hidden entirely, not just badged.
    NotApplicable,
    /// The analysis algorithm panicked or errored during computation.
    /// Inspect `HealthReport.analysis_errors` for details. Numeric value
    /// represents a safe default (0 / empty), not a measurement.
    ComputationFailed,
}

/// T3 dual-metric emission payload.
///
/// Every Layer-3 metric is computed twice — once over all production
/// structural edges, once over the high-confidence subset — and this
/// struct carries the comparison so downstream consumers can render
/// the metric's truthfulness alongside its value. A large gap means
/// the metric's number is dominated by heuristic edges; a small gap
/// means the two views agree and the metric is well-grounded in
/// authoritative evidence.
///
/// Shape is deliberately metric-agnostic: every Layer-3 metric maps
/// its "headline number" to a single f64 (cycles → count, coupling →
/// avg, depth → max, dead_code → ratio, etc.). Details on the metric-
/// specific shape remain in the metric's own detail struct; this
/// block is strictly about the all-vs-high-only divergence.
///
/// Missing field in a decoded report implies the report predates
/// `CAVEAT_DUAL_METRIC_EMISSION_V1` — downstream consumers must not
/// infer "zero gap" from absence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FidelityGap {
    /// Headline numeric value computed over every edge in the
    /// production structural edge set (heuristic + authoritative).
    pub all_edges_value: f64,
    /// Same headline, recomputed after dropping every edge whose
    /// provenance confidence is not `High`. Equals `all_edges_value`
    /// only when every production edge is authoritative.
    pub high_only_value: f64,
    /// Signed difference `all_edges_value - high_only_value`. Positive
    /// for metrics that grow with edge count (coupling, cycles, depth)
    /// when the heuristic edges are dropped; can be negative for
    /// inverse-coded metrics. Callers interpret sign per-metric.
    pub absolute_gap: f64,
    /// `absolute_gap / all_edges_value.abs()`, clamped to `[0, 1]`.
    /// `None` when `all_edges_value` is zero (division is undefined —
    /// consumer must not interpret this as "perfect agreement").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_gap: Option<f64>,
    /// Number of edges in the production set and the high-only
    /// subset, recorded so consumers can tell "gap is 100% because
    /// zero high-only edges exist" from "gap is 100% because the
    /// high-only subgraph has different topology".
    pub all_edges_count: usize,
    pub high_only_edges_count: usize,
}

impl FidelityGap {
    /// Build a gap block from the two computed headline values and
    /// the edge-set sizes they were measured over. Relative gap is
    /// `None` when `all_edges_value` is zero to avoid lying with a
    /// synthetic "0 / 0 = 0" result.
    pub fn from_values(
        all_edges_value: f64,
        high_only_value: f64,
        all_edges_count: usize,
        high_only_edges_count: usize,
    ) -> Self {
        let absolute_gap = all_edges_value - high_only_value;
        let relative_gap = if all_edges_value.abs() > f64::EPSILON {
            Some((absolute_gap / all_edges_value.abs()).clamp(-1.0, 1.0))
        } else {
            None
        };
        Self {
            all_edges_value,
            high_only_value,
            absolute_gap,
            relative_gap,
            all_edges_count,
            high_only_edges_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDetail {
    pub count: usize,
    pub total: usize,
    pub ratio: f64,
    pub description: String,
    #[serde(default)]
    pub status: MetricStatus,
    /// Dual-metric emission block (T3). Absent on reports produced by
    /// engines predating `CAVEAT_DUAL_METRIC_EMISSION_V1`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fidelity: Option<FidelityGap>,
}

// ---------------------------------------------------------------------------
// Dead-code reason classifier — per-function + aggregate distribution
// ---------------------------------------------------------------------------

/// Why a function was judged dead. Populated by the
/// `dead_code_classifier` registry (see
/// `graphengine-analysis/src/health/dead_code_classifier/mod.rs`).
///
/// Ordering / additions: new variants are additive; downstream consumers
/// must tolerate unknown variants (deserialize with `#[serde(other)]` or
/// default to `Unclassified`). Never rename existing variants — the
/// snake_case literal is a cross-version contract just like the
/// `CAVEAT_*` constants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DeadCodeReason {
    /// Fan-in is zero, no entry-point rule matched, no framework marker.
    /// Highest-confidence "truly orphaned" verdict when also
    /// `visibility=private`. See `VisibilityPrivateUnused`.
    NoCallers,
    /// Function carries a framework entry-point marker (Apex
    /// `@AuraEnabled`, `@InvocableMethod`, `@RestResource`, Django
    /// `@api_view`, Celery `@task`, etc.) but no incoming edge was
    /// parsed. The caller is almost certainly the framework and the
    /// resolver hasn't bridged it yet. Not a delete candidate.
    FrameworkAnnotationUnresolved,
    /// Function is referenced by declarative wiring (Django `urls.py`
    /// string, Salesforce XML metadata, Spring XML, Rails routes.rb)
    /// that the engine can't yet parse. The classifier infers this
    /// from structural signals (controller-convention paths / names)
    /// when no direct annotation or marker fits.
    DeclarativeWiringUnparsed,
    /// Evidence the function is dispatched dynamically (reflection,
    /// `Type.forName().newInstance()`, Python `getattr`, JS
    /// `obj[name]()`). Must never be deleted based on static dead-
    /// code alone.
    DynamicDispatchTarget,
    /// Function was passed as a callback / function reference but the
    /// parser did not emit a resolved edge to it. Distinct from
    /// `DynamicDispatchTarget` because the caller site is known, only
    /// the edge is missing.
    CallbackTargetNotTracked,
    /// Function is only referenced from test files; it is not
    /// "dead" in the sense of "unused", but it is dead in production
    /// and should be flagged under a different banner. Kept separate
    /// so a team can filter it out of cleanup sprints.
    TestOnlyReference,
    /// Fan-in is zero, visibility is `private` / `fileprivate`, no
    /// entry-point marker. This is the highest-signal "delete me"
    /// category — nothing outside this file could be calling it.
    VisibilityPrivateUnused,
    /// The classifier could not narrow further. Either the ecosystem
    /// has no validated classifier module, or all rules returned None.
    /// Consumers should treat this as "manual review required" —
    /// NOT as "delete candidate".
    #[serde(other)]
    Unclassified,
}

impl DeadCodeReason {
    /// Short snake_case identifier. Stable across versions — used as a
    /// map key in `reason_breakdown` and in benchmark fixtures.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCallers => "no_callers",
            Self::FrameworkAnnotationUnresolved => "framework_annotation_unresolved",
            Self::DeclarativeWiringUnparsed => "declarative_wiring_unparsed",
            Self::DynamicDispatchTarget => "dynamic_dispatch_target",
            Self::CallbackTargetNotTracked => "callback_target_not_tracked",
            Self::TestOnlyReference => "test_only_reference",
            Self::VisibilityPrivateUnused => "visibility_private_unused",
            Self::Unclassified => "unclassified",
        }
    }

    /// Iterate all variants. Used by mod.rs when building a
    /// breakdown map that must always have the full key set, so
    /// consumers don't have to handle missing keys.
    pub fn all() -> &'static [Self] {
        &[
            Self::NoCallers,
            Self::FrameworkAnnotationUnresolved,
            Self::DeclarativeWiringUnparsed,
            Self::DynamicDispatchTarget,
            Self::CallbackTargetNotTracked,
            Self::TestOnlyReference,
            Self::VisibilityPrivateUnused,
            Self::Unclassified,
        ]
    }
}

/// Dead-code metric detail. Extends `MetricDetail` with a reason
/// distribution so consumers can tell "5 truly-unused" apart from
/// "42 framework-invisible". The raw `count` is the total dead
/// functions (framework-invisible included) for backward compat with
/// pre-classifier consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeMetricDetail {
    pub count: usize,
    pub total: usize,
    pub ratio: f64,
    pub description: String,
    #[serde(default)]
    pub status: MetricStatus,
    /// Histogram over `DeadCodeReason`. Always contains every reason
    /// key; reasons with zero count are still present so downstream
    /// charts can render a stable column set. Absent field (in old
    /// reports) should be treated as `{Unclassified: count}`.
    #[serde(default)]
    pub reason_breakdown: BTreeMap<String, usize>,
    /// Per-bucket honesty caveats for `reason_breakdown`. Maps a
    /// reason key (e.g. `framework_annotation_unresolved`) to a
    /// human-readable note explaining when that bucket may show 0
    /// not because no nodes fit the category, but because the parser
    /// does not yet extract the upstream signal needed to populate
    /// the bucket for the dominant language. Consumers MUST surface
    /// this caveat alongside any bucket count reported as 0 for
    /// languages whose framework-attribute extraction is incomplete
    /// (see `crate::health::dead_code_confidence`). Absent when the
    /// dominant ecosystem has full extraction coverage (Apex today)
    /// or when no dead-code classification pass ran.
    ///
    /// Schema-stable: never remove keys that are present; only add.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason_breakdown_caveats: Option<BTreeMap<String, String>>,
    /// Dual-metric emission block (T3). Absent on reports predating
    /// `CAVEAT_DUAL_METRIC_EMISSION_V1`. Because dead-code membership
    /// is itself edge-count-sensitive, the high-only recount almost
    /// always drifts UP (fewer edges → more nodes with zero fan-in).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fidelity: Option<FidelityGap>,

    /// T8 dual-metric companion for `no_callers` — total count of
    /// dead functions with `DeadCodeReason::NoCallers`, including
    /// those whose confidence was downgraded to `Medium` because
    /// their file had an invalidating extraction-coverage gap.
    ///
    /// Kept separate from the legacy `count` / `reason_breakdown`
    /// so pre-T8 consumers reading this report see identical
    /// numbers; post-T8 consumers compare
    /// `no_callers_total - no_callers_high_confidence` to estimate
    /// the blast radius of extractor gaps.
    ///
    /// Absent (`None`) on reports predating T8 or on reports
    /// produced with no dead-code classifier pass (e.g. early
    /// failures). Absence MUST NOT be substituted with `0`;
    /// consumers should render the metric as "not measured".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub no_callers_total: Option<usize>,

    /// T8 dual-metric companion — candidates whose confidence stayed
    /// at `Confidence::High` after the coverage-gap downgrade. On
    /// pre-T8 reports this equals `no_callers_total`; on post-T8
    /// reports any difference is the extraction-coverage blast
    /// radius.
    ///
    /// The authoritative "dead code we are confident about" number
    /// for downstream triage. Prefer this over raw `count` on any
    /// report whose `schema_caveats` includes
    /// `CAVEAT_EXTRACTION_COVERAGE_GAPS_V1`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub no_callers_high_confidence: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingMetricDetail {
    pub modules_measured: usize,
    pub modules_above_070: usize,
    pub modules_above_050: usize,
    pub avg_coupling: f64,
    pub description: String,
    #[serde(default)]
    pub status: MetricStatus,
    /// Dual-metric emission block (T3). Headline value is
    /// `avg_coupling`. Absent on reports predating
    /// `CAVEAT_DUAL_METRIC_EMISSION_V1`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fidelity: Option<FidelityGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthMetricDetail {
    pub max_call_depth: usize,
    pub description: String,
    #[serde(default)]
    pub status: MetricStatus,
    /// Dual-metric emission block (T3). Headline value is
    /// `max_call_depth` as f64. Absent on reports predating
    /// `CAVEAT_DUAL_METRIC_EMISSION_V1`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fidelity: Option<FidelityGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityMetricDetail {
    pub avg_cyclomatic: f64,
    pub avg_cognitive: f64,
    pub max_cyclomatic: u32,
    pub max_cognitive: u32,
    pub functions_above_threshold: usize,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalCouplingMetricDetail {
    pub high_coupling_pairs: usize,
    pub hidden_coupling_pairs: usize,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_level: Option<ModuleTemporalCouplingDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleTemporalCouplingDetail {
    pub total_module_pairs: usize,
    pub hidden_module_pairs: usize,
    pub top_pairs: Vec<ModuleTemporalPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleTemporalPair {
    pub module_a: String,
    pub module_b: String,
    pub co_change_count: usize,
    pub coupling_score: f64,
    pub has_import_coupling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricConfidence {
    pub level: Confidence,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CohesionMetricDetail {
    pub avg_cohesion: f64,
    pub low_cohesion_modules: usize,
    pub description: String,
    /// Dual-metric emission block (T3). Headline value is
    /// `avg_cohesion`. Absent on reports predating
    /// `CAVEAT_DUAL_METRIC_EMISSION_V1`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fidelity: Option<FidelityGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistanceMetricDetail {
    pub avg_distance: f64,
    pub zone_of_pain_modules: usize,
    pub zone_of_uselessness_modules: usize,
    pub description: String,
    /// Dual-metric emission block (T3). Headline value is
    /// `avg_distance`. Absent on reports predating
    /// `CAVEAT_DUAL_METRIC_EMISSION_V1`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fidelity: Option<FidelityGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsReport {
    pub cycles: MetricDetail,
    pub coupling: CouplingMetricDetail,
    pub hotspot_concentration: MetricDetail,
    pub dead_code: DeadCodeMetricDetail,
    pub depth: DepthMetricDetail,
    pub tangle_index: MetricDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<ComplexityMetricDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cohesion: Option<CohesionMetricDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_from_main_sequence: Option<DistanceMetricDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_coupling: Option<TemporalCouplingMetricDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_confidence: Option<BTreeMap<String, MetricConfidence>>,
}

// ---------------------------------------------------------------------------
// Layer B — Percentile Rank (comparative, optional)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PercentileEntry {
    pub value: f64,
    pub percentile: u32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PercentilesReport {
    pub population_size: usize,
    pub population_version: String,
    pub composite_percentile: u32,
    pub per_metric: BTreeMap<String, PercentileEntry>,
}

// ---------------------------------------------------------------------------
// Health score breakdown
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealthScoreComponents {
    pub cycle_severity: ScoreComponent,
    pub coupling_health: ScoreComponent,
    pub hotspot_concentration: ScoreComponent,
    pub dead_code_ratio: ScoreComponent,
    pub depth_complexity: ScoreComponent,
    pub complexity: ScoreComponent,
    pub cohesion: ScoreComponent,
    pub distance: ScoreComponent,
    pub temporal_coupling: ScoreComponent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreComponent {
    pub score: u32,
    pub weight: f64,
}

// ---------------------------------------------------------------------------
// Resolution quality (import resolution diagnostics)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionQuality {
    pub import_edges_total: usize,
    /// Legacy self-declared tier, retained for compat with pre-T4 readers.
    /// New consumers should read `measured_fidelity` instead — `resolution_tier`
    /// is derived from "did the parser see any Import edges + did the LSP
    /// flag set to true" rather than from how many call edges actually
    /// resolved at `Confidence::High`. See T4 in
    /// `docs/workstreams/universal-fidelity/NEW_ENGINEER_PRIMER.md`.
    pub resolution_tier: ResolutionTier,
    /// T4: empirical fidelity tier computed from `edges_by_confidence`
    /// over call-like edges. This is the source of truth for any
    /// downstream reader that asks "is this graph's call resolution
    /// authoritative?". Always populated, even on empty graphs
    /// (where it reports `Unknown`).
    pub measured_fidelity: MeasuredFidelity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionTier {
    Full,
    HeuristicOnly,
    None,
}

/// Confidence breakdown over a fixed set of edges. Populated by
/// `AnalysisGraph::{all_edges_by_confidence, call_edges_by_confidence}`.
/// Surfaced in the report so a reader can verify the `MeasuredFidelityTier`
/// decision rather than trust the engine's classification.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgesByConfidence {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub unknown: usize,
}

impl EdgesByConfidence {
    pub fn total(&self) -> usize {
        self.high + self.medium + self.low + self.unknown
    }

    /// Share of `High` edges over the total. Returns 0.0 on an empty
    /// set so callers can feed the output straight into tier thresholds
    /// without a pre-check. If you need to distinguish "empty" from
    /// "all non-High", consult `.total()` first.
    pub fn high_ratio(&self) -> f64 {
        let t = self.total();
        if t == 0 {
            0.0
        } else {
            self.high as f64 / t as f64
        }
    }
}

/// T4: empirical fidelity classification of a scan. Derived from
/// `call_edges_by_confidence`, NOT from a flag the resolver sets on
/// itself. The boundary values come from D1's per-canary study in
/// `docs/workstreams/universal-fidelity/DISCOVERY_REPORT.md §FIDELITY_THRESHOLD_CALIBRATION`.
///
/// | Tier              | High-on-call ratio |
/// |-------------------|--------------------|
/// | `Authoritative`   | >= 0.80            |
/// | `HeuristicPrimary`| [0.40, 0.80)       |
/// | `SyntacticOnly`   | (0.00, 0.40)       |
/// | `Unknown`         | zero call edges    |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasuredFidelityTier {
    Authoritative,
    HeuristicPrimary,
    SyntacticOnly,
    Unknown,
}

impl MeasuredFidelityTier {
    /// Classify a call-edge breakdown into a tier. `Unknown` when no
    /// call-like edges exist at all (tree-sitter-only graph, or a
    /// graph whose call extraction hasn't run). Otherwise bucket by
    /// `High`-on-call ratio at 40%/80% cuts.
    pub fn from_call_edges(call_edges: &EdgesByConfidence) -> Self {
        if call_edges.total() == 0 {
            return Self::Unknown;
        }
        let r = call_edges.high_ratio();
        if r >= 0.80 {
            Self::Authoritative
        } else if r >= 0.40 {
            Self::HeuristicPrimary
        } else {
            Self::SyntacticOnly
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasuredFidelity {
    pub tier: MeasuredFidelityTier,
    /// Ratio of High-confidence edges over total call-like edges.
    /// `None` when no call-like edges exist (paired with `tier = Unknown`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_ratio_on_calls: Option<f64>,
    /// Breakdown over call-like edges only (the tier's denominator).
    pub call_edges_by_confidence: EdgesByConfidence,
    /// Breakdown over every edge, including `Contains`. Reported for
    /// context — containment edges always carry `High`, so a healthy-
    /// looking `all_edges.high_ratio()` can mask a `SyntacticOnly`
    /// call graph.
    pub all_edges_by_confidence: EdgesByConfidence,
}

#[cfg(test)]
mod measured_fidelity_tests {
    use super::{EdgesByConfidence, MeasuredFidelityTier};

    fn calls(high: usize, medium: usize, low: usize, unknown: usize) -> EdgesByConfidence {
        EdgesByConfidence {
            high,
            medium,
            low,
            unknown,
        }
    }

    #[test]
    fn unknown_when_no_call_edges() {
        let e = calls(0, 0, 0, 0);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::Unknown
        );
    }

    #[test]
    fn authoritative_at_and_above_80pct() {
        // Exactly 80%.
        let e = calls(80, 10, 10, 0);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::Authoritative
        );
        // Above 80%.
        let e = calls(95, 5, 0, 0);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::Authoritative
        );
    }

    #[test]
    fn heuristic_primary_in_40_to_80pct_half_open() {
        // Exactly 40%.
        let e = calls(40, 30, 30, 0);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::HeuristicPrimary
        );
        // Just under 80%.
        let e = calls(79, 10, 11, 0);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::HeuristicPrimary
        );
    }

    #[test]
    fn syntactic_only_below_40pct_but_non_empty() {
        let e = calls(1, 10, 89, 0);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::SyntacticOnly
        );
        // Zero High but non-zero call edges is still `SyntacticOnly`,
        // not `Unknown` — `Unknown` means the parser never produced
        // a call-like edge at all.
        let e = calls(0, 0, 0, 5);
        assert_eq!(
            MeasuredFidelityTier::from_call_edges(&e),
            MeasuredFidelityTier::SyntacticOnly
        );
    }

    #[test]
    fn high_ratio_is_zero_on_empty() {
        assert_eq!(calls(0, 0, 0, 0).high_ratio(), 0.0);
    }

    #[test]
    fn high_ratio_matches_hand_calc() {
        let e = calls(3, 1, 0, 1);
        // 3 / 5 = 0.6
        assert!((e.high_ratio() - 0.6).abs() < 1e-12);
    }
}

// ---------------------------------------------------------------------------
// Summary statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub total_functions: usize,
    pub total_modules: usize,
    pub cycles_found: usize,
    pub cycle_total_nodes: usize,
    pub hotspot_count: usize,
    pub hotspot_threshold_fan_in: usize,
    pub high_coupling_modules: usize,
    pub dead_functions: usize,
    pub max_call_depth: usize,
    pub tangle_index: f64,
    pub avg_module_coupling: f64,
    pub avg_fan_in: f64,
    pub avg_fan_out: f64,
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    #[serde(rename = "type")]
    pub finding_type: FindingType,
    pub severity: Severity,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub node_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blast_radius: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,

    /// Indicates how reliable this finding is based on input data quality.
    /// `Low` when cross-file edges (Import) are missing from the graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,

    // Type-specific fields (flattened for convenience)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_in: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupling_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub_score: Option<f64>,

    // Temporal coupling fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_b: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub co_change_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_coupling_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_import_edge: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingType {
    CircularDependency,
    BlastRadiusHotspot,
    HighCoupling,
    PotentiallyUnreachable,
    DeepCallChain,
    InformationFlowBottleneck,
    HubNode,
    LowEncapsulation,
    BoundaryViolation,
    ExcessiveComplexity,
    TemporalCoupling,
    LowCohesion,
    LayerViolation,
    GodFunction,
    EntryPoint,
    ZoneOfPain,
    ZoneOfUselessness,
    /// Resolution quality signal: more than a configurable fraction of
    /// call/import/type edges came from the heuristic fallback rather
    /// than the LSP. Surfaced so users understand that cross-file
    /// findings downstream (HighCoupling, BlastRadiusHotspot,
    /// PotentiallyUnreachable, etc.) are computed over a call graph
    /// where the LSP was partially unavailable. Not a code-quality
    /// problem in the target repository — it's a diagnostic about the
    /// *analysis itself*. Wired by `health::resolution_degraded`.
    ResolutionDegraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn rank(&self) -> u8 {
        match self {
            Severity::Critical => 4,
            Severity::High => 3,
            Severity::Warning => 2,
            Severity::Info => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-node annotation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAnnotation {
    pub fqn: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    pub fan_in: usize,
    pub fan_out: usize,
    pub blast_radius: usize,
    pub depth_from_root: usize,
    pub information_flow_complexity: usize,
    pub is_hotspot: bool,
    pub is_dead: bool,
    /// Reason this node was judged dead (from the dead-code classifier
    /// registry). Present when `is_dead == true`; always `None` when
    /// the node is live. Absent field in old reports means the report
    /// predates `CAVEAT_DEAD_CODE_REASONS_V1` — downstream consumers
    /// should treat it as `Unclassified`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dead_code_reason: Option<DeadCodeReason>,
    /// Human-readable evidence string that narrates which signals the
    /// classifier considered and which decision it reached. Free-form,
    /// not machine-parsed. Examples:
    ///   "fan_in=0; visibility=private; no entry-point marker"
    ///   "fan_in=0; @AuraEnabled present; caller frame is LWC (unparsed)"
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dead_code_evidence: Option<String>,
    /// Which classifier module decided. `"apex"`, `"python"`,
    /// `"generic"`, etc. Populated when a classifier dispatched;
    /// absent otherwise. Useful for per-ecosystem validation and for
    /// users debugging "why did this classifier fire?".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dead_code_classifier: Option<String>,
    /// Confidence of the dead-code verdict itself. `High` by
    /// default (every verdict starts at High); downgraded to
    /// `Medium` by the T7 Layer 0 churn post-pass
    /// ([`super::git_signals_attach::apply_dead_code_churn_downgrade_to_annotations`])
    /// when the file has been edited within the
    /// `ACTIVE_RECENT_MAX_DAYS` window and the git-signal
    /// confidence is `High`. Absent (`None`) on reports that
    /// predate T7 or on nodes that were never classified as
    /// dead — consumers MUST treat absence as "no verdict," not
    /// "low confidence."
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dead_code_confidence: Option<Confidence>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub is_test: bool,
    pub cycle_member: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub cycle_ids: Vec<String>,
    pub hub_score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclomatic_complexity: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cognitive_complexity: Option<u32>,
    pub loc: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_layer: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_label: Option<String>,
    pub risk_level: RiskLevel,
}

// ---------------------------------------------------------------------------
// Per-module annotation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleAnnotation {
    pub is_production: bool,
    pub coupling_score: f64,
    pub internal_edges: usize,
    pub external_edges: usize,
    pub instability: f64,
    pub afferent_coupling: usize,
    pub efferent_coupling: usize,
    pub abstractness: f64,
    pub abstract_types: usize,
    pub total_types: usize,
    pub distance_from_main_sequence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    pub cohesion_score: f64,
    pub connected_components: usize,
    pub api_surface_ratio: f64,
    pub exported_functions: usize,
    pub total_functions: usize,
    pub total_nodes: usize,
    pub total_loc: usize,
    pub avg_layer_depth: f64,
    pub layer_violation_count: usize,
    pub risk_level: RiskLevel,
}

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Critical,
    High,
    Warning,
    Info,
    Healthy,
}

// ---------------------------------------------------------------------------
// Module classification (production vs. non-production)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleClassification {
    /// "production", "test", "test_support", "example", "benchmark",
    /// "fixture", "vendor", "generated", "config", "docs", "auxiliary"
    pub role: String,

    /// `true` = included in health score / findings.
    /// `false` = excluded (test, example, benchmark, fixture, vendor, etc.).
    /// This is the canonical boolean the frontend toggle binds to.
    pub counts_toward_score: bool,

    /// "definitive", "high", "medium", "low"
    pub confidence: String,

    /// Human-readable explanation: "imports pytest (structural tier 1)",
    /// "path contains 'test' at word boundary", "user override via config"
    pub reason: String,

    /// Origin of this classification: "structural", "path_heuristic", "user_config"
    pub source: String,
}

// ---------------------------------------------------------------------------
// Analysis errors (graceful degradation)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisError {
    pub algorithm: String,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes_affected: Option<usize>,
}
