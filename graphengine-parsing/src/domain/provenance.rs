//! Provenance and confidence tracking for parsed elements
//!
//! Adapted from the old core system's provenance patterns.
//! Tracks the source and confidence level of parsed information.

/// Source of parsed information
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ProvenanceSource {
    /// Syntactic analysis via Tree-sitter
    TreeSitter,
    /// Semantic analysis via Language Server Protocol
    Lsp,
    /// Heuristic analysis or inference
    Heuristic,
    /// Whole-program semantic analysis via an in-process compiler/analyzer
    /// library or a batch compiler-produced index (rust-analyzer as a library,
    /// SCIP). No subprocess LSP.
    Compiler,
    /// Observed at runtime via coverage or call-trace ingest.
    /// Existence-only: an observed edge is ground truth that the call happened;
    /// absence of an observation must not refute a static edge.
    Runtime,
}

/// Confidence level in the accuracy of parsed information
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
pub enum Confidence {
    /// Low confidence - uncertain or inferred
    Low,
    /// Medium confidence - likely correct but some uncertainty
    Medium,
    /// High confidence - direct, unambiguous parsing
    High,
}

/// Provenance information for a parsed element
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Provenance {
    /// Source of the information
    pub source: ProvenanceSource,
    /// Confidence level in the accuracy
    pub confidence: Confidence,
    /// Non-winning witnesses that agreed with the resolved target (plan 02 Phase B).
    #[serde(default)]
    pub corroborating: crate::domain::authority::SourceSet,
}

impl Provenance {
    /// Create a new provenance with the given source and confidence
    pub fn new(source: ProvenanceSource, confidence: Confidence) -> Self {
        Self {
            source,
            confidence,
            corroborating: crate::domain::authority::SourceSet::default(),
        }
    }

    /// Create a high-confidence Tree-sitter provenance
    pub fn tree_sitter() -> Self {
        Self::new(ProvenanceSource::TreeSitter, Confidence::High)
    }

    /// Create a high-confidence LSP provenance
    pub fn lsp() -> Self {
        Self::new(ProvenanceSource::Lsp, Confidence::High)
    }

    /// Create a low-confidence heuristic provenance
    pub fn heuristic() -> Self {
        Self::new(ProvenanceSource::Heuristic, Confidence::Low)
    }

    /// Create a high-confidence compiler/batch-index provenance
    pub fn compiler() -> Self {
        Self::new(ProvenanceSource::Compiler, Confidence::High)
    }

    /// Create a high-confidence runtime-observed provenance
    pub fn runtime() -> Self {
        Self::new(ProvenanceSource::Runtime, Confidence::High)
    }

    /// Validate that the provenance configuration makes sense
    pub fn validate(&self) -> Result<(), String> {
        match (self.source, self.confidence) {
            // LSP should generally be high confidence (semantic analysis)
            (ProvenanceSource::Lsp, Confidence::Low) => {
                Err("LSP source should not have low confidence".to_string())
            }
            // LSP with medium or high confidence is fine
            (ProvenanceSource::Lsp, Confidence::Medium | Confidence::High) => Ok(()),
            // Compiler (batch index / in-process analyzer) same as LSP
            (ProvenanceSource::Compiler, Confidence::Low) => {
                Err("Compiler source should not have low confidence".to_string())
            }
            (ProvenanceSource::Compiler, Confidence::Medium | Confidence::High) => Ok(()),
            // Tree-sitter can be any confidence (syntactic analysis)
            (ProvenanceSource::TreeSitter, _) => Ok(()),
            // Heuristic is typically low confidence
            (ProvenanceSource::Heuristic, _) => Ok(()),
            (ProvenanceSource::Runtime, Confidence::Low) => {
                Err("Runtime source should not have low confidence".to_string())
            }
            (ProvenanceSource::Runtime, Confidence::Medium | Confidence::High) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_provenance_validates_ok() {
        assert!(Provenance::compiler().validate().is_ok());
    }

    #[test]
    fn compiler_low_confidence_is_invalid() {
        let p = Provenance::new(ProvenanceSource::Compiler, Confidence::Low);
        assert!(p.validate().is_err());
    }

    #[test]
    fn compiler_serde_round_trip() {
        let p = Provenance::compiler();
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"Compiler\""));
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
