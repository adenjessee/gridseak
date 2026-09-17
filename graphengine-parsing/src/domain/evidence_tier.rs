//! Single source of truth for agent-facing evidence tiers.
//!
//! Every consumer (CLI graph queries, MCP `tier_legend`, LIMITATIONS, Cursor
//! rules) must render from this module. Do not invent tier strings elsewhere.

use crate::domain::provenance::ProvenanceSource;

/// Product-facing evidence tier identifiers (`tier_0`, `tier_1`, `tier_3`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceTier {
    /// Tree-sitter parsed import / call site.
    Tier0,
    /// Filtered grep / name-match heuristic.
    Tier1,
    /// Compiler- or LSP-verified (rust-analyzer, SCIP, language server).
    Tier3,
    /// Observed at runtime (coverage or call-trace ingest).
    RuntimeVerified,
}

impl EvidenceTier {
    pub fn id(self) -> &'static str {
        match self {
            Self::Tier0 => "tier_0",
            Self::Tier1 => "tier_1",
            Self::Tier3 => "tier_3",
            Self::RuntimeVerified => "runtime_verified",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tier0 => "tree-sitter parsed import / call site (deterministic, fast)",
            Self::Tier1 => "filtered grep heuristic (may include false positives)",
            Self::Tier3 => "compiler- or LSP-verified (rust-analyzer / SCIP / language server)",
            Self::RuntimeVerified => {
                "observed at runtime via coverage or call-trace ingest (existence only)"
            }
        }
    }

    pub fn sources(self) -> &'static [&'static str] {
        match self {
            Self::Tier0 => &["TreeSitter"],
            Self::Tier1 => &["Heuristic"],
            Self::Tier3 => &["Compiler", "Lsp"],
            Self::RuntimeVerified => &["Runtime"],
        }
    }
}

/// Map a domain provenance source onto the product tier vocabulary.
///
/// Unknown / future sources return `None` so callers render "unknown tier"
/// instead of silently promoting them.
pub fn provenance_source_to_tier(source: ProvenanceSource) -> Option<EvidenceTier> {
    match source {
        ProvenanceSource::TreeSitter => Some(EvidenceTier::Tier0),
        ProvenanceSource::Heuristic => Some(EvidenceTier::Tier1),
        ProvenanceSource::Lsp | ProvenanceSource::Compiler => Some(EvidenceTier::Tier3),
        ProvenanceSource::Runtime => Some(EvidenceTier::RuntimeVerified),
    }
}

/// Parse a stored provenance JSON blob (`{"source":"Compiler",…}`) to a tier id.
pub fn provenance_json_to_tier_id(raw: &str) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let source = value.get("source")?.as_str()?;
    let parsed = match source {
        "TreeSitter" => ProvenanceSource::TreeSitter,
        "Heuristic" => ProvenanceSource::Heuristic,
        "Lsp" => ProvenanceSource::Lsp,
        "Compiler" => ProvenanceSource::Compiler,
        "Runtime" => ProvenanceSource::Runtime,
        _ => return None,
    };
    provenance_source_to_tier(parsed).map(EvidenceTier::id)
}

/// MCP / CLI `tier_legend` object. Keep this the only constructor.
pub fn tier_legend_json() -> serde_json::Value {
    serde_json::json!({
        "tier_0": EvidenceTier::Tier0.label(),
        "tier_1": EvidenceTier::Tier1.label(),
        "tier_3": EvidenceTier::Tier3.label(),
        "runtime_verified": EvidenceTier::RuntimeVerified.label(),
        "sources": {
            "tier_0": EvidenceTier::Tier0.sources(),
            "tier_1": EvidenceTier::Tier1.sources(),
            "tier_3": EvidenceTier::Tier3.sources(),
            "runtime_verified": EvidenceTier::RuntimeVerified.sources(),
        },
        "agent_directive": "When you state a structural fact derived from this response, name the tier you're quoting. Quote `confidence_caveats` verbatim. Do not flatten tiers into 'GridSeak says…'.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_is_tier_3() {
        assert_eq!(
            provenance_source_to_tier(ProvenanceSource::Compiler),
            Some(EvidenceTier::Tier3)
        );
        assert_eq!(
            provenance_json_to_tier_id(r#"{"source":"Compiler","confidence":"High"}"#),
            Some("tier_3")
        );
    }

    #[test]
    fn unknown_source_is_none() {
        assert!(provenance_json_to_tier_id(r#"{"source":"Wat"}"#).is_none());
        assert!(provenance_json_to_tier_id("{}").is_none());
    }

    #[test]
    fn legend_names_compiler_and_lsp() {
        let legend = tier_legend_json();
        let t3 = legend["tier_3"].as_str().unwrap();
        assert!(t3.contains("compiler"));
        assert!(t3.contains("SCIP") || t3.contains("language server"));
        assert_eq!(legend["sources"]["tier_3"][0], "Compiler");
    }
}
