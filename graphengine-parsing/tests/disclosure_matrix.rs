//! Language × present/missing/failed/stale → SkipReason matrix (Batch B).

#![cfg(feature = "scip")]

use graphengine_parsing::application::resolution_disclosure::{
    ResolutionDisclosure, ResolutionTierKind, SkipReason,
};

fn row(language: &str, reason: SkipReason) -> ResolutionDisclosure {
    ResolutionDisclosure::layer2_fallback_to_heuristic(language, reason)
}

#[test]
fn present_index_is_layer2_no_skip() {
    let d = ResolutionDisclosure::layer2_active("typescript", 12);
    assert_eq!(d.tier_used, ResolutionTierKind::Layer2);
    assert!(d.skip_reason.is_none());
}

#[test]
fn missing_index_is_not_provisioned() {
    let d = row("go", SkipReason::IndexerNotProvisioned);
    assert_eq!(d.tier_used, ResolutionTierKind::Heuristic);
    assert_eq!(d.skip_reason, Some(SkipReason::IndexerNotProvisioned));
}

#[test]
fn failed_indexer_is_indexer_failed() {
    let d = row("python", SkipReason::IndexerFailed);
    assert_eq!(d.skip_reason, Some(SkipReason::IndexerFailed));
}

#[test]
fn stale_index_is_index_stale() {
    let d = row("typescript", SkipReason::IndexStale);
    assert_eq!(d.skip_reason, Some(SkipReason::IndexStale));
}

#[test]
fn rust_java_stub_is_not_provisioned() {
    for lang in ["java", "csharp", "cpp", "ruby"] {
        let d = row(lang, SkipReason::IndexerNotProvisioned);
        assert_eq!(d.skip_reason, Some(SkipReason::IndexerNotProvisioned));
    }
}
