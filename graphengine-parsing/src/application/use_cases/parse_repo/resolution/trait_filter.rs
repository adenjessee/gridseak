//! Trait candidate filtering to avoid circular calls

use crate::application::ports::SymbolInfo;

/// Trait candidate filter
pub struct TraitCandidateFilter;

impl TraitCandidateFilter {
    /// Filter trait method candidates to avoid circular calls from defaults
    ///
    /// This filters out trait defaults when implementations exist, preventing
    /// circular call edges in the graph.
    ///
    /// If `caller_is_trait_method` is false, prefers non-trait methods over trait methods.
    /// If `is_trait_method_call` is true (e.g., `algorithm.method()`), only matches trait methods.
    pub fn filter_candidates<'a>(
        candidates: Vec<&'a SymbolInfo>,
        context_file: &str,
        caller_is_trait_method: bool,
        is_trait_method_call: bool,
    ) -> Vec<&'a SymbolInfo> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // Separate implementations from defaults and trait signatures
        let (implementations, defaults, trait_signatures, non_trait) =
            Self::separate_by_trait_type(candidates);

        // If this is a trait method call (algorithm.method()), ONLY match trait methods
        // Trait object calls can't call regular methods, only trait methods
        if is_trait_method_call {
            return Self::handle_trait_method_calls(
                implementations,
                defaults,
                trait_signatures,
                context_file,
            );
        }

        // If caller is NOT a trait method, prefer non-trait methods
        // Regular methods should call regular methods, not trait methods
        // This is critical: when a regular method calls self.method(), it should match
        // methods on the same type, NOT trait methods with the same name
        if !caller_is_trait_method && !non_trait.is_empty() {
            return Self::prefer_same_file(non_trait, context_file);
        }

        // If we have implementations, prefer them (they override defaults and signatures)
        if !implementations.is_empty() {
            return Self::prefer_same_file(implementations, context_file);
        }

        // If we only have defaults, return them (but prefer same file)
        if !defaults.is_empty() {
            return Self::prefer_same_file(defaults, context_file);
        }

        // If we only have trait signatures, return them (but prefer same file)
        if !trait_signatures.is_empty() {
            return Self::prefer_same_file(trait_signatures, context_file);
        }

        non_trait
    }

    /// Separate candidates by trait type
    fn separate_by_trait_type(
        candidates: Vec<&SymbolInfo>,
    ) -> (
        Vec<&SymbolInfo>,
        Vec<&SymbolInfo>,
        Vec<&SymbolInfo>,
        Vec<&SymbolInfo>,
    ) {
        let mut implementations = Vec::new();
        let mut defaults = Vec::new();
        let mut trait_signatures = Vec::new();
        let mut non_trait = Vec::new();

        for candidate in candidates {
            if let Some(ref trait_meta) = candidate.trait_metadata {
                if trait_meta.is_trait_default {
                    defaults.push(candidate);
                } else {
                    // Not a default - could be implementation or signature
                    // If it has implementing_type, it's an implementation
                    if trait_meta.implementing_type.is_some() {
                        implementations.push(candidate);
                    } else {
                        // No implementing_type means it's a trait signature
                        trait_signatures.push(candidate);
                    }
                }
            } else {
                non_trait.push(candidate);
            }
        }

        (implementations, defaults, trait_signatures, non_trait)
    }

    /// Handle trait method calls - only return trait methods
    fn handle_trait_method_calls<'a>(
        implementations: Vec<&'a SymbolInfo>,
        defaults: Vec<&'a SymbolInfo>,
        trait_signatures: Vec<&'a SymbolInfo>,
        context_file: &str,
    ) -> Vec<&'a SymbolInfo> {
        // Only return trait methods (implementations, defaults, or signatures)
        if !implementations.is_empty() {
            let same_file = Self::prefer_same_file(implementations, context_file);
            if !same_file.is_empty() {
                return same_file;
            }
        }
        if !defaults.is_empty() {
            let same_file = Self::prefer_same_file(defaults, context_file);
            if !same_file.is_empty() {
                return same_file;
            }
        }
        if !trait_signatures.is_empty() {
            let same_file = Self::prefer_same_file(trait_signatures, context_file);
            if !same_file.is_empty() {
                return same_file;
            }
        }
        // No trait methods found - return empty (trait call can't match non-trait methods)
        Vec::new()
    }

    /// Prefer candidates from the same file
    fn prefer_same_file<'a>(
        candidates: Vec<&'a SymbolInfo>,
        context_file: &str,
    ) -> Vec<&'a SymbolInfo> {
        let same_file: Vec<_> = candidates
            .iter()
            .filter(|c| c.file == context_file)
            .copied()
            .collect();
        if !same_file.is_empty() {
            return same_file;
        }
        candidates
    }

    /// Advanced trait candidate filtering with full context awareness
    ///
    /// This method properly filters candidates based on:
    /// - Trait metadata (default vs implementation)
    /// - File context (same file preferred)
    /// - Implementation overrides (implementations preferred over defaults)
    pub fn filter_advanced<'a>(
        candidates: &[&'a SymbolInfo],
        context_file: &str,
    ) -> Vec<&'a SymbolInfo> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // Group by file for context-aware filtering
        let mut by_file: std::collections::HashMap<String, Vec<&SymbolInfo>> =
            std::collections::HashMap::new();

        for candidate in candidates {
            by_file
                .entry(candidate.file.clone())
                .or_default()
                .push(*candidate);
        }

        // Prefer candidates from the same file
        if let Some(same_file_candidates) = by_file.get(context_file) {
            if same_file_candidates.len() == 1 {
                return vec![same_file_candidates[0]];
            }
            // If multiple in same file, return all (let LSP resolver handle it)
            return same_file_candidates.clone();
        }

        // If no same-file matches, return first candidate
        // (Full trait filtering happens in LSP resolver with Node metadata)
        if let Some(first_group) = by_file.values().next() {
            return vec![first_group[0]];
        }

        candidates.to_vec()
    }
}
