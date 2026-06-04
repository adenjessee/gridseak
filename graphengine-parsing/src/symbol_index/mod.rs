//! Symbol indexing utilities.
//!
//! Provides a language-agnostic index over extracted symbols so that
//! higher-level logic (call resolution, heuristics, etc.) can disambiguate
//! between multiple potential targets in a deterministic way.
//!
//! ## Index structures
//!
//! - `by_fqn` — exact FQN → record index (O(1) lookup)
//! - `by_simple` — simple name → record indices (O(1) lookup)
//! - `by_id` — symbol ID → record index (O(1) lookup)
//! - `fqn_suffixes` — FQN suffix segments → record indices (O(1) suffix lookup)
//! - `functions_by_file` — file path → sorted function indices (O(file_fns) containment)
//! - `modules_by_file` — file path → module indices (O(file_mods) containment)
//! - `all_symbols_by_file` — file path → symbol indices of all kinds (O(file_syms) lookup)

use std::collections::HashMap;

use crate::application::ports::SyntaxResults;
use crate::domain::{NodeKind, Range};

use crate::module_resolution::ModuleResolver;

mod heuristic_scope;
use heuristic_scope::{filter_heuristic_callees, is_non_production_heuristic_context};

#[derive(Debug, Clone)]
pub struct SymbolRecord {
    pub id: String,
    pub fqn: String,
    pub simple_name: String,
    pub file: String,
    pub range: Range,
    pub kind: NodeKind,
    pub trait_metadata: Option<crate::domain::TraitMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionStrategy {
    DirectFqn,
    FqnSuffix,
    SimpleName,
    DefinitionLocation,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedSymbol<'a> {
    pub record: &'a SymbolRecord,
    pub strategy: ResolutionStrategy,
}

#[derive(Debug, Default, Clone)]
pub struct SymbolIndex {
    records: Vec<SymbolRecord>,

    // --- Primary indexes (O(1) lookup) ---
    by_fqn: HashMap<String, usize>,
    by_simple: HashMap<String, Vec<usize>>,
    by_id: HashMap<String, usize>,

    // --- Suffix index for FQN suffix matching ---
    fqn_suffixes: HashMap<String, Vec<usize>>,

    // --- Spatial indexes (O(file_fns) containment) ---
    functions_by_file: HashMap<String, Vec<usize>>,
    modules_by_file: HashMap<String, Vec<usize>>,
    all_symbols_by_file: HashMap<String, Vec<usize>>,
}

impl SymbolIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_syntax(syntax_results: &SyntaxResults) -> Self {
        let mut index = SymbolIndex::default();

        for symbol in &syntax_results.symbols {
            let parts: Vec<&str> = symbol.fqn.split("::").collect();
            let simple_name = parts
                .last()
                .map(|segment| (*segment).to_string())
                .unwrap_or_else(|| symbol.fqn.clone());

            let record = SymbolRecord {
                id: symbol.id.clone(),
                fqn: symbol.fqn.clone(),
                simple_name: simple_name.clone(),
                file: symbol.location.file.clone(),
                range: symbol.location.clone(),
                kind: symbol.kind,
                trait_metadata: symbol.trait_metadata.clone(),
            };

            let idx = index.records.len();

            // ID index — all symbol kinds
            index.by_id.insert(record.id.clone(), idx);

            // Spatial indexes — all symbol kinds, grouped by file
            index
                .all_symbols_by_file
                .entry(record.file.clone())
                .or_default()
                .push(idx);

            if symbol.kind == NodeKind::Function {
                // Function-specific indexes
                index.by_fqn.insert(record.fqn.clone(), idx);
                index
                    .by_simple
                    .entry(simple_name.clone())
                    .or_default()
                    .push(idx);

                index
                    .functions_by_file
                    .entry(record.file.clone())
                    .or_default()
                    .push(idx);

                // FQN suffix index: store suffixes of 2+ segments
                // e.g. "crate::alpha::beta::foo" → ["beta::foo", "alpha::beta::foo"]
                if parts.len() >= 2 {
                    for window_start in 1..parts.len() {
                        let suffix = parts[window_start..].join("::");
                        index.fqn_suffixes.entry(suffix).or_default().push(idx);
                    }
                }

                // Constructor indexing: "Type::new" also indexed under "Type"
                if let Some(last) = parts.last() {
                    if *last == "new" && parts.len() >= 2 {
                        let type_fqn = parts[..parts.len() - 1].join("::");
                        index.by_fqn.insert(type_fqn.clone(), idx);

                        let type_simple = parts[parts.len() - 2];
                        index
                            .by_simple
                            .entry(type_simple.to_string())
                            .or_default()
                            .push(idx);
                    }
                }
            } else if symbol.kind == NodeKind::Module {
                index
                    .modules_by_file
                    .entry(record.file.clone())
                    .or_default()
                    .push(idx);
            }

            index.records.push(record);
        }

