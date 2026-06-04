//! **ResolutionDegraded** diagnostic finding (Sprint D.2).
//!
//! This finding is a *diagnostic about the analysis itself* — not a
//! structural quality concern in the scanned repository. It fires when
//! the LSP-backed semantic resolver was unavailable or unhealthy for a
//! meaningful fraction of resolution sites during parsing, and the
//! heuristic fallback had to cover the gap.
//!
//! # Why this matters
//!
//! Downstream cross-file findings (`HighCoupling`, `BlastRadiusHotspot`,
//! `PotentiallyUnreachable`, `HubNode`, `InformationFlowBottleneck`,
//! `LayerViolation`, `LowCohesion`) are computed over the call/import
//! graph the resolver produced. When that graph is dominated by
//! heuristic edges, its confidence distribution shifts: per-edge
//! provenance moves from `High` (LSP) to `Medium`/`Low` (heuristic)
//! and, under the short-name fanout cap introduced in Sprint B.7-P2
//! (`HEURISTIC_CALL_FANOUT_CAP = 8`), genuinely ambiguous short-name
//! call sites are dropped entirely. The metrics are still honest about
//! what they measured, but users should see one explicit signal
//! explaining *why* confidence degraded so they can decide whether to
//! re-run with a healthier LSP before acting on individual findings.
//!
//! # Trigger
//!
//! Fallback rate is computed over the total resolved edge population:
//!
//! ```text
//! fallback_rate = total_heuristic_fallbacks / (lsp_edges + heuristic_edges + total_heuristic_fallbacks)
//! ```
//!
//! Where `total_heuristic_fallbacks` is the sum of
//! `heuristic_call_fallbacks + heuristic_import_fallbacks +
//! heuristic_type_fallbacks` plus the `heuristic_call_ambiguous_drops`
//! counter from B.7-P2 (dropped sites are recovered-signal that LSP
//! would have provided).
//!
//! The default threshold is 10% — below that, the LSP path was
//! materially healthy and the finding does not fire. Above, severity
//! escalates:
//!
//! | Fallback rate  | Severity  |
//! |----------------|-----------|
//! | 10–25%         | Warning   |
//! | 25–50%         | High      |
//! | >= 50%         | Critical  |
//!
//! # Input contract
//!
//! All inputs are raw counters from
//! `graphengine_parsing::application::ports::ResolutionStatsSummary`
//! persisted to the database as scalar metadata keys during parsing
//! (see `orchestrator.rs`). The analysis side reads them back via
//! `graph::read_metadata` and passes them into [`evaluate`]. When the
//! keys are missing (e.g. a parse run predating D.2), [`evaluate`]
//! returns `None` — there is nothing honest to say about resolution
//! quality without the underlying counters.

use crate::health::report::{Confidence, Finding, FindingType, Severity};

/// Snapshot of resolution telemetry recovered from the graph's
/// metadata rows. All fields default to zero to keep the call site
/// forgiving when the parser ran before these keys existed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolutionStatsSnapshot {
    pub lsp_edges: usize,
    pub heuristic_edges: usize,
    pub heuristic_call_fallbacks: usize,
    pub heuristic_import_fallbacks: usize,
    pub heuristic_type_fallbacks: usize,
    /// Call sites dropped by the B.7-P2 fanout cap. Counted as
    /// fallback signal because LSP would have resolved them — they
    /// represent recoverable accuracy lost to the current analysis run.
    pub heuristic_call_ambiguous_drops: usize,
}

impl ResolutionStatsSnapshot {
    pub fn total_fallbacks(&self) -> usize {
        self.heuristic_call_fallbacks
            + self.heuristic_import_fallbacks
            + self.heuristic_type_fallbacks
            + self.heuristic_call_ambiguous_drops
    }

    /// Total resolved edges *plus* dropped-for-ambiguity sites. Dropped
    /// sites don't show up as edges in the graph but do represent
    /// resolution work performed, so we include them in the denominator
    /// so a scan that dropped everything doesn't read as "0% fallback"
    /// just because the edges didn't land.
    pub fn total_resolution_work(&self) -> usize {
        self.lsp_edges
            .saturating_add(self.heuristic_edges)
            .saturating_add(self.heuristic_call_ambiguous_drops)
    }

    /// Fraction of resolution work that came from the heuristic path
    /// (or was dropped because the LSP wasn't there to disambiguate).
    /// Returns `None` when no resolution work was recorded — that
    /// means either the language ran a legacy parse without telemetry
    /// or the repo produced zero call/import/type edges, neither of
    /// which is a degradation worth surfacing.
    pub fn fallback_rate(&self) -> Option<f64> {
        let total = self.total_resolution_work();
        if total == 0 {
            return None;
        }
        let fallback_numerator = self
            .heuristic_edges
            .saturating_add(self.heuristic_call_ambiguous_drops);
        Some(fallback_numerator as f64 / total as f64)
    }
}

