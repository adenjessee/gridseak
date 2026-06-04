//! Tiered resolver dispatcher for Apex.
//!
//! Selects between the primary [`SemanticResolver`] — normally
//! `apex-jorje` wrapped in a generic LSP resolver — and the pure-Rust
//! [`ApexHeuristicResolver`] fallback. Dispatch decisions are made at two
//! different scopes:
//!
//! - **Whole-run:** at construction, we probe the primary's
//!   `is_available()`. If it reports unavailable (Java missing, jar
//!   missing, server crashed mid-init), the dispatcher latches to
//!   heuristic for the entire run and logs the downgrade.
//! - **Per-request:** the primary resolver returns an `Err` or an empty
//!   result set for a request (e.g. an LSP timeout on a single file).
//!   The dispatcher re-asks the heuristic for that same request and
//!   merges the result. This keeps a single crashed LSP call from
//!   zeroing out an entire parse.
//!
//! Every edge the dispatcher emits carries its originating provenance
//! exactly as produced by the underlying resolver — LSP edges stay
//! `Provenance::Lsp`, heuristic edges stay `Provenance::Heuristic` — so
//! the resolution-quality telemetry block downstream sees the real
//! picture and not a post-merge muddle.
//!
//! ## Environment overrides
//!
//! - `GRAPHENGINE_APEX_RESOLVER=heuristic` forces heuristic-only. Useful
//!   for CI speed, reproducibility audits, and support scenarios where
//!   the user wants a known-deterministic tier.
//! - `GRAPHENGINE_APEX_RESOLVER=lsp` forces LSP-only (no fallback). If
//!   LSP is unavailable the dispatcher will surface the error rather
//!   than degrade silently. Use this when accuracy is non-negotiable and
//!   a visible failure is preferred over silently degraded output.

use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

use crate::application::ports::{ResolvedEdges, SemanticResolver, SyntaxResults};

/// Which resolver actually handled the run. Stamped into logs and
/// surfaced to telemetry so users understand what they got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverTier {
    /// LSP primary. Highest accuracy, requires Java + `apex-jorje-lsp.jar`.
    LspPrimary,
    /// Pure-Rust heuristic fallback. Always available.
    Heuristic,
    /// LSP succeeded for part of the run, heuristic filled the gaps.
    Merged,
}

impl ResolverTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolverTier::LspPrimary => "lsp_primary",
            ResolverTier::Heuristic => "heuristic",
            ResolverTier::Merged => "merged",
        }
    }
}

/// Env-var name for forcing a specific tier.
pub const ENV_APEX_RESOLVER: &str = "GRAPHENGINE_APEX_RESOLVER";

/// Explicit override value for environment-driven tier selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverOverride {
    /// No override — follow the dispatcher's automatic logic.
    Auto,
    /// Force LSP-only. Surface an error if LSP is unavailable.
    LspOnly,
    /// Force heuristic-only. Never attempt LSP.
    HeuristicOnly,
}

impl ResolverOverride {
    /// Read the `GRAPHENGINE_APEX_RESOLVER` env var, returning the
    /// resulting override. Unknown values fall back to `Auto` with a
    /// warning — never fail startup for a typoed env var.
    pub fn from_env() -> Self {
        let raw = match env::var(ENV_APEX_RESOLVER) {
            Ok(v) => v,
            Err(_) => return ResolverOverride::Auto,
        };
        Self::parse(&raw)
    }

    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => ResolverOverride::Auto,
            "lsp" | "lsp_only" | "lsp-only" => ResolverOverride::LspOnly,
            "heuristic" | "heuristic_only" | "heuristic-only" | "fallback" => {
                ResolverOverride::HeuristicOnly
            }
            other => {
                warn!(
                    "Unknown {ENV_APEX_RESOLVER}='{other}', falling back to auto. \
                     Accepted: auto | lsp | heuristic."
                );
                ResolverOverride::Auto
            }
        }
    }
}

/// Tiered Apex resolver. `primary` is typically an LSP resolver for
/// `apex-jorje`; `fallback` is [`super::ApexHeuristicResolver`].
///
/// The dispatcher owns the probe + selection logic but never the
/// resolvers themselves — callers construct each tier with its own
/// language config / registry and pass them in. This keeps the
/// dispatcher free of LSP-specific dependencies and makes it trivially
/// testable with arbitrary resolver mocks.
pub struct ApexResolverDispatcher {
    primary: Arc<dyn SemanticResolver>,
    fallback: Arc<dyn SemanticResolver>,
    override_mode: ResolverOverride,
}

