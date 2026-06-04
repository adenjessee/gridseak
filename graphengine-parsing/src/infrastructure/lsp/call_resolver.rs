//! Call resolution service for analyzing function calls
//!
//! Resolves function call sites to their actual targets using the prebuilt
//! `SymbolIndex` for O(1) lookups instead of linear scans.

use crate::application::ports::{CallSite, SyntaxResults, UnresolvedReference};
use crate::domain::{Confidence, Edge, EdgeKind, Provenance, ProvenanceSource, Range};
use crate::module_resolution::ModuleResolver;
use crate::symbol_index::{ResolutionStrategy, ResolvedSymbol, SymbolIndex};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

/// Service for resolving function calls from syntax analysis.
#[derive(Clone)]
pub struct CallResolver {
    symbol_index: SymbolIndex,
    receiver_detector:
        Option<Arc<crate::infrastructure::lsp::receiver_detector::ReceiverTypeDetector>>,
}

impl Default for CallResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CallResolver {
    pub fn new() -> Self {
        Self {
            symbol_index: SymbolIndex::empty(),
            receiver_detector: None,
        }
    }

    pub fn with_receiver_detector(
        receiver_detector: Arc<crate::infrastructure::lsp::receiver_detector::ReceiverTypeDetector>,
    ) -> Self {
        Self {
            symbol_index: SymbolIndex::empty(),
            receiver_detector: Some(receiver_detector),
        }
    }

    /// Build all indexes from syntax results. Must be called before `resolve_calls`.
    pub fn prepare(&mut self, syntax_results: &SyntaxResults) {
        self.symbol_index = SymbolIndex::from_syntax(syntax_results);
    }

    /// Resolve call sites to actual function calls.
    ///
    /// Uses the prebuilt `SymbolIndex` for O(1)/O(file_fns) lookups instead
    /// of scanning all 12K+ symbols per call site.
    pub fn resolve_calls(
        &self,
        syntax_results: &SyntaxResults,
        module_resolver: &ModuleResolver,
    ) -> Result<Vec<Edge>> {
        self.resolve_references(&syntax_results.references, module_resolver)
    }

    /// Resolve an explicit slice of [`UnresolvedReference`]s (avoids cloning
    /// `SyntaxResults` when the caller only wants to process a subset, e.g.
    /// unresolved sites).
    ///
    /// The resolver honours the edge kind declared by each variant — a
    /// `FrameworkBinding(_)` that resolves via the heuristic fallback still
    /// emits a `Framework(_)` edge rather than collapsing to `Call`, which
    /// is the dissolving-element of the P1.d rework: the variant is the
    /// typed channel, not a hint the resolver was free to ignore.
    pub fn resolve_references(
        &self,
        references: &[UnresolvedReference],
        module_resolver: &ModuleResolver,
    ) -> Result<Vec<Edge>> {
        let mut edges = Vec::new();
        let mut seen_edges = HashSet::new();
        let mut telemetry = ResolutionTelemetry::default();
        let mut file_cache: HashMap<String, Option<String>> = HashMap::new();

        for reference in references {
            let call_site = reference.call_site();
            let edge_kind = reference.edge_kind();
            if let Some(edge) = self.resolve_single_call(
                call_site,
                edge_kind,
                module_resolver,
                &mut telemetry,
                &mut file_cache,
            )? {
                let edge_key = format!("{}:{}:{:?}", edge.from_id, edge.to_id, edge.kind);
                if seen_edges.insert(edge_key) {
                    edges.push(edge);
                }
            }
        }

        Ok(edges)
    }

