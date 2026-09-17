//! Authority ladder and witness-set bitset for provenance fusion.
//!
//! Centralizes the ordering `Compiler >= Lsp > Heuristic > TreeSitter` and
//! provides a `Copy` bitset over the closed `ProvenanceSource` enum for
//! corroboration tracking (plan 02 Phase B).

use super::{Confidence, ProvenanceSource};

/// Numeric authority rank for a provenance source. Higher = more authoritative.
pub fn authority_rank(s: ProvenanceSource) -> u8 {
    match s {
        ProvenanceSource::Runtime => 5,
        ProvenanceSource::Compiler => 4,
        ProvenanceSource::Lsp => 3,
        ProvenanceSource::Heuristic => 2,
        ProvenanceSource::TreeSitter => 1,
    }
}

/// Default confidence for edges stamped with this source when no fusion occurs.
pub fn default_confidence(s: ProvenanceSource) -> Confidence {
    match s {
        ProvenanceSource::Runtime | ProvenanceSource::Compiler | ProvenanceSource::Lsp => {
            Confidence::High
        }
        ProvenanceSource::Heuristic | ProvenanceSource::TreeSitter => Confidence::Low,
    }
}

/// `Copy` bitset over the four `ProvenanceSource` variants.
///
/// Bit layout (stable for serde on `Provenance::corroborating`):
/// - bit 0: TreeSitter
/// - bit 1: Heuristic
/// - bit 2: Lsp
/// - bit 3: Compiler
/// - bit 4: Runtime
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct SourceSet(u8);

impl SourceSet {
    const TREE_SITTER: u8 = 1 << 0;
    const HEURISTIC: u8 = 1 << 1;
    const LSP: u8 = 1 << 2;
    const COMPILER: u8 = 1 << 3;
    const RUNTIME: u8 = 1 << 4;

    fn bit_for(s: ProvenanceSource) -> u8 {
        match s {
            ProvenanceSource::TreeSitter => Self::TREE_SITTER,
            ProvenanceSource::Heuristic => Self::HEURISTIC,
            ProvenanceSource::Lsp => Self::LSP,
            ProvenanceSource::Compiler => Self::COMPILER,
            ProvenanceSource::Runtime => Self::RUNTIME,
        }
    }

    pub fn insert(&mut self, s: ProvenanceSource) {
        self.0 |= Self::bit_for(s);
    }

    pub fn contains(&self, s: ProvenanceSource) -> bool {
        self.0 & Self::bit_for(s) != 0
    }

    pub fn count(&self) -> u32 {
        self.0.count_ones()
    }

    pub fn iter(&self) -> impl Iterator<Item = ProvenanceSource> + '_ {
        [
            ProvenanceSource::TreeSitter,
            ProvenanceSource::Heuristic,
            ProvenanceSource::Lsp,
            ProvenanceSource::Compiler,
            ProvenanceSource::Runtime,
        ]
        .into_iter()
        .filter(|s| self.contains(*s))
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_rank_orders_compiler_above_lsp_above_heuristic_above_tree_sitter() {
        assert!(authority_rank(ProvenanceSource::Compiler) > authority_rank(ProvenanceSource::Lsp));
        assert!(
            authority_rank(ProvenanceSource::Lsp) > authority_rank(ProvenanceSource::Heuristic)
        );
        assert!(
            authority_rank(ProvenanceSource::Heuristic)
                > authority_rank(ProvenanceSource::TreeSitter)
        );
    }

    #[test]
    fn default_confidence_matches_source_tier() {
        assert_eq!(
            default_confidence(ProvenanceSource::Compiler),
            Confidence::High
        );
        assert_eq!(default_confidence(ProvenanceSource::Lsp), Confidence::High);
        assert_eq!(
            default_confidence(ProvenanceSource::Heuristic),
            Confidence::Low
        );
        assert_eq!(
            default_confidence(ProvenanceSource::TreeSitter),
            Confidence::Low
        );
    }

    #[test]
    fn source_set_bit_layout_round_trip() {
        let mut set = SourceSet::default();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);

        set.insert(ProvenanceSource::Compiler);
        set.insert(ProvenanceSource::Heuristic);
        assert!(set.contains(ProvenanceSource::Compiler));
        assert!(set.contains(ProvenanceSource::Heuristic));
        assert!(!set.contains(ProvenanceSource::Lsp));
        assert!(!set.contains(ProvenanceSource::TreeSitter));
        assert_eq!(set.count(), 2);

        let collected: Vec<_> = set.iter().collect();
        assert_eq!(collected.len(), 2);
        assert!(collected.contains(&ProvenanceSource::Compiler));
        assert!(collected.contains(&ProvenanceSource::Heuristic));
    }

    #[test]
    fn source_set_serde_round_trip() {
        let mut set = SourceSet::default();
        set.insert(ProvenanceSource::Lsp);
        let json = serde_json::to_string(&set).unwrap();
        let back: SourceSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, set);
        assert!(back.contains(ProvenanceSource::Lsp));
    }
}
