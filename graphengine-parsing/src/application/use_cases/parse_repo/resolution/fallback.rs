//! Fallback edge creation for unresolved call sites and type usage

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::{
    CallSite, GlobalSymbolTable, ResolvedEdges, SyntaxResults, TypeUsageKind,
};
use super::trait_filter::TraitCandidateFilter;
use crate::domain::{Confidence, Edge, EdgeKind, NodeKind, Provenance, ProvenanceSource};
use std::collections::HashMap;
use tracing::{debug, info};

/// Fallback edge builder for unresolved call sites and type usage
pub struct FallbackEdgeBuilder;

impl FallbackEdgeBuilder {
    /// Create fallback edges for unresolved call sites and type usage
    ///
    /// # Arguments
    /// * `syntax_results` - Syntax extraction results
    /// * `global` - Global symbol table
    /// * `resolved_edges` - Already resolved edges (to avoid duplicates)
    ///
    /// # Returns
    /// * `ResolvedEdges` - Updated edges with fallback edges added
    /// * `ParsingError` - If creation fails
    pub fn create_fallback_edges(
        syntax_results: &SyntaxResults,
        global: &GlobalSymbolTable,
        mut resolved_edges: ResolvedEdges,
    ) -> Result<ResolvedEdges, ParsingError> {
        // Step 1: Create call edges
        Self::create_call_edges(syntax_results, global, &mut resolved_edges)?;

        // Step 2: Create type usage edges
        Self::create_type_usage_edges(syntax_results, global, &mut resolved_edges)?;

        // Step 3: Create identifier usage edges (variables)
        Self::create_identifier_usage_edges(syntax_results, global, &mut resolved_edges)?;

        Ok(resolved_edges)
    }