    /// Resolve a single call site to a function call edge.
    ///
    /// All lookups use the prebuilt index:
    /// - `find_containing_function` → O(functions_in_file) via `functions_by_file`
    /// - caller metadata → O(1) via `by_id`
    /// - target resolution → O(1) via `by_fqn` / `by_simple` / `fqn_suffixes`
    pub(crate) fn resolve_single_call(
        &self,
        call_site: &CallSite,
        edge_kind: EdgeKind,
        module_resolver: &ModuleResolver,
        telemetry: &mut ResolutionTelemetry,
        file_cache: &mut HashMap<String, Option<String>>,
    ) -> Result<Option<Edge>> {
        // O(functions_in_file) via spatial index — not O(all_symbols)
        let containing = self
            .symbol_index
            .find_containing_function(&call_site.location);
        let (call_type, target_name) = parse_call_site_name(&call_site.function_name);

        // O(1) via by_id index — not O(all_symbols)
        let (caller_is_trait_method, caller_fqn) = containing
            .map(|rec| {
                let is_trait = rec.trait_metadata.is_some();
                (is_trait, rec.fqn.as_str())
            })
            .unwrap_or((false, ""));

        let caller_id = match containing {
            Some(rec) => rec.id.clone(),
            None => return Ok(None),
        };

        if let Some((callee_id, strategy)) = self.resolve_call_target(
            &call_type,
            &target_name,
            call_site,
            module_resolver,
            telemetry,
            caller_is_trait_method,
            if caller_fqn.is_empty() {
                None
            } else {
                Some(caller_fqn)
            },
            file_cache,
        ) {
            if caller_id != callee_id {
                let confidence = match strategy {
                    ResolutionStrategy::DirectFqn => Confidence::High,
                    ResolutionStrategy::FqnSuffix => Confidence::Medium,
                    ResolutionStrategy::SimpleName => Confidence::Low,
                    ResolutionStrategy::DefinitionLocation => Confidence::High,
                };
                let edge = Edge::new(
                    caller_id,
                    callee_id,
                    edge_kind,
                    Provenance::new(ProvenanceSource::Heuristic, confidence),
                );
                return Ok(Some(edge));
            }
        } else {
            telemetry.unresolved += 1;
        }

        Ok(None)
    }

    /// Resolve call target using different strategies based on call type.
    #[allow(clippy::too_many_arguments)]
    fn resolve_call_target(
        &self,
        call_type: &str,
        target_name: &str,
        call_site: &CallSite,
        module_resolver: &ModuleResolver,
        telemetry: &mut ResolutionTelemetry,
        caller_is_trait_method: bool,
        caller_fqn: Option<&str>,
        file_cache: &mut HashMap<String, Option<String>>,
    ) -> Option<(String, ResolutionStrategy)> {
        let context_file = &call_site.location.file;

        let is_trait_method_call = if call_type == "method_call" {
            if let Some(ref receiver_range) = call_site.receiver_range {
                if let Some(ref detector) = self.receiver_detector {
                    match detect_trait_object_via_lsp(detector, call_site, receiver_range) {
                        Some(true) => true,
                        Some(false) => false,
                        None => detect_trait_object_via_ast(receiver_range, file_cache)
                            .unwrap_or(caller_is_trait_method),
                    }
                } else {
                    detect_trait_object_via_ast(receiver_range, file_cache)
                        .unwrap_or(caller_is_trait_method)
                }
            } else {
                caller_is_trait_method
            }
        } else {
            false
        };

        if call_type == "method_call" && is_ambiguous_builtin_method(target_name) {
            let fqn_candidates = module_resolver.resolve_name_in_context(context_file, target_name);
            for resolved in fqn_candidates {
                if let Some(sym) =
                    self.symbol_index
                        .resolve_function(&resolved.fqn, context_file, module_resolver)
                {
                    telemetry.record(sym.strategy);
                    return Some((sym.record.id.clone(), sym.strategy));
                }
            }
            return None;
        }

        match call_type {
            "constructor_call" => {
                if let Some((id, strategy)) = self.resolve_function_via_index(
                    target_name,
                    context_file,
                    module_resolver,
                    caller_is_trait_method,
                    false,
                    caller_fqn,
                ) {
                    telemetry.record(strategy);
                    return Some((id, strategy));
                }

                if target_name.ends_with("::new") {
                    let type_name = target_name.trim_end_matches("::new");
                    if let Some((id, strategy)) = self.resolve_function_via_index(
                        type_name,
                        context_file,
                        module_resolver,
                        caller_is_trait_method,
                        false,
                        caller_fqn,
                    ) {
                        telemetry.record_constructor(strategy);
                        return Some((id, strategy));
                    }

                    if let Some(simple_type) = type_name.split("::").last() {
                        let fallback = format!("{}::new", simple_type);
                        if let Some((id, strategy)) = self.resolve_function_via_index(
                            &fallback,
                            context_file,
                            module_resolver,
                            caller_is_trait_method,
                            false,
                            caller_fqn,
                        ) {
                            telemetry.record_constructor(strategy);
                            return Some((id, strategy));
                        }
                    }
                }

                None
            }
            _ => {
                if let Some((id, strategy)) = self.resolve_function_via_index(
                    target_name,
                    context_file,
                    module_resolver,
                    caller_is_trait_method,
                    is_trait_method_call,
                    caller_fqn,
                ) {
                    telemetry.record(strategy);
                    Some((id, strategy))
                } else {
                    None
                }
            }
        }
    }

