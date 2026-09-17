//! Witness fusion: adjudicate multiple provenance sources into one edge stamp.
//!
//! Pure functions only — no I/O. Used by the heuristic fallback path to
//! record agreement on an already-resolved semantic edge (plan 02 Phase B).

use super::authority::{authority_rank, default_confidence, SourceSet};
use super::{Confidence, Provenance, ProvenanceSource};

/// Pick the highest-authority witness, apply default confidence, promote
/// Medium→High when >= 2 distinct sources agree, and record non-winners in
/// `corroborating`.
pub fn adjudicate(witnesses: &[ProvenanceSource]) -> Provenance {
    if witnesses.is_empty() {
        return Provenance::heuristic();
    }

    let winner = witnesses
        .iter()
        .copied()
        .max_by_key(|s| authority_rank(*s))
        .unwrap_or(ProvenanceSource::Heuristic);

    let mut corroborating = SourceSet::default();
    for w in witnesses {
        if *w != winner {
            corroborating.insert(*w);
        }
    }

    let distinct_count = witnesses
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();

    let mut confidence = default_confidence(winner);
    if distinct_count >= 2 && confidence == Confidence::Medium {
        confidence = Confidence::High;
    }

    Provenance {
        source: winner,
        confidence,
        corroborating,
    }
}

/// Parse stored provenance JSON and return calibrated `p_true`.
pub fn p_true_from_provenance_json(language: &str, raw: &str, edge_kind: &str) -> Option<f64> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let source = match value.get("source")?.as_str()? {
        "TreeSitter" => ProvenanceSource::TreeSitter,
        "Heuristic" => ProvenanceSource::Heuristic,
        "Lsp" => ProvenanceSource::Lsp,
        "Compiler" => ProvenanceSource::Compiler,
        "Runtime" => ProvenanceSource::Runtime,
        _ => return None,
    };
    Some(p_true(language, source, edge_kind))
}

/// Calibrated P(true) from per-(language, source, edge-kind) reliability
/// tables. Runtime observations are 1.0. Values are priors until Batch C
/// corpus training overwrites them.
pub fn p_true(language: &str, source: ProvenanceSource, edge_kind: &str) -> f64 {
    if source == ProvenanceSource::Runtime {
        return 1.0;
    }
    reliability(language, source, edge_kind)
}

fn reliability(language: &str, source: ProvenanceSource, edge_kind: &str) -> f64 {
    let lang = language.to_ascii_lowercase();
    match (lang.as_str(), source, edge_kind) {
        (_, ProvenanceSource::Compiler, "Call") => 0.92,
        (_, ProvenanceSource::Lsp, "Call") => 0.88,
        (_, ProvenanceSource::TreeSitter, "Call") => 0.70,
        (_, ProvenanceSource::Heuristic, "Call") => 0.45,
        (_, ProvenanceSource::Compiler, _) => 0.90,
        (_, ProvenanceSource::Lsp, _) => 0.85,
        (_, ProvenanceSource::TreeSitter, _) => 0.65,
        (_, ProvenanceSource::Heuristic, _) => 0.40,
        (_, ProvenanceSource::Runtime, _) => 1.0,
    }
}

/// Stamp heuristic agreement onto an existing semantic edge without changing
/// its winner source or downgrading confidence.
pub fn stamp_heuristic_corroboration(edge: &mut crate::domain::Edge) {
    edge.provenance
        .corroborating
        .insert(ProvenanceSource::Heuristic);
    if edge.provenance.confidence == Confidence::Medium {
        edge.provenance.confidence = Confidence::High;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjudicate_picks_highest_authority_winner() {
        let p = adjudicate(&[ProvenanceSource::Heuristic, ProvenanceSource::Compiler]);
        assert_eq!(p.source, ProvenanceSource::Compiler);
        assert_eq!(p.confidence, Confidence::High);
        assert!(p.corroborating.contains(ProvenanceSource::Heuristic));
    }

    #[test]
    fn adjudicate_promotes_medium_to_high_on_two_distinct_agreeing_sources() {
        let p = adjudicate(&[ProvenanceSource::Lsp, ProvenanceSource::Compiler]);
        assert_eq!(p.source, ProvenanceSource::Compiler);
        assert_eq!(p.confidence, Confidence::High);
    }

    #[test]
    fn adjudicate_single_witness_no_corroboration() {
        let p = adjudicate(&[ProvenanceSource::Compiler]);
        assert!(p.corroborating.is_empty());
    }

    #[test]
    fn stamp_heuristic_corroboration_promotes_medium_to_high() {
        use crate::domain::{Edge, EdgeKind};
        let mut edge = Edge::new(
            "a".into(),
            "b".into(),
            EdgeKind::Call,
            Provenance::new(ProvenanceSource::Compiler, Confidence::Medium),
        );
        stamp_heuristic_corroboration(&mut edge);
        assert!(edge
            .provenance
            .corroborating
            .contains(ProvenanceSource::Heuristic));
        assert_eq!(edge.provenance.confidence, Confidence::High);
        assert_eq!(edge.provenance.source, ProvenanceSource::Compiler);
    }

    #[test]
    fn adjudicate_is_order_independent() {
        let a = adjudicate(&[ProvenanceSource::Heuristic, ProvenanceSource::Compiler]);
        let b = adjudicate(&[ProvenanceSource::Compiler, ProvenanceSource::Heuristic]);
        assert_eq!(a.source, b.source);
        assert_eq!(a.confidence, b.confidence);
    }

    #[test]
    fn adjudicate_is_monotonic_in_witness_set() {
        let one = adjudicate(&[ProvenanceSource::Heuristic]);
        let two = adjudicate(&[ProvenanceSource::Heuristic, ProvenanceSource::Compiler]);
        assert!(authority_rank(two.source) >= authority_rank(one.source));
    }

    #[test]
    fn runtime_p_true_is_one() {
        assert_eq!(p_true("python", ProvenanceSource::Runtime, "Call"), 1.0);
    }

    #[test]
    fn compiler_call_prior_is_above_heuristic() {
        assert!(
            p_true("typescript", ProvenanceSource::Compiler, "Call")
                > p_true("typescript", ProvenanceSource::Heuristic, "Call")
        );
    }

    #[test]
    fn runtime_outranks_compiler() {
        let p = adjudicate(&[ProvenanceSource::Compiler, ProvenanceSource::Runtime]);
        assert_eq!(p.source, ProvenanceSource::Runtime);
        assert_eq!(p_true("go", p.source, "Call"), 1.0);
    }
}
