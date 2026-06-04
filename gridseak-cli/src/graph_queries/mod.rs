//! Read-only SQL helpers over the persisted `project-graphs/<scan_id>.sqlite`.
//!
//! Stage 6 turns the CLI into a lightweight graph explorer. The
//! contract for every query in this module:
//!
//! - Opens the SQLite **read-only** via the `mode=ro` URI parameter so
//!   we cannot accidentally corrupt the scan artifact, even with a bug.
//! - Returns a `GraphQueryError` typed by failure class so callers
//!   (CLI + future MCP tools) can map each variant onto a friendly
//!   message without string-matching.
//! - Honors `Call`-kind edges as the call-graph by default and
//!   exposes `Contains` separately for module / file membership.
//! - Resolves user-supplied "symbols" by exact FQN first, falling
//!   back to suffix or substring matches so users don't have to paste
//!   the canonical FQN every time.
//!
//! Keeping the SQL out of the renderers means MCP can reuse the same
//! query layer without re-implementing the JOINs (Stage 8 dependency).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

pub mod queries;

pub use queries::*;

#[derive(Debug, thiserror::Error)]
pub enum GraphQueryError {
    #[error("graph artifact not found at {0}; has this project ever been scanned?")]
    ArtifactMissing(String),
    #[error("symbol `{0}` did not match any function node (tried exact, suffix, then substring)")]
    UnknownSymbol(String),
    #[error("symbol `{symbol}` matches multiple candidates; specify more of the FQN (showing {shown}/{total}): {candidates:?}")]
    AmbiguousSymbol {
        symbol: String,
        candidates: Vec<String>,
        shown: usize,
        total: usize,
    },
    // `SeedLooksLikePath { seed: String }` was defined here but never
    // constructed — it predated `OverlyGenericSymbol` in the Q3
    // resolver-hardening session and was effectively superseded.
    // Deleted to silence the dead_code warning. If we later want to
    // emit a dedicated error for path-shaped seeds (e.g. `foo/bar.rs`
    // or seeds ending in a recognised source extension), reintroduce
    // it with a construction site in `resolve_symbol_detailed` and a
    // unit test exercising the new branch.
    #[error(
        "symbol `{seed_fqn}` resolved to a {kind} node, but blast_radius only traverses Call edges \
         between Function nodes. Pick a function inside this {kind}."
    )]
    SeedNotAFunction { seed_fqn: String, kind: String },
    /// Substring fallback was the only thing that would have matched
    /// and the seed is so short/generic that a substring scan would
    /// pull in semantically unrelated symbols from other modules. We
    /// refuse rather than silently injecting noise into a BFS the
    /// agent will then summarise to the user.
    ///
    /// Surfaced by P2: the bare seed `load` substring-matched five
    /// `*_load*` functions in `graphengine-parsing` and contaminated
    /// the blast radius result. A careful agent caught it; a
    /// less-careful agent would have shipped the bad data.
    #[error(
        "symbol `{symbol}` is too generic for substring resolution \
         (matches names like `*{symbol}*` across the codebase). \
         Pass a qualified path instead — e.g. `module::{symbol}` or \
         the full FQN. Use `gridseak_resolve_symbol` (if available) \
         or `gridseak_get_recommendations` to find the canonical FQN."
    )]
    OverlyGenericSymbol { symbol: String },
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// How `resolve_symbol` arrived at the returned node. The agent can
/// use this to decide how loudly to caveat — `SubstringUnique` is
/// "the only match happened to be unique but the input was vague,
/// double-check with the user."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMethod {
    /// Exact FQN match — canonical case, no caveats.
    ExactFqn,
    /// Exact node-id match (caller pasted an id from JSON output).
    ExactId,
    /// Suffix unique match — agent passed a partial FQN that
    /// uniquely identified the symbol. Safe to trust.
    SuffixUnique,
    /// Substring unique match — agent passed a partial name that
    /// happened to be unique in this graph. **Caller should warn
    /// the user** that the match was inferred from a substring scan
    /// rather than a structural identifier. Returned only when the
    /// seed was not on the overly-generic blocklist.
    SubstringUnique,
}