        index
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    // ---------------------------------------------------------------
    // O(1) lookups
    // ---------------------------------------------------------------

    /// Look up a symbol record by its unique ID. O(1).
    pub fn get_by_id(&self, id: &str) -> Option<&SymbolRecord> {
        self.by_id.get(id).and_then(|&idx| self.records.get(idx))
    }

    // ---------------------------------------------------------------
    // Containment queries (O(functions_in_file) per lookup)
    // ---------------------------------------------------------------

    /// Find the innermost function containing the given range.
    ///
    /// Scans only the functions in the same file (typically 10–100),
    /// not all 12K+ symbols. Prefers same-file, smallest span.
    pub fn find_containing_function(&self, location: &Range) -> Option<&SymbolRecord> {
        let file_fns = self.functions_by_file.get(&location.file)?;

        let mut best: Option<&SymbolRecord> = None;
        let mut best_span: u32 = u32::MAX;

        for &idx in file_fns {
            if let Some(record) = self.records.get(idx) {
                if is_within_range(location, &record.range) {
                    let span = record
                        .range
                        .end_line
                        .saturating_sub(record.range.start_line);
                    if span < best_span {
                        best_span = span;
                        best = Some(record);
                    }
                }
            }
        }

        best
    }

    /// Find the module containing the given range.
    ///
    /// Scans only modules in the same file. Prefers smallest enclosing module,
    /// falls back to file-level module.
    pub fn find_containing_module(&self, location: &Range) -> Option<&SymbolRecord> {
        let file_mods = self.modules_by_file.get(&location.file)?;

        let mut best: Option<&SymbolRecord> = None;
        let mut best_span: u32 = u32::MAX;

        for &idx in file_mods {
            if let Some(record) = self.records.get(idx) {
                if is_within_range(location, &record.range) {
                    let span = record
                        .range
                        .end_line
                        .saturating_sub(record.range.start_line);
                    if span < best_span {
                        best_span = span;
                        best = Some(record);
                    }
                }
            }
        }

        // Fallback: if no module contains the range, find any module in this file
        if best.is_none() {
            for &idx in file_mods {
                if let Some(record) = self.records.get(idx) {
                    return Some(record);
                }
            }
        }

        best
    }

    /// Find the most specific symbol at a given location (smallest enclosing span).
    ///
    /// Scans only symbols in the same file. Used by import/type resolution.
    pub fn find_symbol_at_location(&self, location: &Range) -> Option<&SymbolRecord> {
        let file_syms = self.all_symbols_by_file.get(&location.file)?;

        let mut best: Option<&SymbolRecord> = None;
        let mut best_span: u32 = u32::MAX;

        for &idx in file_syms {
            if let Some(record) = self.records.get(idx) {
                if is_within_range(location, &record.range) {
                    let span = record
                        .range
                        .end_line
                        .saturating_sub(record.range.start_line);
                    if span < best_span {
                        best_span = span;
                        best = Some(record);
                    }
                }
            }
        }

        best
    }

    // ---------------------------------------------------------------
    // Resolution: FQN, suffix, simple name
    // ---------------------------------------------------------------