impl ApexResolverDispatcher {
    /// Build a new dispatcher. The override is read from the
    /// environment by default; use [`Self::with_override`] for tests.
    pub fn new(primary: Arc<dyn SemanticResolver>, fallback: Arc<dyn SemanticResolver>) -> Self {
        Self::with_override(primary, fallback, ResolverOverride::from_env())
    }

    /// Build with an explicit override — avoids the env read. Callers
    /// running inside tests should use this.
    pub fn with_override(
        primary: Arc<dyn SemanticResolver>,
        fallback: Arc<dyn SemanticResolver>,
        override_mode: ResolverOverride,
    ) -> Self {
        Self {
            primary,
            fallback,
            override_mode,
        }
    }

    /// Peek at which tier this dispatcher will use, without actually
    /// running a resolution pass. Useful for logging on startup.
    pub async fn selected_tier(&self) -> ResolverTier {
        match self.override_mode {
            ResolverOverride::HeuristicOnly => ResolverTier::Heuristic,
            ResolverOverride::LspOnly => ResolverTier::LspPrimary,
            ResolverOverride::Auto => {
                if self.primary.is_available().await {
                    ResolverTier::LspPrimary
                } else {
                    ResolverTier::Heuristic
                }
            }
        }
    }
}

#[async_trait]
impl SemanticResolver for ApexResolverDispatcher {
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        match self.override_mode {
            ResolverOverride::HeuristicOnly => {
                info!("ApexResolverDispatcher: heuristic-only (env override)");
                return self.fallback.resolve(hints).await;
            }
            ResolverOverride::LspOnly => {
                info!("ApexResolverDispatcher: LSP-only (env override)");
                if !self.primary.is_available().await {
                    anyhow::bail!(
                        "{ENV_APEX_RESOLVER}=lsp but primary LSP is not available; \
                         refusing to silently downgrade. Install Java and the \
                         apex-jorje-lsp.jar, or unset the override to allow heuristic \
                         fallback."
                    );
                }
                return self.primary.resolve(hints).await;
            }
            ResolverOverride::Auto => {}
        }

        if !self.primary.is_available().await {
            warn!(
                "Apex LSP primary reports unavailable — falling back to heuristic \
                 resolver for this run. Set GRAPHENGINE_JAVA_HOME + \
                 GRAPHENGINE_APEX_JORJE_JAR, or install Java, to get top-tier accuracy."
            );
            return self.fallback.resolve(hints).await;
        }

        // Happy path: run the LSP primary. On error, merge in heuristic
        // output rather than aborting the whole run.
        match self.primary.resolve(hints).await {
            Ok(primary_edges) => {
                // Even on success, kick off heuristic for gap-filling
                // on the exact inputs the LSP didn't cover. We keep the
                // primary's edges untouched and only add heuristic
                // edges for (caller, callee) pairs the primary missed.
                match self.fallback.resolve(hints).await {
                    Ok(fallback_edges) => Ok(merge_with_gap_fill(primary_edges, fallback_edges)),
                    Err(e) => {
                        warn!(
                            "Apex heuristic gap-fill failed (non-fatal): {e}; \
                             returning LSP-primary edges only."
                        );
                        Ok(primary_edges)
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Apex LSP primary resolve() errored ({e}); \
                     degrading to heuristic resolver for this run."
                );
                self.fallback.resolve(hints).await
            }
        }
    }

    fn supported_language(&self) -> &str {
        "apex"
    }

    /// The dispatcher is available as long as at least one tier is
    /// available. Since the heuristic tier is always available in
    /// practice, this returns `true` in all realistic configurations.
    async fn is_available(&self) -> bool {
        self.primary.is_available().await || self.fallback.is_available().await
    }

    /// Expose the **primary** (LSP) tier's session metrics. We
    /// deliberately ignore the fallback heuristic resolver here
    /// because:
    ///   * it has no LSP session to report on (it's pure Rust), and
    ///   * the telemetry's whole purpose is answering "did LSP do the
    ///     work?" — shadowing a healthy heuristic on top of a dead LSP
    ///     would defeat that.
    async fn session_metrics(&self) -> Option<crate::application::ports::SessionMetricsSnapshot> {
        self.primary.session_metrics().await
    }
}

