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
}

impl Provenance {
    /// Create a new provenance with the given source and confidence
    pub fn new(source: ProvenanceSource, confidence: Confidence) -> Self {
        Self { source, confidence }
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

    /// Validate that the provenance configuration makes sense
    pub fn validate(&self) -> Result<(), String> {
        match (self.source, self.confidence) {
            // LSP should generally be high confidence (semantic analysis)
            (ProvenanceSource::Lsp, Confidence::Low) => {
                Err("LSP source should not have low confidence".to_string())
            }
            // LSP with medium or high confidence is fine
            (ProvenanceSource::Lsp, Confidence::Medium | Confidence::High) => Ok(()),
            // Tree-sitter can be any confidence (syntactic analysis)
            (ProvenanceSource::TreeSitter, _) => Ok(()),
            // Heuristic is typically low confidence
            (ProvenanceSource::Heuristic, _) => Ok(()),
        }
    }
}