    pub fn resolve_function<'a>(
        &'a self,
        raw_target: &str,
        context_file: &str,
        module_resolver: &ModuleResolver,
    ) -> Option<ResolvedSymbol<'a>> {
        let candidates =
            self.resolve_function_candidates(raw_target, context_file, module_resolver);
        candidates.first().cloned()
    }

    pub fn resolve_function_candidates<'a>(
        &'a self,
        raw_target: &str,
        context_file: &str,
        module_resolver: &ModuleResolver,
    ) -> Vec<ResolvedSymbol<'a>> {
        let target = normalize_target_name(raw_target);
        let mut candidates = Vec::new();

        // Try FQN matches via module resolver
        let fqn_candidates = module_resolver.resolve_name_in_context(context_file, &target);
        for resolved in fqn_candidates {
            if let Some(&idx) = self.by_fqn.get(&resolved.fqn) {
                if let Some(record) = self.records.get(idx) {
                    candidates.push(ResolvedSymbol {
                        record,
                        strategy: ResolutionStrategy::DirectFqn,
                    });
                }
            }
        }

        // Direct FQN lookup
        if let Some(&idx) = self.by_fqn.get(&target) {
            if let Some(record) = self.records.get(idx) {
                candidates.push(ResolvedSymbol {
                    record,
                    strategy: ResolutionStrategy::DirectFqn,
                });
            }
        }

        // FQN suffix matching — now O(1) via suffix index
        if let Some(candidate) = self.match_by_fqn_suffix(&target, context_file, module_resolver) {
            candidates.push(ResolvedSymbol {
                record: candidate,
                strategy: ResolutionStrategy::FqnSuffix,
            });
        }

        // Simple-name matching
        let simple = target.split("::").last().unwrap_or(&target);
        if let Some(candidate) = self.match_by_simple_name(simple, context_file, module_resolver) {
            candidates.push(ResolvedSymbol {
                record: candidate,
                strategy: ResolutionStrategy::SimpleName,
            });
        }

        // Q3-followon: production callers must not heuristic-resolve into
        // test / test-support / fixture callees (SimpleName + FqnSuffix).
        let heuristic_only: Vec<ResolvedSymbol<'a>> = candidates
            .iter()
            .filter(|c| {
                matches!(
                    c.strategy,
                    ResolutionStrategy::SimpleName | ResolutionStrategy::FqnSuffix
                )
            })
            .cloned()
            .collect();
        let filtered_heuristic =
            filter_heuristic_callees(context_file, heuristic_only, |r: &ResolvedSymbol<'_>| {
                r.record.file.as_str()
            });
        if filtered_heuristic.is_empty() && !candidates.is_empty() {
            // Drop heuristic strategies; keep DirectFqn / module-resolver hits.
            candidates.retain(|c| {
                !matches!(
                    c.strategy,
                    ResolutionStrategy::SimpleName | ResolutionStrategy::FqnSuffix
                )
            });
        } else {
            candidates.retain(|c| {
                !matches!(
                    c.strategy,
                    ResolutionStrategy::SimpleName | ResolutionStrategy::FqnSuffix
                ) || filtered_heuristic
                    .iter()
                    .any(|f| f.record.id == c.record.id)
            });
        }

        candidates
    }

    pub fn resolve_by_location<'a>(
        &'a self,
        file: &str,
        location: &Range,
    ) -> Option<ResolvedSymbol<'a>> {
        self.resolve_by_location_all(file, location)
            .into_iter()
            .next()
    }

    pub fn resolve_by_location_all<'a>(
        &'a self,
        file: &str,
        location: &Range,
    ) -> Vec<ResolvedSymbol<'a>> {
        let file_syms = match self.all_symbols_by_file.get(file) {
            Some(syms) => syms,
            None => return Vec::new(),
        };

        let mut candidates: Vec<&SymbolRecord> = file_syms
            .iter()
            .filter_map(|&idx| self.records.get(idx))
            .filter(|record| contains_location(&record.range, location))
            .collect();

        if candidates.is_empty() {
            return Vec::new();
        }

        candidates.sort_by_key(|record| function_span(&record.range));

        candidates
            .into_iter()
            .map(|record| ResolvedSymbol {
                record,
                strategy: ResolutionStrategy::DefinitionLocation,
            })
            .collect()
    }

    /// FQN suffix matching using the prebuilt suffix index. O(1) lookup + O(candidates) filtering.
    fn match_by_fqn_suffix<'a>(
        &'a self,
        target: &str,
        context_file: &str,
        module_resolver: &ModuleResolver,
    ) -> Option<&'a SymbolRecord> {
        let clean_target = target.trim_start_matches(':');

        // Look up directly in the suffix index
        if let Some(indices) = self.fqn_suffixes.get(clean_target) {
            let mut candidates: Vec<&SymbolRecord> = indices
                .iter()
                .filter_map(|&idx| self.records.get(idx))
                .collect();

            if candidates.is_empty() {
                return None;
            }

            let affiliated: Vec<&SymbolRecord> = candidates
                .iter()
                .filter(|rec| {
                    rec.file == context_file
                        || module_resolver.shared_prefix_len_between(context_file, &rec.file) > 0
                })
                .copied()
                .collect();

            if !affiliated.is_empty() {
                let mut affiliated = affiliated;
                return choose_best_match(&mut affiliated, context_file, module_resolver);
            }

            return choose_best_match(&mut candidates, context_file, module_resolver);
        }

        // Fallback: also check exact FQN match (for targets that equal the full FQN)
        if let Some(&idx) = self.by_fqn.get(clean_target) {
            return self.records.get(idx);
        }

        None
    }

    fn match_by_simple_name<'a>(
        &'a self,
        simple: &str,
        context_file: &str,
        module_resolver: &ModuleResolver,
    ) -> Option<&'a SymbolRecord> {
        let indexes = self.by_simple.get(simple)?;
        let mut candidates: Vec<&SymbolRecord> = indexes
            .iter()
            .filter_map(|idx| self.records.get(*idx))
            .collect();
        if candidates.is_empty() {
            return None;
        }

        let affiliated: Vec<&SymbolRecord> = candidates
            .iter()
            .filter(|rec| {
                rec.file == context_file
                    || module_resolver.shared_prefix_len_between(context_file, &rec.file) > 0
            })
            .copied()
            .collect();

        if !affiliated.is_empty() {
            let mut affiliated = affiliated;
            return choose_best_match(&mut affiliated, context_file, module_resolver);
        }

        choose_best_match(&mut candidates, context_file, module_resolver)
    }
}