/// Merge LSP-primary edges with heuristic gap-fill. Primary edges win
/// on (from, to, kind) collisions — their higher confidence and real
/// semantic resolution must never be downgraded by a heuristic pair
/// that happened to match the same endpoints.
fn merge_with_gap_fill(primary: ResolvedEdges, fallback: ResolvedEdges) -> ResolvedEdges {
    let mut merged = primary;
    merge_bucket(
        &mut merged.call_edges,
        fallback.call_edges,
        crate::domain::EdgeKind::Call,
    );
    merge_bucket(
        &mut merged.type_edges,
        fallback.type_edges,
        crate::domain::EdgeKind::Type,
    );
    merge_bucket(
        &mut merged.import_edges,
        fallback.import_edges,
        crate::domain::EdgeKind::Import,
    );
    merge_bucket(
        &mut merged.containment_edges,
        fallback.containment_edges,
        crate::domain::EdgeKind::Contains,
    );

    // Accumulate the heuristic gap-fill stats *on top* of the primary's
    // stats. This lets telemetry report both tiers separately.
    merged.stats.heuristic_edges += fallback.stats.heuristic_edges;
    merged.stats.heuristic_call_fallbacks += fallback.stats.heuristic_call_fallbacks;
    merged.stats.heuristic_import_fallbacks += fallback.stats.heuristic_import_fallbacks;
    merged.stats.heuristic_type_fallbacks += fallback.stats.heuristic_type_fallbacks;
    merged.stats.heuristic_call_ambiguous_drops += fallback.stats.heuristic_call_ambiguous_drops;
    for f in fallback.stats.heuristic_failures {
        merged.stats.heuristic_failures.push(f);
    }
    merged
}