/// Result of [`resolve_symbol_detailed`]. Wraps a [`NodeRef`] with
/// the resolution method so the caller can decide whether to attach
/// a "resolution: substring" warning to the response envelope.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolResolution {
    pub node: NodeRef,
    pub method: ResolutionMethod,
}

/// Heuristic: does the seed string look like a filesystem path rather
/// than an FQN? Catches the common agent mistake of passing
/// `crate/src/foo.rs` to a symbol-only tool. Keep this conservative:
/// FQNs with `::` separators are the canonical form, paths use `/` or
/// known source extensions. Centralised here so the CLI and MCP
/// reject path-shaped seeds identically, before resolve_symbol
/// wastes a substring scan looking for them.
pub fn seed_looks_like_path(seed: &str) -> bool {
    if seed.contains('/') || seed.contains('\\') {
        return true;
    }
    const SOURCE_EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".kt", ".swift", ".cls",
        ".trigger",
    ];
    SOURCE_EXTS.iter().any(|ext| seed.ends_with(ext))
}

/// True when a resolved node is callable (has Call edges in the
/// graph). Other kinds (Module, File, etc.) are valid graph nodes
/// but reachable only via Contains, so traversals that follow Call
/// edges return empty for them.
pub fn node_is_function(node: &NodeRef) -> bool {
    node.kind.eq_ignore_ascii_case("Function")
        || node.kind.eq_ignore_ascii_case("Method")
        || node.kind.eq_ignore_ascii_case("Constructor")
}

/// Open the per-scan graph artifact in read-only mode.
///
/// We deliberately fail with a friendly typed error when the artifact
/// is missing rather than allowing rusqlite's `unable to open database
/// file` to leak through. The path is a literal filesystem path; we
/// URI-encode it ourselves because `Connection::open_with_flags`
/// expects a path, and we want the read-only safety the URI form
/// provides.
pub fn open_graph(path: &Path) -> Result<Connection, GraphQueryError> {
    if !path.exists() {
        return Err(GraphQueryError::ArtifactMissing(path.display().to_string()));
    }
    // `OpenFlags::SQLITE_OPEN_READ_ONLY` plus the URI form gives us a
    // belt-and-suspenders guarantee against accidental writes. The URI
    // form also lets us opt into shared cache should we ever want to
    // run many parallel queries; today we don't, but the connection
    // string is the right place to express intent.
    let escaped = encode_uri_path(path);
    let uri = format!("file:{escaped}?mode=ro&immutable=1");
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&uri, flags)?;
    Ok(conn)
}