fn choose_best_match<'a>(
    candidates: &mut Vec<&'a SymbolRecord>,
    context_file: &str,
    module_resolver: &ModuleResolver,
) -> Option<&'a SymbolRecord> {
    if candidates.is_empty() {
        return None;
    }

    // Q3-followon: strip test-support / fixture targets when the caller
    // is production code (extends the older test-file-only filter).
    if !is_non_production_heuristic_context(context_file) {
        let scoped =
            filter_heuristic_callees(context_file, candidates.clone(), |r| r.file.as_str());
        if !scoped.is_empty() {
            *candidates = scoped;
        } else {
            return None;
        }
    }

    if candidates.len() == 1 {
        return candidates.pop();
    }

    if let Some(position) = candidates.iter().position(|rec| rec.file == context_file) {
        return candidates.get(position).copied();
    }

    let mut best_score = 0usize;
    let mut best_index = None;

    for (idx, candidate) in candidates.iter().enumerate() {
        let score = module_resolver.shared_prefix_len_between(context_file, &candidate.file);
        if score > best_score {
            best_score = score;
            best_index = Some(idx);
        }
    }

    if let Some(idx) = best_index {
        return candidates.get(idx).copied();
    }

    candidates.iter().min_by_key(|rec| rec.fqn.len()).copied()
}

/// Check if `inner` is geometrically contained within `outer`.
fn is_within_range(inner: &Range, outer: &Range) -> bool {
    if inner.file != outer.file {
        return false;
    }

    if inner.start_line < outer.start_line || inner.end_line > outer.end_line {
        return false;
    }

    if inner.start_line == outer.start_line && inner.start_char < outer.start_char {
        return false;
    }

    if inner.end_line == outer.end_line && inner.end_char > outer.end_char {
        return false;
    }

    true
}