    fn resolve_function_via_index(
        &self,
        target_name: &str,
        context_file: &str,
        module_resolver: &ModuleResolver,
        caller_is_trait_method: bool,
        is_trait_method_call: bool,
        caller_fqn: Option<&str>,
    ) -> Option<(String, ResolutionStrategy)> {
        let candidates = self.symbol_index.resolve_function_candidates(
            target_name,
            context_file,
            module_resolver,
        );

        if candidates.is_empty() {
            return None;
        }

        let filtered = filter_trait_candidates(
            candidates,
            context_file,
            caller_is_trait_method,
            is_trait_method_call,
            caller_fqn,
        );

        filtered
            .first()
            .map(|resolved| (resolved.record.id.clone(), resolved.strategy))
    }
}

// ---------------------------------------------------------------
// Trait candidate filtering (pure function, no self)
// ---------------------------------------------------------------

fn filter_trait_candidates<'a>(
    candidates: Vec<ResolvedSymbol<'a>>,
    context_file: &str,
    caller_is_trait_method: bool,
    is_trait_method_call: bool,
    caller_fqn: Option<&str>,
) -> Vec<ResolvedSymbol<'a>> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut implementations = Vec::new();
    let mut defaults = Vec::new();
    let mut trait_signatures = Vec::new();
    let mut non_trait = Vec::new();

    for candidate in candidates {
        if let Some(ref trait_meta) = candidate.record.trait_metadata {
            if trait_meta.is_trait_default {
                defaults.push(candidate);
            } else if trait_meta.implementing_type.is_some() {
                implementations.push(candidate);
            } else {
                trait_signatures.push(candidate);
            }
        } else {
            non_trait.push(candidate);
        }
    }

    if is_trait_method_call {
        return pick_same_file_first(implementations, context_file)
            .or_else(|| pick_same_file_first(defaults, context_file))
            .or_else(|| pick_same_file_first(trait_signatures, context_file))
            .unwrap_or_default();
    }

    if !is_trait_method_call && !caller_is_trait_method && !non_trait.is_empty() {
        let filtered: Vec<_> = if let Some(caller_fqn) = caller_fqn {
            let struct_name = caller_fqn.rsplit("::").nth(1).unwrap_or("");
            non_trait
                .into_iter()
                .filter(|c| !c.record.fqn.contains(&format!("::{}::", struct_name)))
                .collect()
        } else {
            non_trait
        };

        let same_file: Vec<_> = filtered
            .iter()
            .filter(|c| c.record.file == context_file)
            .copied()
            .collect();
        if !same_file.is_empty() {
            return same_file;
        }
        return filtered;
    }

    if !implementations.is_empty() {
        return pick_same_file_first(implementations, context_file).unwrap_or_default();
    }

    if !defaults.is_empty() {
        return pick_same_file_first(defaults, context_file).unwrap_or_default();
    }

    non_trait
}

fn pick_same_file_first<'a>(
    items: Vec<ResolvedSymbol<'a>>,
    context_file: &str,
) -> Option<Vec<ResolvedSymbol<'a>>> {
    if items.is_empty() {
        return None;
    }
    let same_file: Vec<_> = items
        .iter()
        .filter(|c| c.record.file == context_file)
        .copied()
        .collect();
    if !same_file.is_empty() {
        Some(same_file)
    } else {
        Some(items)
    }
}

// ---------------------------------------------------------------
// Call site name parsing
// ---------------------------------------------------------------

fn parse_call_site_name(name: &str) -> (String, String) {
    if let Some(colon_pos) = name.find(':') {
        let call_type = name[..colon_pos].to_string();
        let target_name = name[colon_pos + 1..].to_string();
        (call_type, target_name)
    } else {
        ("function_call".to_string(), name.to_string())
    }
}

// ---------------------------------------------------------------
// Trait object detection (tiers 1–3)
// ---------------------------------------------------------------

