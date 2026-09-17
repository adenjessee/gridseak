//! Per-language resolution tier disclosure persisted into graph metadata.
//!
//! Distinct from measured-fidelity tiers in analysis reports: this records
//! which resolver path ran for a language pass, not the empirical call-edge
//! confidence mix.

use serde::{Deserialize, Serialize};

/// Which resolution tier was attempted or used for a language pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionTierKind {
    Layer2,
    SubprocessLsp,
    Heuristic,
}

/// Closed set of machine-readable skip reasons when semantic tiers did not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    AdapterInitFailed,
    ServerMissing,
    LanguageNotRouted,
    NoReferences,
    PolicyDisabled,
    IndexerFailed,
    IndexerNotProvisioned,
    IndexStale,
}

/// One row of per-language resolution disclosure written to
/// `resolution_disclosure_<lang>` graph metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionDisclosure {
    pub language: String,
    pub tier_attempted: ResolutionTierKind,
    pub tier_used: ResolutionTierKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<SkipReason>,
    /// Layer-2 / LSP call edges actually emitted to the graph for this language pass.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "high_edges")]
    pub emitted_edges: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_age_seconds: Option<u64>,
}

impl ResolutionDisclosure {
    pub fn metadata_key(language: &str) -> String {
        format!("resolution_disclosure_{language}")
    }

    pub fn layer2_active(language: &str, emitted_edges: u64) -> Self {
        Self {
            language: language.to_string(),
            tier_attempted: ResolutionTierKind::Layer2,
            tier_used: ResolutionTierKind::Layer2,
            skip_reason: None,
            emitted_edges: Some(emitted_edges),
            index_fingerprint: None,
            index_age_seconds: None,
        }
    }

    pub fn layer2_fallback_to_lsp(language: &str, reason: SkipReason) -> Self {
        Self {
            language: language.to_string(),
            tier_attempted: ResolutionTierKind::Layer2,
            tier_used: ResolutionTierKind::SubprocessLsp,
            skip_reason: Some(reason),
            emitted_edges: None,
            index_fingerprint: None,
            index_age_seconds: None,
        }
    }

    pub fn layer2_fallback_to_heuristic(language: &str, reason: SkipReason) -> Self {
        Self {
            language: language.to_string(),
            tier_attempted: ResolutionTierKind::Layer2,
            tier_used: ResolutionTierKind::Heuristic,
            skip_reason: Some(reason),
            emitted_edges: None,
            index_fingerprint: None,
            index_age_seconds: None,
        }
    }

    pub fn subprocess_lsp(language: &str, emitted_edges: Option<u64>) -> Self {
        Self {
            language: language.to_string(),
            tier_attempted: ResolutionTierKind::SubprocessLsp,
            tier_used: ResolutionTierKind::SubprocessLsp,
            skip_reason: None,
            emitted_edges,
            index_fingerprint: None,
            index_age_seconds: None,
        }
    }

    pub fn heuristic_only(language: &str, reason: SkipReason) -> Self {
        Self {
            language: language.to_string(),
            tier_attempted: ResolutionTierKind::SubprocessLsp,
            tier_used: ResolutionTierKind::Heuristic,
            skip_reason: Some(reason),
            emitted_edges: None,
            index_fingerprint: None,
            index_age_seconds: None,
        }
    }

    pub fn with_index_meta(mut self, fingerprint: String, age_seconds: u64) -> Self {
        self.index_fingerprint = Some(fingerprint);
        self.index_age_seconds = Some(age_seconds);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_snake_case() {
        let d = ResolutionDisclosure {
            language: "rust".into(),
            tier_attempted: ResolutionTierKind::Layer2,
            tier_used: ResolutionTierKind::SubprocessLsp,
            skip_reason: Some(SkipReason::AdapterInitFailed),
            emitted_edges: None,
            index_fingerprint: None,
            index_age_seconds: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tier_attempted\":\"layer2\""));
        assert!(json.contains("\"tier_used\":\"subprocess_lsp\""));
        assert!(json.contains("\"skip_reason\":\"adapter_init_failed\""));
        let back: ResolutionDisclosure = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn indexer_failed_round_trip_snake_case() {
        let d = ResolutionDisclosure {
            language: "typescript".into(),
            tier_attempted: ResolutionTierKind::Layer2,
            tier_used: ResolutionTierKind::Heuristic,
            skip_reason: Some(SkipReason::IndexerFailed),
            emitted_edges: None,
            index_fingerprint: None,
            index_age_seconds: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"skip_reason\":\"indexer_failed\""));
        let back: ResolutionDisclosure = serde_json::from_str(&json).unwrap();
        assert_eq!(back.skip_reason, Some(SkipReason::IndexerFailed));
    }

    #[test]
    fn layer2_fallback_to_heuristic_disclosure_shape() {
        let d = ResolutionDisclosure::layer2_fallback_to_heuristic(
            "typescript",
            SkipReason::IndexerFailed,
        );
        assert_eq!(d.tier_attempted, ResolutionTierKind::Layer2);
        assert_eq!(d.tier_used, ResolutionTierKind::Heuristic);
        assert_eq!(d.skip_reason, Some(SkipReason::IndexerFailed));
    }
}