fn normalize_target_name(name: &str) -> String {
    let mut trimmed = name.trim();

    if let Some(paren_pos) = trimmed.find('(') {
        trimmed = &trimmed[..paren_pos];
    }

    trimmed = trimmed.trim();

    if let Some(generic_pos) = trimmed.find('<') {
        trimmed = &trimmed[..generic_pos];
    }

    trimmed
        .trim()
        .trim_end_matches(':')
        .trim_end_matches(':')
        .to_string()
}

fn contains_location(symbol_range: &Range, location: &Range) -> bool {
    if location.start_line < symbol_range.start_line || location.start_line > symbol_range.end_line
    {
        return false;
    }

    if location.start_line == symbol_range.start_line
        && location.start_char < symbol_range.start_char
    {
        return false;
    }

    if location.start_line == symbol_range.end_line && location.start_char > symbol_range.end_char {
        return false;
    }

    true
}

fn function_span(range: &Range) -> u32 {
    range.end_line.saturating_sub(range.start_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::SyntaxResults;
    use crate::domain::{Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range};

    fn make_function(fqn: &str, file: &str) -> Node {
        Node {
            id: format!("{}_id", fqn),
            kind: NodeKind::Function,
            fqn: fqn.to_string(),
            location: Range::with_file(1, 0, 5, 10, file.to_string()),
            provenance: Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
            properties: std::collections::HashMap::new(),
            trait_metadata: None,
        }
    }

    #[test]
    fn production_caller_does_not_resolve_simple_name_to_test_support() {
        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(make_function(
            "mock_lsp_session::clone",
            "graphengine-parsing-test-support/src/mock_lsp_session.rs",
        ));
        syntax.symbols.push(make_function(
            "graphengine_parsing::resolver::resolve",
            "graphengine-parsing/src/infrastructure/lsp/resolver.rs",
        ));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index.resolve_function(
            "clone",
            "graphengine-parsing/src/infrastructure/lsp/resolver.rs",
            &module_resolver,
        );
        assert!(
            resolved.is_none() || !resolved.unwrap().record.file.contains("test-support"),
            "production caller must not heuristic-resolve into test-support mocks"
        );
    }

    #[test]
    fn prefers_same_file_match() {
        let mut syntax = SyntaxResults::new();
        syntax
            .symbols
            .push(make_function("crate::a::foo", "src/a.rs"));
        syntax
            .symbols
            .push(make_function("crate::b::foo", "src/b.rs"));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index
            .resolve_function("foo", "src/a.rs", &module_resolver)
            .unwrap();
        assert_eq!(resolved.record.file, "src/a.rs");
        assert!(matches!(
            resolved.strategy,
            ResolutionStrategy::DirectFqn
                | ResolutionStrategy::SimpleName
                | ResolutionStrategy::FqnSuffix
        ));
    }

    #[test]
    fn falls_back_to_module_similarity() {
        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(make_function(
            "crate::alpha::beta::foo",
            "src/alpha/beta.rs",
        ));
        syntax.symbols.push(make_function(
            "crate::alpha::gamma::foo",
            "src/alpha/gamma.rs",
        ));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index.resolve_function("foo", "src/alpha/gamma/utils.rs", &module_resolver);
        assert!(resolved.is_some());
    }

    #[test]
    fn resolves_by_location_prefers_smallest_span() {
        let mut syntax = SyntaxResults::new();
        let mut outer = make_function("crate::module::outer", "src/module.rs");
        outer.location = Range::with_file(1, 0, 50, 0, "src/module.rs".to_string());
        let mut inner = make_function("crate::module::inner", "src/module.rs");
        inner.location = Range::with_file(10, 4, 20, 0, "src/module.rs".to_string());

        syntax.symbols.push(outer);
        syntax.symbols.push(inner);

        let index = SymbolIndex::from_syntax(&syntax);
        let lookup = Range::with_file(12, 8, 12, 12, "src/module.rs".to_string());

        let resolved = index
            .resolve_by_location("src/module.rs", &lookup)
            .expect("should resolve location");

        assert_eq!(resolved.record.fqn, "crate::module::inner");
        assert_eq!(resolved.strategy, ResolutionStrategy::DefinitionLocation);
    }

    #[test]
    fn matches_fqn_suffix() {
        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(make_function(
            "crate::alpha::beta::foo",
            "src/alpha/beta.rs",
        ));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index
            .resolve_function("alpha::beta::foo", "src/other.rs", &module_resolver)
            .unwrap();
        assert_eq!(resolved.record.fqn, "crate::alpha::beta::foo");
        assert_eq!(resolved.strategy, ResolutionStrategy::FqnSuffix);
    }

    #[test]
    fn normalizes_constructor_targets() {
        let mut syntax = SyntaxResults::new();
        syntax
            .symbols
            .push(make_function("crate::widget::Widget::new", "src/widget.rs"));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index
            .resolve_function(
                "crate::widget::Widget::new()",
                "src/widget.rs",
                &module_resolver,
            )
            .unwrap();
        assert_eq!(resolved.record.fqn, "crate::widget::Widget::new");

        let resolved_type = index
            .resolve_function("crate::widget::Widget", "src/widget.rs", &module_resolver)
            .unwrap();
        assert_eq!(resolved_type.record.fqn, "crate::widget::Widget::new");
    }

    #[test]
    fn strips_generics_and_whitespace() {
        let mut syntax = SyntaxResults::new();
        syntax
            .symbols
            .push(make_function("crate::alpha::foo", "src/alpha.rs"));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index
            .resolve_function(
                "  crate::alpha::foo::<T>(  ) ",
                "src/alpha.rs",
                &module_resolver,
            )
            .unwrap();
        assert_eq!(resolved.record.fqn, "crate::alpha::foo");
        assert_eq!(resolved.strategy, ResolutionStrategy::DirectFqn);
    }

    #[test]
    fn returns_none_when_symbol_missing() {
        let mut syntax = SyntaxResults::new();
        syntax
            .symbols
            .push(make_function("crate::a::foo", "src/a.rs"));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        let resolved = index.resolve_function("missing", "src/a.rs", &module_resolver);
        assert!(resolved.is_none());
    }

    #[test]
    fn find_containing_function_prefers_smallest() {
        let mut syntax = SyntaxResults::new();
        let mut outer = make_function("crate::outer", "src/file.rs");
        outer.location = Range::with_file(1, 0, 100, 0, "src/file.rs".to_string());
        let mut inner = make_function("crate::inner", "src/file.rs");
        inner.location = Range::with_file(10, 0, 20, 0, "src/file.rs".to_string());

        syntax.symbols.push(outer);
        syntax.symbols.push(inner);

        let index = SymbolIndex::from_syntax(&syntax);
        let call_loc = Range::with_file(15, 4, 15, 20, "src/file.rs".to_string());

        let found = index.find_containing_function(&call_loc).unwrap();
        assert_eq!(found.fqn, "crate::inner");
    }

    #[test]
    fn get_by_id_works() {
        let mut syntax = SyntaxResults::new();
        syntax
            .symbols
            .push(make_function("crate::a::foo", "src/a.rs"));

        let index = SymbolIndex::from_syntax(&syntax);

        let found = index.get_by_id("crate::a::foo_id").unwrap();
        assert_eq!(found.fqn, "crate::a::foo");
        assert!(index.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn fqn_suffix_index_works() {
        let mut syntax = SyntaxResults::new();
        syntax.symbols.push(make_function(
            "crate::alpha::beta::foo",
            "src/alpha/beta.rs",
        ));

        let index = SymbolIndex::from_syntax(&syntax);
        let module_resolver = ModuleResolver::from_syntax(&syntax);

        // "beta::foo" is a suffix of "crate::alpha::beta::foo"
        let resolved = index
            .resolve_function("beta::foo", "src/other.rs", &module_resolver)
            .unwrap();
        assert_eq!(resolved.record.fqn, "crate::alpha::beta::foo");
        assert_eq!(resolved.strategy, ResolutionStrategy::FqnSuffix);
    }
}