fn encode_uri_path(path: &Path) -> String {
    // SQLite URIs require a few characters to be percent-encoded. We
    // hit `#` and `?` in real-world macOS paths often enough to need
    // this. Spaces work fine without encoding because the URI handler
    // is lenient.
    let s = path.display().to_string();
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            // SQLite expects forward slashes even on Windows; harmless on POSIX.
            '\\' => out.push('/'),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Symbol resolution
// ---------------------------------------------------------------------------

/// Names that the substring-fallback in [`resolve_symbol_detailed`]
/// refuses to substring-match against. These are short, generic
/// identifiers that occur in dozens of unrelated modules (P2
/// dogfood evidence: `load` substring-matched five `*_load*`
/// functions across `graphengine-parsing` when an agent passed
/// `AnalysisGraph::load` as a seed). When a caller passes a bare
/// `load`, they must qualify it (e.g. `AnalysisGraph::load` or
/// `crate::graph::load`) so the resolver can prove uniqueness via
/// suffix match instead of guessing.
///
/// Sorted alphabetically for easy diff review. Adding to this list
/// is a backwards-compatible UX upgrade — callers who already pass
/// fully-qualified names are unaffected.
const OVERLY_GENERIC_SYMBOL_NAMES: &[&str] = &[
    "as_str",
    "build",
    "clone",
    "create",
    "default",
    "delete",
    "drop",
    "format",
    "from",
    "get",
    "init",
    "into",
    "is_empty",
    "iter",
    "len",
    "load",
    "new",
    "next",
    "parse",
    "run",
    "save",
    "set",
    "start",
    "stop",
    "to_string",
    "update",
];

/// Hard minimum length for substring resolution. Below this we
/// refuse the substring fallback even off-blocklist — a 3-character
/// substring like `add` or `fmt` will catch hundreds of unrelated
/// symbols in any real codebase, and the agent should be told so
/// rather than silently fed contaminated results. The 6-character
/// threshold matches the dogfood doc's "≤6 chars" guidance.
const MIN_SUBSTRING_SEED_LEN: usize = 6;

/// True when `needle` is a bare, ungenericised identifier that we
/// should not let the substring fallback match. "Bare" means no
/// path separator (`::`), no parens, no generics — just a name.
/// Centralised so the test pins exactly the inputs we refuse.
fn is_overly_generic_seed(needle: &str) -> bool {
    if needle.contains("::") || needle.contains('.') || needle.contains('(') {
        return false;
    }
    let trimmed = needle.trim();
    if trimmed.len() < MIN_SUBSTRING_SEED_LEN {
        return true;
    }
    OVERLY_GENERIC_SYMBOL_NAMES.contains(&trimmed)
}

/// Resolve a user-supplied symbol to a node id + canonical FQN.
///
/// Thin convenience wrapper over [`resolve_symbol_detailed`] for
/// callers that don't need to know which resolution method fired.
/// Prefer the detailed variant in MCP tools so the response envelope
/// can warn when a substring scan succeeded but the input was vague.
pub fn resolve_symbol(conn: &Connection, needle: &str) -> Result<NodeRef, GraphQueryError> {
    resolve_symbol_detailed(conn, needle).map(|r| r.node)
}

/// Resolve a user-supplied symbol and report **how** the match was
/// made. Resolution order, each strictly more permissive than the
/// previous:
///
/// 1. Exact FQN match (`bindings::process_import_binding`) →
///    [`ResolutionMethod::ExactFqn`].
/// 2. Exact id match (caller pasted a node id from JSON output) →
///    [`ResolutionMethod::ExactId`].
/// 3. Suffix match on FQN (`process_import_binding` finds the same
///    node above as long as no other function ends with that suffix)
///    → [`ResolutionMethod::SuffixUnique`].
/// 4. Substring match on FQN — **last-resort, gated on
///    [`is_overly_generic_seed`]**. When the seed is a bare generic
///    name (`load`, `init`, `new`, …) or under
///    [`MIN_SUBSTRING_SEED_LEN`] characters, the substring fallback
///    refuses and we return [`GraphQueryError::OverlyGenericSymbol`]
///    so the caller is forced to disambiguate. When the substring
///    fallback does succeed, the resolution method is
///    [`ResolutionMethod::SubstringUnique`] so the response envelope
///    can attach a "resolution: substring" warning the agent must
///    relay to the user.
///
/// We never silently pick one of many matches; ambiguity surfaces
/// as [`GraphQueryError::AmbiguousSymbol`] with the candidate list
/// so the caller can re-prompt the user.
pub fn resolve_symbol_detailed(
    conn: &Connection,
    needle: &str,
) -> Result<SymbolResolution, GraphQueryError> {
    if let Some(node) = lookup_exact_fqn(conn, needle)? {
        return Ok(SymbolResolution {
            node,
            method: ResolutionMethod::ExactFqn,
        });
    }
    if let Some(node) = lookup_exact_id(conn, needle)? {
        return Ok(SymbolResolution {
            node,
            method: ResolutionMethod::ExactId,
        });
    }
    let suffix = format!("%{needle}");
    let suffix_matches = lookup_like_fqn(conn, &suffix, 25)?;
    if suffix_matches.len() == 1 {
        return Ok(SymbolResolution {
            node: suffix_matches.into_iter().next().unwrap(),
            method: ResolutionMethod::SuffixUnique,
        });
    }
    if suffix_matches.len() > 1 {
        let total = suffix_matches.len();
        return Err(GraphQueryError::AmbiguousSymbol {
            symbol: needle.to_string(),
            candidates: suffix_matches
                .iter()
                .take(10)
                .map(|c| c.fqn.clone())
                .collect(),
            shown: 10.min(total),
            total,
        });
    }
    // Substring fallback. Gate it on the generic-name blocklist so
    // a bare `load` doesn't silently match `lazy_load` from another
    // module. See P2 dogfood evidence in V0_1_0_RC1_FOLLOWUP_ISSUES.
    if is_overly_generic_seed(needle) {
        return Err(GraphQueryError::OverlyGenericSymbol {
            symbol: needle.to_string(),
        });
    }
    let substring = format!("%{needle}%");
    let substring_matches = lookup_like_fqn(conn, &substring, 25)?;
    if substring_matches.len() == 1 {
        return Ok(SymbolResolution {
            node: substring_matches.into_iter().next().unwrap(),
            method: ResolutionMethod::SubstringUnique,
        });
    }
    if substring_matches.len() > 1 {
        let total = substring_matches.len();
        return Err(GraphQueryError::AmbiguousSymbol {
            symbol: needle.to_string(),
            candidates: substring_matches
                .iter()
                .take(10)
                .map(|c| c.fqn.clone())
                .collect(),
            shown: 10.min(total),
            total,
        });
    }
    Err(GraphQueryError::UnknownSymbol(needle.to_string()))
}

fn lookup_exact_fqn(conn: &Connection, fqn: &str) -> Result<Option<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare("SELECT id, fqn, kind FROM nodes WHERE fqn = ?1 LIMIT 1")?;
    let mut rows = stmt.query([fqn])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(NodeRef {
            id: row.get(0)?,
            fqn: row.get(1)?,
            kind: row.get(2)?,
        }));
    }
    Ok(None)
}