    /// Create fallback call edges for unresolved call sites
    fn create_call_edges(
        syntax_results: &SyntaxResults,
        global: &GlobalSymbolTable,
        resolved_edges: &mut ResolvedEdges,
    ) -> Result<(), ParsingError> {
        // Build a set of existing call edges to avoid duplicates (caller_id -> callee_id)
        let mut existing = std::collections::HashSet::new();
        for e in &resolved_edges.call_edges {
            existing.insert((e.from_id.clone(), e.to_id.clone()));
        }

        // Fallback: for each unresolved reference, if we can identify
        // a unique target by name, create a low-confidence edge. Edge
        // kind is derived from the reference variant, not hardcoded,
        // so framework / declarative bindings retain their typed edge
        // kind even through the heuristic-fallback path.
        //
        // T6 Gate 1.2 — any call site already resolved by a Layer 2
        // semantic resolver (e.g. `RustLayer2SemanticResolver`) is
        // skipped here. Without this guard, the heuristic's
        // name-match path would emit a `Confidence::Low` sibling
        // edge to a *different* candidate whenever the Layer-2
        // target disagrees with the heuristic's best name-match,
        // silently populating the graph with misleading low-authority
        // edges next to the High-authority truth. See the field-level
        // docs on `ResolvedEdges::resolved_call_sites`.
        for reference in &syntax_results.references {
            let call_site = reference.call_site();
            if resolved_edges
                .resolved_call_sites
                .contains(&call_site.location)
            {
                continue;
            }
            let fallback_edge_kind = reference.edge_kind();
            let caller = Self::find_caller_function(call_site, global);

            if let Some(caller_fn) = caller {
                // Find candidates by simple name match
                let name = call_site
                    .function_name
                    .split(':')
                    .next_back()
                    .unwrap_or(call_site.function_name.as_str());
                let candidates: Vec<_> = global
                    .find_symbols_by_name(name)
                    .into_iter()
                    .filter(|s| s.kind == NodeKind::Function)
                    .collect();

                // Check if caller is a trait method (implementation or default)
                let caller_is_trait_method = caller_fn.trait_metadata.is_some();

                // Check if this is a trait method call
                let is_trait_method_call =
                    caller_is_trait_method && call_site.function_name.starts_with("method_call:");

                // Filter trait candidates
                let filtered_candidates = TraitCandidateFilter::filter_candidates(
                    candidates,
                    &call_site.location.file,
                    caller_is_trait_method,
                    is_trait_method_call,
                );

                // Create edge if unique candidate found
                if let Some(edge) = Self::create_edge_if_unique(
                    caller_fn,
                    &filtered_candidates,
                    &mut existing,
                    fallback_edge_kind,
                ) {
                    resolved_edges.add_call_edge(edge);
                    // Heuristic-fallback call edges are the only
                    // call-edge source when the primary semantic
                    // resolver returns nothing (e.g. Rust Layer 2
                    // against a file outside the project model, or
                    // any non-Apex path without an LSP subprocess).
                    // Without this bump, `total_call_edges()` would
                    // silently underreport the edges we actually
                    // emitted into the graph — which is exactly the
                    // measured-fidelity discipline the sprint is
                    // trying to uphold.
                    resolved_edges.stats.heuristic_edges += 1;
                } else if filtered_candidates.len() > 1 {
                    if call_site.function_name.starts_with("method_call:")
                        || call_site.function_name.starts_with("chained_call:")
                    {
                        // For polymorphic method calls, emit low-confidence edges to all candidates
                        for candidate in filtered_candidates {
                            if caller_fn.id == candidate.id {
                                continue;
                            }
                            let key = (caller_fn.id.clone(), candidate.id.clone());
                            if !existing.contains(&key) {
                                existing.insert(key.clone());
                                let edge = Edge::new(
                                    caller_fn.id.clone(),
                                    candidate.id.clone(),
                                    fallback_edge_kind,
                                    Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
                                );
                                resolved_edges.add_call_edge(edge);
                                resolved_edges.stats.heuristic_edges += 1;
                            }
                        }
                    } else {
                        // Multiple candidates after filtering - log warning
                        debug!(
                            "[CALL_RESOLUTION] Multiple candidates after filtering for call '{}' from '{}': {:?}",
                            call_site.function_name,
                            caller_fn.fqn,
                            filtered_candidates.iter().map(|c| &c.fqn).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Create type usage edges connecting functions/classes to types they reference
    fn create_type_usage_edges(
        syntax_results: &SyntaxResults,
        global: &GlobalSymbolTable,
        resolved_edges: &mut ResolvedEdges,
    ) -> Result<(), ParsingError> {
        // Build type symbol lookup by name
        let type_symbols = Self::build_type_symbol_lookup(global);
        info!(
            "Built type symbol lookup with {} type names",
            type_symbols.len()
        );

        // Track existing type edges to avoid duplicates
        let mut existing_type_edges = std::collections::HashSet::new();
        for e in &resolved_edges.type_edges {
            existing_type_edges.insert((e.from_id.clone(), e.to_id.clone()));
        }

        let mut type_edges_created = 0;

        // Process each type reference
        for type_ref in &syntax_results.type_references {
            // Find the container (function/class) that uses this type
            let container = Self::find_containing_symbol(&type_ref.location, global);

            if let Some(container_symbol) = container {
                // Look up the type definition by name
                if let Some(type_symbols) = type_symbols.get(&type_ref.type_name) {
                    // If unique match, create edge
                    if type_symbols.len() == 1 {
                        let type_symbol = type_symbols[0];
                        let key = (container_symbol.id.clone(), type_symbol.id.clone());

                        // Skip self-references and duplicates
                        if container_symbol.id != type_symbol.id
                            && !existing_type_edges.contains(&key)
                        {
                            existing_type_edges.insert(key);

                            // Sprint E.1: map inheritance relationships to their
                            // dedicated edge kinds. Non-inheritance type references
                            // (parameter types, return types, field types, etc.)
                            // remain `Uses` so analysis that filters on
                            // inheritance-only edges can do so precisely.
                            let edge_kind = match type_ref.usage_kind {
                                TypeUsageKind::Extends => EdgeKind::Extends,
                                TypeUsageKind::Implements => EdgeKind::Implements,
                                _ => EdgeKind::Uses,
                            };

                            let edge = Edge::new(
                                container_symbol.id.clone(),
                                type_symbol.id.clone(),
                                edge_kind,
                                Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium),
                            );
                            resolved_edges.add_type_edge(edge);
                            type_edges_created += 1;

                            debug!(
                                "[TYPE_USAGE] Created {:?} edge: {} --{:?}--> {}",
                                edge_kind,
                                container_symbol.fqn,
                                type_ref.usage_kind,
                                type_symbol.fqn
                            );
                        }
                    } else if type_symbols.len() > 1 {
                        // Multiple candidates - try to pick same-file match
                        let same_file: Vec<_> = type_symbols
                            .iter()
                            .filter(|s| s.file == type_ref.location.file)
                            .collect();

                        if same_file.len() == 1 {
                            let type_symbol = same_file[0];
                            let key = (container_symbol.id.clone(), type_symbol.id.clone());

                            if container_symbol.id != type_symbol.id
                                && !existing_type_edges.contains(&key)
                            {
                                existing_type_edges.insert(key);

                                let edge_kind = match type_ref.usage_kind {
                                    TypeUsageKind::Extends => EdgeKind::Extends,
                                    TypeUsageKind::Implements => EdgeKind::Implements,
                                    _ => EdgeKind::Uses,
                                };

                                let edge = Edge::new(
                                    container_symbol.id.clone(),
                                    type_symbol.id.clone(),
                                    edge_kind,
                                    Provenance::new(
                                        ProvenanceSource::TreeSitter,
                                        Confidence::Medium,
                                    ),
                                );
                                resolved_edges.add_type_edge(edge);
                                type_edges_created += 1;
                            }
                        }
                    }
                }
            }
        }

        info!(
            "Created {} type usage edges from {} type references",
            type_edges_created,
            syntax_results.type_references.len()
        );

        Ok(())
    }

    /// Create identifier usage edges connecting containers to referenced variables
    fn create_identifier_usage_edges(
        syntax_results: &SyntaxResults,
        global: &GlobalSymbolTable,
        resolved_edges: &mut ResolvedEdges,
    ) -> Result<(), ParsingError> {
        if syntax_results.identifier_uses.is_empty() {
            return Ok(());
        }

        let mut existing_edges = std::collections::HashSet::new();
        for e in &resolved_edges.type_edges {
            existing_edges.insert((e.from_id.clone(), e.to_id.clone(), e.kind));
        }

        // Build variable lookup by name
        let mut variables_by_name: HashMap<String, Vec<&crate::application::ports::SymbolInfo>> =
            HashMap::new();
        for symbols in global.symbols_by_name.values() {
            for symbol in symbols {
                if symbol.kind == NodeKind::Variable {
                    variables_by_name
                        .entry(symbol.name.clone())
                        .or_default()
                        .push(symbol);
                }
            }
        }

        let mut edges_created = 0;
        for ident_use in &syntax_results.identifier_uses {
            let container = Self::find_containing_symbol(&ident_use.location, global);
            if container.is_none() {
                continue;
            }
            let container = container.unwrap();
            let candidates = variables_by_name.get(&ident_use.name);
            if let Some(candidates) = candidates {
                // Prefer same-file matches
                let same_file: Vec<_> = candidates
                    .iter()
                    .copied()
                    .filter(|s| s.file == ident_use.location.file)
                    .collect();
                let selected = if same_file.len() == 1 {
                    Some(same_file[0])
                } else if candidates.len() == 1 {
                    Some(candidates[0])
                } else {
                    None
                };

                if let Some(target) = selected {
                    let key = (container.id.clone(), target.id.clone(), EdgeKind::Uses);
                    if container.id != target.id && !existing_edges.contains(&key) {
                        existing_edges.insert(key);
                        let edge = Edge::new(
                            container.id.clone(),
                            target.id.clone(),
                            EdgeKind::Uses,
                            Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
                        );
                        resolved_edges.add_type_edge(edge);
                        edges_created += 1;
                    }
                }
            }
        }

        debug!(
            "[IDENTIFIER_USAGE] Created {} uses edges from {} identifier references",
            edges_created,
            syntax_results.identifier_uses.len()
        );

        Ok(())
    }

    /// Build a lookup table from type name to type symbols (Interface, Type, Enum, Struct)
    fn build_type_symbol_lookup(
        global: &GlobalSymbolTable,
    ) -> HashMap<String, Vec<&crate::application::ports::SymbolInfo>> {
        let mut lookup: HashMap<String, Vec<_>> = HashMap::new();

        for symbols in global.symbols_by_name.values() {
            for symbol in symbols {
                match symbol.kind {
                    NodeKind::Interface | NodeKind::Type | NodeKind::Enum | NodeKind::Struct => {
                        lookup
                            .entry(symbol.name.clone())
                            .or_insert_with(Vec::new)
                            .push(symbol);
                    }
                    _ => {}
                }
            }
        }

        lookup
    }

    /// Find the containing symbol (function, class, interface) for a location
    fn find_containing_symbol<'a>(
        location: &crate::domain::Range,
        global: &'a GlobalSymbolTable,
    ) -> Option<&'a crate::application::ports::SymbolInfo> {
        // First try same-file symbols
        if let Some(symbols_in_file) = global.symbols_by_file.get(&location.file) {
            // Try to find containing function first (most specific)
            if let Some(func) = symbols_in_file
                .iter()
                .find(|s| s.kind == NodeKind::Function && Self::contains(location, &s.range))
            {
                return Some(func);
            }

            // Then try class/struct
            if let Some(class) = symbols_in_file
                .iter()
                .find(|s| s.kind == NodeKind::Struct && Self::contains(location, &s.range))
            {
                return Some(class);
            }

            // Then try interface
            if let Some(interface) = symbols_in_file
                .iter()
                .find(|s| s.kind == NodeKind::Interface && Self::contains(location, &s.range))
            {
                return Some(interface);
            }
        }

        None
    }

    /// Find the caller function containing a call site
    fn find_caller_function<'a>(
        call_site: &CallSite,
        global: &'a GlobalSymbolTable,
    ) -> Option<&'a crate::application::ports::SymbolInfo> {
        let caller_file = call_site.location.file.clone();

        // First try same-file functions (most accurate)
        if let Some(symbols_in_file) = global.symbols_by_file.get(&caller_file) {
            let mut containing: Vec<_> = symbols_in_file
                .iter()
                .filter(|s| {
                    s.kind == NodeKind::Function && Self::contains(&call_site.location, &s.range)
                })
                .collect();
            containing.sort_by_key(|s| Self::range_span(&s.range));
            if let Some(caller) = containing.first() {
                return Some(*caller);
            }
        }

        // Fallback: search ALL symbols globally if not found in same file
        let mut candidates: Vec<_> = global
            .symbols_by_name
            .values()
            .flatten()
            .filter(|s| {
                s.kind == NodeKind::Function && Self::contains(&call_site.location, &s.range)
            })
            .collect();
        candidates.sort_by_key(|s| Self::range_span(&s.range));
        candidates.first().copied()
    }

    /// Range span used to prefer the most specific (smallest) container.
    fn range_span(range: &crate::domain::Range) -> (u32, u32) {
        let line_span = range.end_line.saturating_sub(range.start_line);
        let char_span = if line_span == 0 {
            range.end_char.saturating_sub(range.start_char)
        } else {
            0
        };
        (line_span, char_span)
    }

    /// Check if inner range is contained within outer range
    fn contains(inner: &crate::domain::Range, outer: &crate::domain::Range) -> bool {
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

    /// Create edge if there's a unique candidate
    fn create_edge_if_unique(
        caller_fn: &crate::application::ports::SymbolInfo,
        candidates: &[&crate::application::ports::SymbolInfo],
        existing: &mut std::collections::HashSet<(String, String)>,
        edge_kind: EdgeKind,
    ) -> Option<Edge> {
        if candidates.len() == 1 {
            let callee = candidates[0];
            // Skip self-loops (functions calling themselves - recursion is not represented as edges)
            if caller_fn.id != callee.id {
                let key = (caller_fn.id.clone(), callee.id.clone());
                if !existing.contains(&key) {
                    existing.insert(key.clone());
                    return Some(Edge::new(
                        caller_fn.id.clone(),
                        callee.id.clone(),
                        edge_kind,
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Low),
                    ));
                }
            }
        }
        None
    }
}