/// Configurable thresholds for the finding. Defaults align with the
/// Sprint D plan ("above 10% fallback rate").
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Minimum fallback rate below which the finding does not fire at
    /// all. Default 0.10.
    pub warning: f64,
    /// Above this rate severity escalates to `High`. Default 0.25.
    pub high: f64,
    /// Above this rate severity escalates to `Critical`. Default 0.50.
    pub critical: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            warning: 0.10,
            high: 0.25,
            critical: 0.50,
        }
    }
}

/// Emit at most one `ResolutionDegraded` finding from a stats
/// snapshot. Returns `None` when:
///
/// * the snapshot recorded no resolution work (telemetry missing or
///   the repo yielded zero resolved edges), or
/// * the fallback rate is below [`Thresholds::warning`].
pub fn evaluate(stats: &ResolutionStatsSnapshot, thresholds: Thresholds) -> Option<Finding> {
    let rate = stats.fallback_rate()?;
    if rate < thresholds.warning {
        return None;
    }

    let severity = if rate >= thresholds.critical {
        Severity::Critical
    } else if rate >= thresholds.high {
        Severity::High
    } else {
        Severity::Warning
    };

    let total = stats.total_resolution_work();
    let fallback_count = stats
        .heuristic_edges
        .saturating_add(stats.heuristic_call_ambiguous_drops);

    let description = format!(
        "{:.1}% of resolution work came from the heuristic fallback ({} of {} sites); \
         LSP-dependent findings below are computed over a degraded call graph",
        rate * 100.0,
        fallback_count,
        total,
    );

    let detail = Some(format!(
        "LSP edges: {lsp}; heuristic edges: {heur}; dropped-for-ambiguity sites: {drops}; \
         heuristic call fallbacks: {call_fb}; heuristic import fallbacks: {import_fb}; \
         heuristic type fallbacks: {type_fb}. Dropped-for-ambiguity sites are recoverable \
         call-graph signal that LSP would have disambiguated — they exist because the \
         heuristic resolver refuses to emit edges when a short name matches more than \
         HEURISTIC_CALL_FANOUT_CAP candidates (currently 8). Restart the language server \
         or verify the resolver binary before acting on cross-file findings.",
        lsp = stats.lsp_edges,
        heur = stats.heuristic_edges,
        drops = stats.heuristic_call_ambiguous_drops,
        call_fb = stats.heuristic_call_fallbacks,
        import_fb = stats.heuristic_import_fallbacks,
        type_fb = stats.heuristic_type_fallbacks,
    ));

    Some(Finding {
        id: "resolution-degraded-1".into(),
        finding_type: FindingType::ResolutionDegraded,
        severity,
        description,
        detail,
        node_ids: Vec::new(),
        edge_ids: None,
        primary_node_id: None,
        metric_name: Some("lsp_fallback_rate".into()),
        metric_value: Some(rate),
        impact: Some(format!(
            "{} edge(s) / drop sites resolved without type information; \
             downstream HighCoupling, BlastRadiusHotspot, and PotentiallyUnreachable \
             findings should be read with this caveat in mind",
            fallback_count,
        )),
        blast_radius: None,
        recommendation: Some(
            "Rerun the parse with a healthy LSP: verify the language server binary is \
             installed and reachable, check for recent crashes in the supervisor logs, \
             and confirm the workspace root is correctly set. On Apex, re-verify the \
             apex-jorje-lsp.jar SHA256 and Temurin 17 availability as documented in \
             docs/workstreams/apex/INTEGRATION.md."
                .into(),
        ),
        // Setting an explicit Low/Medium confidence on this finding
        // would double-count the signal — the finding *is* the
        // confidence signal. Leave it unset so the frontend shows it
        // at face value.
        confidence: Some(match severity {
            Severity::Critical => Confidence::High,
            Severity::High => Confidence::High,
            Severity::Warning => Confidence::Medium,
            Severity::Info => Confidence::Low,
        }),
        cycle_length: None,
        fan_in: None,
        coupling_score: None,
        internal_edges: None,
        external_edges: None,
        count: Some(fallback_count),
        hub_score: None,
        file_a: None,
        file_b: None,
        co_change_count: None,
        temporal_coupling_score: None,
        has_import_edge: None,
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(lsp: usize, heur: usize, drops: usize) -> ResolutionStatsSnapshot {
        ResolutionStatsSnapshot {
            lsp_edges: lsp,
            heuristic_edges: heur,
            heuristic_call_fallbacks: 0,
            heuristic_import_fallbacks: 0,
            heuristic_type_fallbacks: 0,
            heuristic_call_ambiguous_drops: drops,
        }
    }

    #[test]
    fn zero_work_produces_no_finding() {
        let finding = evaluate(&ResolutionStatsSnapshot::default(), Thresholds::default());
        assert!(
            finding.is_none(),
            "no resolution work recorded — finding would be dishonest"
        );
    }

    #[test]
    fn below_warning_threshold_produces_no_finding() {
        // 5% fallback rate — below 10% default threshold
        let s = snap(95, 5, 0);
        assert!((s.fallback_rate().unwrap() - 0.05).abs() < 1e-9);
        assert!(evaluate(&s, Thresholds::default()).is_none());
    }

    #[test]
    fn exactly_at_warning_threshold_fires_warning() {
        // 10% fallback — at threshold
        let s = snap(90, 10, 0);
        let f = evaluate(&s, Thresholds::default()).expect("should fire at warning threshold");
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.finding_type, FindingType::ResolutionDegraded);
        assert_eq!(f.id, "resolution-degraded-1");
        assert_eq!(f.metric_name.as_deref(), Some("lsp_fallback_rate"));
        assert!((f.metric_value.unwrap() - 0.10).abs() < 1e-9);
    }

    #[test]
    fn high_fallback_rate_escalates_to_high() {
        // 30% fallback — between 25 and 50
        let s = snap(70, 30, 0);
        let f = evaluate(&s, Thresholds::default()).expect("should fire");
        assert_eq!(f.severity, Severity::High);
    }

    #[test]
    fn very_high_fallback_rate_escalates_to_critical() {
        // 60% fallback — above critical
        let s = snap(40, 60, 0);
        let f = evaluate(&s, Thresholds::default()).expect("should fire");
        assert_eq!(f.severity, Severity::Critical);
    }

    #[test]
    fn dropped_ambiguous_sites_count_as_fallback() {
        // Pure LSP edges but many drops — drops must shift severity up
        // because they represent lost signal the LSP path would have
        // provided.
        let s = snap(50, 0, 50);
        let rate = s.fallback_rate().expect("rate computable");
        assert!((rate - 0.50).abs() < 1e-9, "50/100 = 50%");
        let f = evaluate(&s, Thresholds::default()).expect("should fire on drops alone");
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(
            f.count,
            Some(50),
            "count field reflects heuristic_edges + drops"
        );
    }

    #[test]
    fn finding_description_mentions_rate_and_counts() {
        let s = snap(70, 20, 10);
        let f = evaluate(&s, Thresholds::default()).expect("should fire");
        assert!(
            f.description.contains("30.0%"),
            "description must report the computed fallback percentage, got {:?}",
            f.description
        );
        assert!(
            f.description.contains("30 of 100 sites"),
            "description must quote absolute counts, got {:?}",
            f.description
        );
    }

    #[test]
    fn finding_detail_includes_all_counter_fields() {
        let stats = ResolutionStatsSnapshot {
            lsp_edges: 100,
            heuristic_edges: 30,
            heuristic_call_fallbacks: 5,
            heuristic_import_fallbacks: 2,
            heuristic_type_fallbacks: 1,
            heuristic_call_ambiguous_drops: 10,
        };
        let f = evaluate(&stats, Thresholds::default()).expect("should fire");
        let detail = f.detail.expect("detail present");
        for expected in [
            "LSP edges: 100",
            "heuristic edges: 30",
            "dropped-for-ambiguity sites: 10",
            "heuristic call fallbacks: 5",
            "heuristic import fallbacks: 2",
            "heuristic type fallbacks: 1",
        ] {
            assert!(
                detail.contains(expected),
                "detail must contain {expected:?}, got: {detail}"
            );
        }
    }

    #[test]
    fn custom_thresholds_are_respected() {
        let s = snap(80, 20, 0); // 20% fallback
                                 // With defaults (warn=10%, high=25%, crit=50%) → Warning
        let f_default = evaluate(&s, Thresholds::default()).expect("fires");
        assert_eq!(f_default.severity, Severity::Warning);

        // With stricter thresholds (warn=15%, high=18%, crit=22%) → High
        let strict = Thresholds {
            warning: 0.15,
            high: 0.18,
            critical: 0.22,
        };
        let f_strict = evaluate(&s, strict).expect("fires");
        assert_eq!(f_strict.severity, Severity::High);
    }

    #[test]
    fn finding_is_stable_across_runs() {
        // Same inputs → identical output (id, description, severity).
        // This is the contract that lets the triage/override system
        // suppress specific findings reliably.
        let s = snap(70, 30, 0);
        let a = evaluate(&s, Thresholds::default()).expect("fires");
        let b = evaluate(&s, Thresholds::default()).expect("fires");
        assert_eq!(a.id, b.id);
        assert_eq!(a.description, b.description);
        assert_eq!(a.severity, b.severity);
    }
}