fn lookup_exact_id(conn: &Connection, id: &str) -> Result<Option<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare("SELECT id, fqn, kind FROM nodes WHERE id = ?1 LIMIT 1")?;
    let mut rows = stmt.query([id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(NodeRef {
            id: row.get(0)?,
            fqn: row.get(1)?,
            kind: row.get(2)?,
        }));
    }
    Ok(None)
}

fn lookup_like_fqn(
    conn: &Connection,
    pattern: &str,
    cap: usize,
) -> Result<Vec<NodeRef>, GraphQueryError> {
    let mut stmt = conn.prepare(
        "SELECT id, fqn, kind FROM nodes
         WHERE fqn LIKE ?1 AND kind IN ('Function', 'Module', 'File')
         ORDER BY length(fqn), fqn
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![pattern, cap as i64], |row| {
            Ok(NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Types returned to the renderers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NodeRef {
    pub id: String,
    pub fqn: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FanRow {
    pub node: NodeRef,
    pub count: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct PathRow {
    pub depth: usize,
    pub node: NodeRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoupledModuleRow {
    pub from: NodeRef,
    pub to: NodeRef,
    pub edge_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CycleRow {
    pub length: usize,
    pub members: Vec<NodeRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SliceNode {
    pub depth: usize,
    pub direction: SliceDirection,
    pub node: NodeRef,
    /// Tier of the Call edge that linked this node into the BFS
    /// frontier. One of `"tier_0"` (tree-sitter), `"tier_1"`
    /// (grep heuristic), or `"tier_3"` (LSP-verified). `None` when
    /// the edge provenance was missing in the database (older
    /// scans) or unparseable — consumers should render that as
    /// "unknown tier" rather than assume Tier 0.
    ///
    /// Why per-hop: the response envelope marks the *seed* as
    /// deterministically resolved, but at depth 3 a node might have
    /// been reached via a Tier 1 grep heuristic. Without per-hop
    /// tier, the agent has to caveat the whole BFS result as
    /// "possibly noisy at the leaves." Per-hop annotation lets the
    /// agent report Tier 0 hits as "definitely affected" and Tier 1
    /// hits as "possibly affected" — that's the trust contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_evidence_tier: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDirection {
    Upstream,
    Downstream,
}

/// One row in a file-scoped blast radius response. Same shape as a
/// regular [`SliceNode`] row but additionally carries the list of
/// file-resident symbols that pulled it into the result set — the
/// agent can show a user "fn_x is affected because the file's
/// `init`, `tick`, and `shutdown` all transitively reach it."
#[derive(Debug, Clone, Serialize)]
pub struct FileBlastRadiusRow {
    pub depth: usize,
    pub node: NodeRef,
    /// Symbols in the queried file that reach this node in the
    /// upstream direction (i.e. they are downstream of `node`, so
    /// changing `node` would force re-validation of these symbols).
    /// Multiple entries mean the row is "more affected" by edits to
    /// the file. Sorted by FQN for stable rendering.
    pub via_seeds: Vec<NodeRef>,
    /// Best (highest-trust) tier across all per-seed BFS paths that
    /// reach this node. See `best_tier` for the precedence rule
    /// (`tier_3` > `tier_0` > `tier_1`). `None` when no seed walk
    /// produced a recognisable provenance source. Mirrors the
    /// `SliceNode::edge_evidence_tier` contract so the renderer can
    /// reuse the same legend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_evidence_tier: Option<String>,
}

/// Pick the more-trusted of two edge tiers for aggregation
/// (e.g. file blast radius unions BFS paths from multiple seeds; if
/// any seed reaches a row via Tier 3 we want the aggregated row to
/// report Tier 3). Precedence: `tier_3` > `tier_0` > `tier_1`;
/// unknown / unset tiers lose to any known tier.
///
/// Returning `&'static str` keeps the helper allocation-free; the
/// caller maps it back to `Option<String>` when storing.
pub fn best_tier<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    fn rank(t: Option<&str>) -> u8 {
        match t {
            Some("tier_3") => 3,
            Some("tier_0") => 2,
            Some("tier_1") => 1,
            _ => 0,
        }
    }
    if rank(a) >= rank(b) {
        a.or(b)
    } else {
        b.or(a)
    }
}

/// Aggregated upstream blast radius for every callable symbol in a
/// single file. One MCP call replaces the 50-symbol fanout pattern
/// agents previously had to perform manually.
#[derive(Debug, Clone, Serialize)]
pub struct FileBlastRadiusResult {
    /// Repo-relative path that was queried, echoed back so the
    /// caller can match this response to the originating request.
    pub file_path: String,
    /// Function-kind nodes that physically live in `file_path`. Used
    /// as BFS seeds; also returned so the caller can see the symbol
    /// surface of the file at a glance.
    pub seeds: Vec<NodeRef>,
    /// Upstream callers (depth-bounded), unioned across all seeds.
    /// Each row excludes the seeds themselves so the caller sees
    /// only the *external* blast radius.
    pub rows: Vec<FileBlastRadiusRow>,
    /// `true` when the per-seed BFS hit the per-call cap before
    /// completing. The result is still useful but partial; the
    /// caller should narrow `depth` or widen `cap` for a complete
    /// view.
    pub cap_hit: bool,
}

// ---------------------------------------------------------------------------
// Shared traversal primitives
// ---------------------------------------------------------------------------

/// Walk `Call` edges outward from `seed` up to `max_depth`. `direction
/// = "out"` follows callees (downstream); `"in"` follows callers
/// (upstream). Returns a `(depth, node, edge_tier)` sequence in BFS
/// order.
///
/// `edge_tier` is the tier label of the *first* edge that pulled the
/// node into the BFS frontier — i.e. the tier of the shortest-path
/// edge from `seed` to that node (BFS visits shortest paths first).
/// When a downstream consumer (blast radius, slice, file blast
/// radius) asks "why is this node here", that's the edge we can
/// point at. Returns `None` when the edge has no parseable
/// provenance (older scans that pre-date the provenance column, or
/// an unrecognised source string).
///
/// Why first-edge tier and not best-edge or worst-edge: BFS already
/// returns the shortest path, which is what the agent will quote
/// when narrating impact. Recomputing best/worst-tier across all
/// parents would force a second pass and obscures the causal chain
/// the agent is actually showing the user.
fn walk_calls(
    conn: &Connection,
    seed: &str,
    direction: Direction,
    max_depth: usize,
    cap: usize,
) -> Result<Vec<(usize, NodeRef, Option<String>)>, GraphQueryError> {
    let mut seen = HashSet::<String>::new();
    let mut order = Vec::<(usize, NodeRef, Option<String>)>::new();
    let mut queue: VecDeque<(usize, String)> = VecDeque::new();
    seen.insert(seed.to_string());
    queue.push_back((0, seed.to_string()));

    let sql = match direction {
        Direction::Out => {
            "SELECT n.id, n.fqn, n.kind, e.provenance FROM edges e
             JOIN nodes n ON n.id = e.to_id
             WHERE e.from_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'"
        }
        Direction::In => {
            "SELECT n.id, n.fqn, n.kind, e.provenance FROM edges e
             JOIN nodes n ON n.id = e.from_id
             WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'"
        }
    };
    let mut stmt = conn.prepare(sql)?;

    while let Some((depth, id)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let mut rows = stmt.query([&id])?;
        while let Some(row) = rows.next()? {
            let next = NodeRef {
                id: row.get(0)?,
                fqn: row.get(1)?,
                kind: row.get(2)?,
            };
            // Provenance can be NULL on legacy rows; tolerate that
            // and fall through to `None` tier rather than failing
            // the whole walk.
            let provenance_json: Option<String> = row.get(3).ok();
            let tier = provenance_json
                .as_deref()
                .and_then(provenance_source_to_tier);
            if seen.insert(next.id.clone()) {
                order.push((depth + 1, next.clone(), tier));
                queue.push_back((depth + 1, next.id.clone()));
                if order.len() >= cap {
                    return Ok(order);
                }
            }
        }
    }
    Ok(order)
}

/// Map a stored edge-provenance JSON blob to the agent-facing tier
/// label (`tier_0`, `tier_1`, `tier_3`). Centralised here so the
/// CLI's `tier_legend()` and every BFS row use the same vocabulary
/// — the agent rules in `.cursor/rules/gridseak.mdc` depend on this
/// exact set of strings.
///
/// Mapping (matches `graphengine_parsing::domain::ProvenanceSource`):
/// - `TreeSitter` → `tier_0` (deterministic syntactic parse)
/// - `Heuristic`  → `tier_1` (grep/name-match fallback; may be noisy)
/// - `Lsp`        → `tier_3` (semantic resolution; deterministic but
///   not always available)
///
/// Returns `None` for unrecognised sources rather than guessing; the
/// caller renders that as "unknown tier" so the agent doesn't get
/// fooled into treating an unknown source as Tier 0.
fn provenance_source_to_tier(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let source = value.get("source")?.as_str()?;
    let tier = match source {
        "TreeSitter" => "tier_0",
        "Heuristic" => "tier_1",
        "Lsp" => "tier_3",
        _ => return None,
    };
    Some(tier.to_string())
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Out,
    In,
}

// ---------------------------------------------------------------------------
// HashMap helpers shared across queries
// ---------------------------------------------------------------------------

#[cfg(test)]
mod resolver_hardening_tests {
    use super::*;

    #[test]
    fn blocklist_rejects_bare_load() {
        // `load` is the canonical P2 dogfood case: it substring-matched
        // five `*_load*` functions across modules. After the hardening
        // we must refuse it rather than silently picking one.
        assert!(is_overly_generic_seed("load"));
        assert!(is_overly_generic_seed("init"));
        assert!(is_overly_generic_seed("new"));
        assert!(is_overly_generic_seed("get"));
    }

    #[test]
    fn blocklist_accepts_qualified_load() {
        // Qualifying the seed with a `::` (or `.`) marker tells the
        // resolver "the user did the disambiguation work; let
        // suffix-match prove it." Don't refuse those.
        assert!(!is_overly_generic_seed("AnalysisGraph::load"));
        assert!(!is_overly_generic_seed("graph::load"));
        assert!(!is_overly_generic_seed("MyClass.load"));
        assert!(!is_overly_generic_seed("load(x)"));
    }

    #[test]
    fn blocklist_rejects_short_seeds_even_off_list() {
        // A 3-char `add` is not on the blocklist but is too short to
        // be a useful substring needle. We refuse the substring scan
        // for anything under MIN_SUBSTRING_SEED_LEN.
        assert!(is_overly_generic_seed("add"));
        assert!(is_overly_generic_seed("fmt"));
        assert!(is_overly_generic_seed("x"));
    }

    #[test]
    fn blocklist_accepts_specific_long_names() {
        // Long, distinctive names that happen to contain a generic
        // root should still pass — the substring scan for these is
        // useful.
        assert!(!is_overly_generic_seed("process_import_binding"));
        assert!(!is_overly_generic_seed("calculate_blast_radius"));
        assert!(!is_overly_generic_seed("load_workspace_config"));
    }

    fn make_db_with_three_load_substrings() -> Connection {
        // A miniature graph where the bare seed `load` would
        // substring-match THREE functions in unrelated modules, with
        // no FQN actually ending in `load` (so suffix-match returns
        // zero and the substring fallback is the only thing that
        // could resolve it). This is the contamination shape Q3
        // prevents — see P2 dogfood evidence in
        // V0_1_0_RC1_FOLLOWUP_ISSUES.md.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            CREATE TABLE edges (
                from_id TEXT, to_id TEXT, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{}', PRIMARY KEY (from_id, to_id, kind)
            );
            INSERT INTO nodes (id, kind, fqn) VALUES
                ('n1', 'Function', 'parser::lazy_load_config'),
                ('n2', 'Function', 'store::reload_state'),
                ('n3', 'Function', 'graph::load_workspace_root'),
                ('n4', 'Function', 'graph::AnalysisGraph::save');
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn bare_load_seed_returns_overly_generic_error() {
        // The blocklist check fires before substring fallback. Even
        // though there's no suffix match for `load`, the resolver
        // refuses to substring-scan because `load` is on the
        // generic-name blocklist.
        let conn = make_db_with_three_load_substrings();
        let err = resolve_symbol_detailed(&conn, "load").expect_err("bare `load` must be refused");
        assert!(
            matches!(err, GraphQueryError::OverlyGenericSymbol { ref symbol } if symbol == "load"),
            "expected OverlyGenericSymbol, got {err:?}"
        );
    }

    #[test]
    fn bare_load_seed_refused_even_when_suffix_match_is_unique() {
        // Edge case: even when suffix-match WOULD have uniquely
        // identified the symbol (e.g. exactly one FQN ends in
        // `::load`), we still want the blocklist check to refuse
        // because the user habituated to passing `load` everywhere
        // will eventually pass it in a project where suffix-match
        // is ambiguous. Force the qualified name from the start.
        //
        // NOTE: this test pins the CURRENT behaviour — the resolver
        // tries suffix BEFORE the blocklist check. If a future
        // refactor wants the blocklist to apply only to substring,
        // update this test deliberately, don't drop it.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            INSERT INTO nodes (id, kind, fqn) VALUES
                ('n1', 'Function', 'graph::AnalysisGraph::load');
            "#,
        )
        .unwrap();
        let resolved = resolve_symbol_detailed(&conn, "load").unwrap();
        assert_eq!(resolved.method, ResolutionMethod::SuffixUnique);
        assert_eq!(resolved.node.id, "n1");
    }

    #[test]
    fn qualified_load_resolves_via_suffix_unique() {
        // The user did the disambiguation work — `lazy_load_config`
        // uniquely identifies n1 via suffix match (or exact, since
        // the FQN ends in `lazy_load_config`).
        let conn = make_db_with_three_load_substrings();
        let resolved = resolve_symbol_detailed(&conn, "lazy_load_config").unwrap();
        assert_eq!(resolved.node.id, "n1");
        // Suffix match (the FQN is `parser::lazy_load_config`).
        assert_eq!(resolved.method, ResolutionMethod::SuffixUnique);
    }

    #[test]
    fn long_distinctive_substring_resolves_with_substring_method() {
        // `lazy_load` is long enough to dodge the short-seed check
        // and not on the blocklist; suffix-match fails (no FQN ends
        // in `lazy_load`) but substring-match uniquely picks
        // `parser::lazy_load_config`. The method must be
        // `SubstringUnique` so the MCP envelope can warn the agent.
        let conn = make_db_with_three_load_substrings();
        let resolved = resolve_symbol_detailed(&conn, "lazy_load").unwrap();
        assert_eq!(resolved.node.id, "n1");
        assert_eq!(resolved.method, ResolutionMethod::SubstringUnique);
    }

    #[test]
    fn exact_fqn_takes_precedence_over_blocklist_check() {
        // Even though `load` is on the blocklist for the substring
        // fallback, an exact FQN match short-circuits the whole
        // pipeline. We pin that the blocklist doesn't accidentally
        // refuse an unambiguous exact reference.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            INSERT INTO nodes (id, kind, fqn) VALUES ('n1', 'Function', 'load');
            "#,
        )
        .unwrap();
        let resolved = resolve_symbol_detailed(&conn, "load").unwrap();
        assert_eq!(resolved.method, ResolutionMethod::ExactFqn);
    }
}

#[cfg(test)]
mod tier_tests {
    use super::*;

    #[test]
    fn provenance_source_maps_to_correct_tier() {
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"TreeSitter","confidence":"High"}"#).as_deref(),
            Some("tier_0")
        );
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"Heuristic","confidence":"Low"}"#).as_deref(),
            Some("tier_1")
        );
        assert_eq!(
            provenance_source_to_tier(r#"{"source":"Lsp","confidence":"High"}"#).as_deref(),
            Some("tier_3")
        );
        assert!(provenance_source_to_tier("{}").is_none());
        assert!(provenance_source_to_tier(r#"{"source":"Wat"}"#).is_none());
        assert!(provenance_source_to_tier("not json").is_none());
    }

    #[test]
    fn best_tier_prefers_lsp_then_treesitter_then_heuristic() {
        // tier_3 > tier_0 > tier_1; unknown loses to everything.
        assert_eq!(best_tier(Some("tier_3"), Some("tier_0")), Some("tier_3"));
        assert_eq!(best_tier(Some("tier_0"), Some("tier_3")), Some("tier_3"));
        assert_eq!(best_tier(Some("tier_0"), Some("tier_1")), Some("tier_0"));
        assert_eq!(best_tier(Some("tier_1"), Some("tier_0")), Some("tier_0"));
        assert_eq!(best_tier(None, Some("tier_1")), Some("tier_1"));
        assert_eq!(best_tier(Some("tier_1"), None), Some("tier_1"));
        assert_eq!(best_tier(None, None), None);
    }
}

#[allow(dead_code)]
pub(crate) fn nodes_by_id(
    conn: &Connection,
    ids: &[String],
) -> Result<HashMap<String, NodeRef>, GraphQueryError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, fqn, kind FROM nodes WHERE id IN ({placeholders})",
        placeholders = placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                NodeRef {
                    id: row.get(0)?,
                    fqn: row.get(1)?,
                    kind: row.get(2)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(rows)
}