/// Tier 1: LSP hover — cannot run inside async context (would panic on block_on).
fn detect_trait_object_via_lsp(
    detector: &Arc<crate::infrastructure::lsp::receiver_detector::ReceiverTypeDetector>,
    call_site: &CallSite,
    receiver_range: &Range,
) -> Option<bool> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return None;
    }

    match tokio::runtime::Runtime::new() {
        Ok(rt) => {
            match rt.block_on(detector.is_trait_object_call(call_site, Some(receiver_range))) {
                Ok(Some(is_trait)) => Some(is_trait),
                Ok(None) => None,
                Err(e) => {
                    debug!("LSP hover failed: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            debug!("Failed to create runtime for LSP hover: {}", e);
            None
        }
    }
}

/// Tier 2: AST pattern matching using cached file contents.
fn detect_trait_object_via_ast(
    receiver_range: &Range,
    file_cache: &mut HashMap<String, Option<String>>,
) -> Option<bool> {
    let content = file_cache
        .entry(receiver_range.file.clone())
        .or_insert_with(|| std::fs::read_to_string(&receiver_range.file).ok())
        .as_ref()?;

    let receiver_name = extract_receiver_name(receiver_range, content);
    if receiver_name.is_empty() {
        return None;
    }

    let call_line = receiver_range.start_line as usize;
    let lines: Vec<&str> = content.lines().collect();
    let search_start = call_line.saturating_sub(50);

    for line_num in search_start..call_line {
        if let Some(line) = lines.get(line_num) {
            if check_line_for_trait_object_pattern(line, &receiver_name) {
                return Some(true);
            }
        }
    }

    None
}

fn extract_receiver_name(receiver_range: &Range, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if let Some(line) = lines.get(receiver_range.start_line as usize) {
        let start_char = receiver_range.start_char as usize;
        let end_char = receiver_range.end_char as usize;
        if start_char < end_char {
            if let Some(s) = line.get(start_char..end_char) {
                return s.trim().to_string();
            }
            let start = start_char.min(line.chars().count());
            let end = end_char.min(line.chars().count());
            if start < end {
                return line
                    .chars()
                    .skip(start)
                    .take(end - start)
                    .collect::<String>()
                    .trim()
                    .to_string();
            }
        }
    }
    String::new()
}

fn check_line_for_trait_object_pattern(line: &str, var_name: &str) -> bool {
    const TRAIT_PATTERNS: &[&str] = &["&dyn", "Box<dyn", "Arc<dyn", "Rc<dyn", "dyn "];

    if !line.contains(var_name) {
        return false;
    }

    for pattern in TRAIT_PATTERNS {
        if line.contains(pattern) {
            if let Some(var_pos) = line.find(var_name) {
                if let Some(pattern_pos) = line.find(pattern) {
                    let distance = var_pos.abs_diff(pattern_pos);
                    if distance < 100 {
                        return true;
                    }
                }
            }
        }
    }

    false
}

// ---------------------------------------------------------------
// Ambiguous builtin filter
// ---------------------------------------------------------------

fn is_ambiguous_builtin_method(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "write"
            | "close"
            | "open"
            | "send"
            | "copy"
            | "clone"
            | "encode"
            | "decode"
            | "sign"
            | "verify"
            | "create"
            | "delete"
            | "get"
            | "set"
            | "push"
            | "pop"
            | "append"
            | "extend"
            | "update"
            | "remove"
            | "insert"
            | "contains"
            | "seek"
            | "flush"
            | "format"
            | "parse"
            | "setdefault"
            | "items"
            | "keys"
            | "values"
            | "Write"
            | "Read"
            | "Close"
            | "String"
            | "Error"
            | "Serve"
            | "Handle"
            | "Listen"
            | "decrypt"
            | "encrypt"
            | "digest"
            | "fetch"
            | "abort"
            | "dispatch"
    )
}

// ---------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct ResolutionTelemetry {
    direct_hits: usize,
    suffix_hits: usize,
    simple_hits: usize,
    constructor_fallbacks: usize,
    unresolved: usize,
}

impl ResolutionTelemetry {
    fn record(&mut self, strategy: ResolutionStrategy) {
        match strategy {
            ResolutionStrategy::DirectFqn => self.direct_hits += 1,
            ResolutionStrategy::FqnSuffix => self.suffix_hits += 1,
            ResolutionStrategy::SimpleName => self.simple_hits += 1,
            ResolutionStrategy::DefinitionLocation => self.direct_hits += 1,
        }
    }

    fn record_constructor(&mut self, strategy: ResolutionStrategy) {
        self.constructor_fallbacks += 1;
        self.record(strategy);
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod utf8_tests {
    use super::*;

    #[test]
    fn extract_receiver_name_never_panics_on_utf8_boundaries() {
        let content = "let café = 1;\n";
        let range = Range::with_file(0, 0, 0, 0, "test.rs".to_string());
        let mut receiver = range;
        receiver.start_line = 0;
        receiver.start_char = 6;
        receiver.end_line = 0;
        receiver.end_char = 7;

        let _ = extract_receiver_name(&receiver, content);
    }
}