fn merge_bucket(
    primary_bucket: &mut Vec<crate::domain::Edge>,
    fallback_bucket: Vec<crate::domain::Edge>,
    _kind: crate::domain::EdgeKind,
) {
    use std::collections::HashSet;
    let existing: HashSet<(String, String)> = primary_bucket
        .iter()
        .map(|e| (e.from_id.clone(), e.to_id.clone()))
        .collect();
    for e in fallback_bucket {
        if !existing.contains(&(e.from_id.clone(), e.to_id.clone())) {
            primary_bucket.push(e);
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{
        CallSite, ResolutionStatsSummary, ResolvedEdges, SemanticResolver, SyntaxResults,
    };
    use crate::domain::{Confidence, Edge, Provenance, ProvenanceSource, Range};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct StubResolver {
        available: AtomicBool,
        call_count: AtomicUsize,
        tag_from: String,
        tag_to: String,
        source: ProvenanceSource,
        should_err: AtomicBool,
    }

    impl StubResolver {
        fn new(available: bool, tag_from: &str, tag_to: &str, source: ProvenanceSource) -> Self {
            Self {
                available: AtomicBool::new(available),
                call_count: AtomicUsize::new(0),
                tag_from: tag_from.to_string(),
                tag_to: tag_to.to_string(),
                source,
                should_err: AtomicBool::new(false),
            }
        }

        fn set_err(&self, v: bool) {
            self.should_err.store(v, Ordering::SeqCst);
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SemanticResolver for StubResolver {
        async fn resolve(&self, _hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            if self.should_err.load(Ordering::SeqCst) {
                anyhow::bail!("stub forced error");
            }
            let mut e = ResolvedEdges::new();
            e.add_call_edge(Edge::call(
                self.tag_from.clone(),
                self.tag_to.clone(),
                Provenance::new(self.source, Confidence::Medium),
            ));
            e.stats = ResolutionStatsSummary {
                lsp_edges: if self.source == ProvenanceSource::Lsp {
                    1
                } else {
                    0
                },
                heuristic_edges: if self.source == ProvenanceSource::Heuristic {
                    1
                } else {
                    0
                },
                heuristic_call_fallbacks: if self.source == ProvenanceSource::Heuristic {
                    1
                } else {
                    0
                },
                ..Default::default()
            };
            Ok(e)
        }

        fn supported_language(&self) -> &str {
            "apex"
        }

        async fn is_available(&self) -> bool {
            self.available.load(Ordering::SeqCst)
        }
    }

    fn stub_call_site() -> CallSite {
        CallSite {
            location: Range::with_file(1, 0, 1, 10, "Test.cls".to_string()),
            function_name: "x".to_string(),
            receiver_range: None,
            receiver_text: None,
            arg_types: Vec::new(),
        }
    }

    #[tokio::test]
    async fn auto_uses_primary_when_available() {
        let primary = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_lsp",
            ProvenanceSource::Lsp,
        ));
        let fallback = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_heur",
            ProvenanceSource::Heuristic,
        ));
        let disp = ApexResolverDispatcher::with_override(
            primary.clone(),
            fallback.clone(),
            ResolverOverride::Auto,
        );

        let mut hints = SyntaxResults::new();
        hints
            .references
            .push(crate::application::ports::UnresolvedReference::Call(
                stub_call_site(),
            ));
        let out = disp.resolve(&hints).await.unwrap();

        assert_eq!(primary.calls(), 1);
        // Fallback also runs for gap-filling even on primary success.
        assert_eq!(fallback.calls(), 1);
        // Primary edge must be present, with LSP provenance intact.
        assert!(out
            .call_edges
            .iter()
            .any(|e| e.to_id == "callee_lsp" && e.provenance.source == ProvenanceSource::Lsp));
        // Heuristic gap-fill edge must also be present (different endpoints).
        assert!(out.call_edges.iter().any(|e| e.to_id == "callee_heur"));
    }

    #[tokio::test]
    async fn auto_falls_back_when_primary_unavailable() {
        let primary = Arc::new(StubResolver::new(
            false,
            "caller",
            "callee_lsp",
            ProvenanceSource::Lsp,
        ));
        let fallback = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_heur",
            ProvenanceSource::Heuristic,
        ));
        let disp = ApexResolverDispatcher::with_override(
            primary.clone(),
            fallback.clone(),
            ResolverOverride::Auto,
        );

        let out = disp.resolve(&SyntaxResults::new()).await.unwrap();
        assert_eq!(
            primary.calls(),
            0,
            "unavailable primary must not be invoked"
        );
        assert_eq!(fallback.calls(), 1);
        assert!(out
            .call_edges
            .iter()
            .all(|e| e.provenance.source == ProvenanceSource::Heuristic));
    }

    #[tokio::test]
    async fn lsp_only_errors_if_primary_unavailable() {
        let primary = Arc::new(StubResolver::new(
            false,
            "caller",
            "callee_lsp",
            ProvenanceSource::Lsp,
        ));
        let fallback = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_heur",
            ProvenanceSource::Heuristic,
        ));
        let disp = ApexResolverDispatcher::with_override(
            primary,
            fallback.clone(),
            ResolverOverride::LspOnly,
        );

        let err = disp
            .resolve(&SyntaxResults::new())
            .await
            .expect_err("lsp-only must error, not silently fall back");
        let msg = format!("{err}");
        assert!(msg.contains("refusing to silently downgrade"), "got: {msg}");
        assert_eq!(fallback.calls(), 0, "fallback must not run under lsp-only");
    }

    #[tokio::test]
    async fn heuristic_only_skips_primary_entirely() {
        let primary = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_lsp",
            ProvenanceSource::Lsp,
        ));
        let fallback = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_heur",
            ProvenanceSource::Heuristic,
        ));
        let disp = ApexResolverDispatcher::with_override(
            primary.clone(),
            fallback.clone(),
            ResolverOverride::HeuristicOnly,
        );

        let out = disp.resolve(&SyntaxResults::new()).await.unwrap();
        assert_eq!(
            primary.calls(),
            0,
            "primary must not run under heuristic-only"
        );
        assert_eq!(fallback.calls(), 1);
        assert!(out
            .call_edges
            .iter()
            .all(|e| e.provenance.source == ProvenanceSource::Heuristic));
    }

    #[tokio::test]
    async fn primary_error_triggers_fallback_not_hard_failure() {
        let primary = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_lsp",
            ProvenanceSource::Lsp,
        ));
        primary.set_err(true);
        let fallback = Arc::new(StubResolver::new(
            true,
            "caller",
            "callee_heur",
            ProvenanceSource::Heuristic,
        ));
        let disp = ApexResolverDispatcher::with_override(
            primary,
            fallback.clone(),
            ResolverOverride::Auto,
        );

        let out = disp
            .resolve(&SyntaxResults::new())
            .await
            .expect("primary error must not abort when fallback is available");
        assert_eq!(fallback.calls(), 1);
        assert!(out
            .call_edges
            .iter()
            .all(|e| e.provenance.source == ProvenanceSource::Heuristic));
    }

    #[tokio::test]
    async fn gap_fill_never_overwrites_primary_edge() {
        // Both resolvers emit an edge with the SAME endpoints; primary
        // is LSP, fallback is Heuristic. After merge we MUST see the
        // LSP edge and not the heuristic one.
        let primary = Arc::new(StubResolver::new(true, "from", "to", ProvenanceSource::Lsp));
        let fallback = Arc::new(StubResolver::new(
            true,
            "from",
            "to",
            ProvenanceSource::Heuristic,
        ));
        let disp = ApexResolverDispatcher::with_override(
            primary.clone(),
            fallback.clone(),
            ResolverOverride::Auto,
        );

        let out = disp.resolve(&SyntaxResults::new()).await.unwrap();
        assert_eq!(out.call_edges.len(), 1);
        assert_eq!(
            out.call_edges[0].provenance.source,
            ProvenanceSource::Lsp,
            "primary provenance must win on endpoint collision"
        );
    }

    #[tokio::test]
    async fn selected_tier_reflects_overrides_and_availability() {
        let primary_up = Arc::new(StubResolver::new(true, "a", "b", ProvenanceSource::Lsp));
        let primary_down = Arc::new(StubResolver::new(false, "a", "b", ProvenanceSource::Lsp));
        let fallback = Arc::new(StubResolver::new(
            true,
            "a",
            "b",
            ProvenanceSource::Heuristic,
        ));

        let d_up_auto = ApexResolverDispatcher::with_override(
            primary_up.clone(),
            fallback.clone(),
            ResolverOverride::Auto,
        );
        assert_eq!(d_up_auto.selected_tier().await, ResolverTier::LspPrimary);

        let d_down_auto = ApexResolverDispatcher::with_override(
            primary_down,
            fallback.clone(),
            ResolverOverride::Auto,
        );
        assert_eq!(d_down_auto.selected_tier().await, ResolverTier::Heuristic);

        let d_lsp_only = ApexResolverDispatcher::with_override(
            primary_up.clone(),
            fallback.clone(),
            ResolverOverride::LspOnly,
        );
        assert_eq!(d_lsp_only.selected_tier().await, ResolverTier::LspPrimary);

        let d_heur_only = ApexResolverDispatcher::with_override(
            primary_up,
            fallback,
            ResolverOverride::HeuristicOnly,
        );
        assert_eq!(d_heur_only.selected_tier().await, ResolverTier::Heuristic);
    }

    #[test]
    fn env_override_parser_accepts_documented_values() {
        assert_eq!(ResolverOverride::parse(""), ResolverOverride::Auto);
        assert_eq!(ResolverOverride::parse("auto"), ResolverOverride::Auto);
        assert_eq!(ResolverOverride::parse("AUTO"), ResolverOverride::Auto);
        assert_eq!(ResolverOverride::parse("lsp"), ResolverOverride::LspOnly);
        assert_eq!(
            ResolverOverride::parse("lsp_only"),
            ResolverOverride::LspOnly
        );
        assert_eq!(
            ResolverOverride::parse("lsp-only"),
            ResolverOverride::LspOnly
        );
        assert_eq!(
            ResolverOverride::parse("heuristic"),
            ResolverOverride::HeuristicOnly
        );
        assert_eq!(
            ResolverOverride::parse("fallback"),
            ResolverOverride::HeuristicOnly
        );
        assert_eq!(
            ResolverOverride::parse("garbage-value"),
            ResolverOverride::Auto,
            "unknown values must not panic; they fall back to auto"
        );
    }

    #[test]
    fn resolver_tier_strings_are_stable() {
        // Telemetry consumers key on these exact strings.
        assert_eq!(ResolverTier::LspPrimary.as_str(), "lsp_primary");
        assert_eq!(ResolverTier::Heuristic.as_str(), "heuristic");
        assert_eq!(ResolverTier::Merged.as_str(), "merged");
    }
}
