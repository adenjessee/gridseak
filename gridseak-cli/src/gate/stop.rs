//! `gridseak gate --stop` — claim check at the end of an agent turn.
//!
//! Claude Code runs the `Stop` hook when the agent is about to end its
//! turn. If the final message claims completeness ("I updated all the
//! callers"), we check that claim against the scan graph: for every
//! function the working tree removed, renamed or re-signatured, each
//! graph caller must live in a changed file, and a caller of a removed
//! name must no longer mention it. Compiler/LSP-witnessed misses block
//! the stop and hand the list back to the agent; tree-sitter-only
//! misses are surfaced to the user without blocking.
//!
//! No claim → no work: the hook returns `{}` before touching git or
//! the graph. Every failure path also returns `{}`: a broken hook must
//! never trap the agent in its turn.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use gridseak_local_store::ProjectStore;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use crate::gate::artifact::{class_from_provenance, nodes_in_file, parse_location, WitnessClass};
use crate::gate::decision::{Decision, Permission, SymbolVerdict};
use crate::gate::ts_names::{extract_names, language_for_file, ExtractedName};
use crate::graph_queries::{is_top_level_scope_fqn, open_graph};

/// Upper bound on files inspected per stop; a giant working-tree diff
/// is a rebase or a vendored drop, not an agent edit.
const MAX_CHANGED_FILES: usize = 200;
/// Callers listed back to the agent; the rest are counted.
const MAX_LISTED: usize = 25;
/// Text fallback: names shorter than this match too much to be useful.
const MIN_TEXT_NAME: usize = 4;
/// Text fallback: more hits than this per name means the name is too
/// common to tell stale references from unrelated ones (Tier 1 rule).
const TEXT_SATURATION: usize = 50;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StopPayload {
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// True when Claude is already continuing because a Stop hook
    /// blocked once. We never block twice in a row.
    #[serde(default)]
    pub stop_hook_active: bool,
    /// Newer hosts pass the final message directly.
    #[serde(default)]
    pub last_assistant_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Removed,
    Signature,
}

impl ChangeKind {
    fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Removed => "removed or renamed",
            ChangeKind::Signature => "signature changed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedSymbol {
    /// Repo-relative path of the file that defined the symbol.
    pub file: String,
    pub name: String,
    pub kind: ChangeKind,
    /// 1-based line of the definition at `HEAD`. Tells apart same-named
    /// definitions in one file (Go `func URLParam` and `(*Context).URLParam`).
    pub old_line: u32,
}

#[derive(Debug, Clone)]
pub struct MissedCaller {
    pub target_fqn: String,
    pub target_kind: ChangeKind,
    pub caller_fqn: String,
    pub caller_file: String,
    pub caller_line: u32,
    pub class: WitnessClass,
    pub why: String,
}

/// A word match for a removed name that the graph did not report: code
/// outside the scan (Go `_examples/`, build-tagged files) or a call the
/// resolver missed. Heuristic; never blocks on its own.
#[derive(Debug, Clone)]
pub struct TextRef {
    pub name: String,
    pub kind: ChangeKind,
    pub file: String,
    pub line: u32,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct StopOutcome {
    /// JSON for the host (`{}`, a block, or a system message).
    pub host_stdout: String,
    /// Ledger row; `None` when no claim was made.
    pub decision: Option<Decision>,
}

impl StopOutcome {
    fn quiet() -> Self {
        Self {
            host_stdout: "{}\n".into(),
            decision: None,
        }
    }
}

/// Inputs the hook resolves from the host and the environment. Split
/// out so tests can pin the repo, artifact and message.
pub struct StopInputs {
    pub message: String,
    pub repo_root: PathBuf,
    pub artifact: PathBuf,
    pub scan_id: String,
    pub stop_hook_active: bool,
}

pub fn run_from_host(store: &ProjectStore, raw: &str, artifact: Option<PathBuf>) -> StopOutcome {
    match evaluate_host(store, raw, artifact) {
        Ok(outcome) => outcome,
        Err(_) => StopOutcome::quiet(),
    }
}

fn evaluate_host(
    store: &ProjectStore,
    raw: &str,
    artifact: Option<PathBuf>,
) -> Result<StopOutcome> {
    let payload: StopPayload = if raw.trim().is_empty() {
        StopPayload::default()
    } else {
        serde_json::from_str(raw).context("stop payload is not JSON")?
    };
    let message = match payload.last_assistant_message.clone() {
        Some(m) if !m.trim().is_empty() => m,
        _ => match &payload.transcript_path {
            Some(p) => last_assistant_text(Path::new(p)).unwrap_or_default(),
            None => String::new(),
        },
    };
    // Cheap path: no completeness claim, nothing to check.
    if find_completeness_claim(&message).is_none() {
        return Ok(StopOutcome::quiet());
    }

    let cwd = payload
        .cwd
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let repo_root = git_toplevel(&cwd).context("not a git repository")?;
    let (artifact, scan_id) = match artifact {
        Some(a) => (a, "cli-artifact".to_string()),
        None => match artifact_for(store, &repo_root) {
            Some(pair) => pair,
            None => return Ok(no_scan_notice()),
        },
    };
    evaluate(&StopInputs {
        message,
        repo_root,
        artifact,
        scan_id,
        stop_hook_active: payload.stop_hook_active,
    })
}

pub fn evaluate(inputs: &StopInputs) -> Result<StopOutcome> {
    let Some(claim) = find_completeness_claim(&inputs.message) else {
        return Ok(StopOutcome::quiet());
    };

    let changed_files = git_changed_files(&inputs.repo_root)?;
    if changed_files.is_empty() || changed_files.len() > MAX_CHANGED_FILES {
        return Ok(StopOutcome::quiet());
    }
    let changed_set: HashSet<String> = changed_files.iter().cloned().collect();

    let mut changed_symbols = Vec::new();
    let mut defined_now: HashSet<String> = HashSet::new();
    for file in &changed_files {
        if language_for_file(file).is_none() {
            continue;
        }
        let old = git_show_head(&inputs.repo_root, file).unwrap_or_default();
        let new = std::fs::read_to_string(inputs.repo_root.join(file)).unwrap_or_default();
        let (new_names, _) = extract_names(file, &new);
        defined_now.extend(new_names.iter().map(|n| n.name.clone()));
        changed_symbols.extend(changed_symbols_in(file, &old, &new));
    }
    // A name removed from one file and defined in another moved; its
    // callers may only need an import, which we cannot see here.
    changed_symbols.retain(|s| s.kind != ChangeKind::Removed || !defined_now.contains(&s.name));
    if changed_symbols.is_empty() {
        return Ok(StopOutcome::quiet());
    }

    let conn = open_graph(&inputs.artifact).context("open scan graph")?;
    let removed_here: HashSet<(String, String)> = changed_symbols
        .iter()
        .filter(|s| s.kind == ChangeKind::Removed)
        .map(|s| (s.file.clone(), s.name.clone()))
        .collect();

    let mut missed = Vec::new();
    let mut file_cache: HashMap<String, String> = HashMap::new();
    for sym in &changed_symbols {
        let same_name: Vec<_> = nodes_in_file(&conn, &sym.file)?
            .into_iter()
            .filter(|n| short_name(&n.fqn).eq_ignore_ascii_case(&sym.name))
            .collect();
        // Several definitions share the name: keep the one whose scanned
        // line is nearest the HEAD line (the scan may lag HEAD slightly).
        let targets = match same_name
            .iter()
            .map(|n| n.start_line.abs_diff(sym.old_line))
            .min()
        {
            Some(best) if same_name.len() > 1 && sym.old_line > 0 => same_name
                .into_iter()
                .filter(|n| n.start_line.abs_diff(sym.old_line) == best)
                .collect(),
            _ => same_name,
        };
        for target in targets {
            for caller in callers_located(&conn, &target.id)? {
                let caller_rel = relative_to(&caller.file, &inputs.repo_root);
                // A caller the agent deleted along with the target is not a miss.
                if removed_here.contains(&(caller_rel.clone(), short_name(&caller.fqn).to_string()))
                {
                    continue;
                }
                // Module-level "callers" are often just `from x import f`.
                // An import doesn't change when f's signature does.
                if sym.kind == ChangeKind::Signature && is_top_level_scope_fqn(&caller.fqn) {
                    let text = file_cache.entry(caller_rel.clone()).or_insert_with(|| {
                        std::fs::read_to_string(inputs.repo_root.join(&caller_rel))
                            .unwrap_or_default()
                    });
                    if only_import_mentions(text, &sym.name) {
                        continue;
                    }
                }
                let touched = file_in_set(&caller_rel, &changed_set);
                let why = if !touched {
                    Some("file not changed".to_string())
                } else if sym.kind == ChangeKind::Removed {
                    let text = file_cache.entry(caller_rel.clone()).or_insert_with(|| {
                        std::fs::read_to_string(inputs.repo_root.join(&caller_rel))
                            .unwrap_or_default()
                    });
                    contains_word(text, &sym.name)
                        .then(|| format!("still references `{}`", sym.name))
                } else {
                    None
                };
                if let Some(why) = why {
                    missed.push(MissedCaller {
                        target_fqn: target.fqn.clone(),
                        target_kind: sym.kind,
                        caller_fqn: caller.fqn,
                        caller_file: caller_rel,
                        caller_line: caller.line,
                        class: caller.class,
                        why,
                    });
                }
            }
        }
    }

    // A file's module-level entry adds nothing when a real function in
    // the same file is already listed for the same target.
    let named: HashSet<(String, String)> = missed
        .iter()
        .filter(|m| !is_top_level_scope_fqn(&m.caller_fqn))
        .map(|m| (m.target_fqn.clone(), m.caller_file.clone()))
        .collect();
    missed.retain(|m| {
        !is_top_level_scope_fqn(&m.caller_fqn)
            || !named.contains(&(m.target_fqn.clone(), m.caller_file.clone()))
    });

    let text_refs = text_references(
        &conn,
        &inputs.repo_root,
        &changed_symbols,
        &missed,
        &changed_set,
    );
    Ok(verdict(inputs, &claim, &changed_symbols, missed, text_refs))
}

/// Word matches the graph check did not already report.
///
/// - Removed or renamed names: any mention in a source file is stale.
/// - Changed signatures: mentions in files the agent did not touch, minus
///   import lines. The text can't show whether the call passes the new
///   arguments, so these are reported as "may still use the old signature".
///
/// Skipped for short names, names another graph node still defines, and
/// names past the saturation cap.
fn text_references(
    conn: &Connection,
    repo_root: &Path,
    changed: &[ChangedSymbol],
    missed: &[MissedCaller],
    changed_files: &HashSet<String>,
) -> Vec<TextRef> {
    let mut names: BTreeSet<(&str, ChangeKindKey)> = BTreeSet::new();
    for s in changed {
        if s.name.chars().count() >= MIN_TEXT_NAME {
            names.insert((s.name.as_str(), ChangeKindKey::from(s.kind)));
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    for (name, key) in names {
        let kind = key.kind();
        if graph_definitions(conn, name) > 1 {
            continue;
        }
        let reported: HashSet<&str> = missed
            .iter()
            .filter(|m| short_name(&m.target_fqn).eq_ignore_ascii_case(name))
            .map(|m| m.caller_file.as_str())
            .collect();
        let hits: Vec<TextRef> = git_grep_word(repo_root, name, kind)
            .into_iter()
            .filter(|r| language_for_file(&r.file).is_some())
            .filter(|r| !reported.contains(r.file.as_str()))
            .filter(|r| !is_comment_line(&r.snippet))
            .filter(|r| {
                kind == ChangeKind::Removed
                    || (!file_in_set(&r.file, changed_files) && !is_import_line(&r.snippet))
            })
            .collect();
        if hits.len() <= TEXT_SATURATION {
            for hit in hits {
                if seen.insert((hit.file.clone(), hit.line)) {
                    out.push(hit);
                }
            }
        }
    }
    out
}

/// Orderable stand-in for [`ChangeKind`] in the name set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ChangeKindKey {
    Removed,
    Signature,
}

impl ChangeKindKey {
    fn from(kind: ChangeKind) -> Self {
        match kind {
            ChangeKind::Removed => ChangeKindKey::Removed,
            ChangeKind::Signature => ChangeKindKey::Signature,
        }
    }

    fn kind(self) -> ChangeKind {
        match self {
            ChangeKindKey::Removed => ChangeKind::Removed,
            ChangeKindKey::Signature => ChangeKind::Signature,
        }
    }
}

/// True when every line of `text` that mentions `name` is part of an
/// import statement (Python `from x import (...)`, TS `import {...}`).
fn only_import_mentions(text: &str, name: &str) -> bool {
    let mut in_block: Option<char> = None;
    let mut mentioned = false;
    for line in text.lines() {
        let importing = match in_block {
            Some(closer) => {
                if line.contains(closer) {
                    in_block = None;
                }
                true
            }
            None => {
                let is_import = is_import_line(line);
                if is_import {
                    for (open, close) in [('(', ')'), ('{', '}')] {
                        if line.contains(open) && !line.contains(close) {
                            in_block = Some(close);
                        }
                    }
                }
                is_import
            }
        };
        if contains_word(line, name) {
            mentioned = true;
            if !importing {
                return false;
            }
        }
    }
    mentioned
}

fn is_import_line(line: &str) -> bool {
    let t = line.trim_start();
    ["import ", "from ", "export {", "export *", "use "]
        .iter()
        .any(|p| t.starts_with(p))
}

fn graph_definitions(conn: &Connection, name: &str) -> usize {
    let Ok(mut stmt) =
        conn.prepare("SELECT fqn FROM nodes WHERE kind IN ('Function', 'Method', 'Constructor')")
    else {
        return 0;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return 0;
    };
    rows.map_while(Result::ok)
        .filter(|fqn| short_name(fqn).eq_ignore_ascii_case(name))
        .count()
}

fn is_comment_line(line: &str) -> bool {
    let t = line.trim_start();
    ["//", "#", "/*", "*", "--"]
        .iter()
        .any(|p| t.starts_with(p))
}

fn verdict(
    inputs: &StopInputs,
    claim: &str,
    changed: &[ChangedSymbol],
    mut missed: Vec<MissedCaller>,
    text_refs: Vec<TextRef>,
) -> StopOutcome {
    missed.sort_by(|a, b| {
        (
            a.class != WitnessClass::Compiler,
            &a.caller_file,
            a.caller_line,
        )
            .cmp(&(
                b.class != WitnessClass::Compiler,
                &b.caller_file,
                b.caller_line,
            ))
    });
    let compiler = missed
        .iter()
        .filter(|m| m.class == WitnessClass::Compiler)
        .count();
    let tier = if compiler > 0 {
        "compiler"
    } else if !missed.is_empty() {
        "treesitter"
    } else if !text_refs.is_empty() {
        "text"
    } else {
        "none"
    };
    let clean = missed.is_empty() && text_refs.is_empty();

    let block = compiler > 0 && !inputs.stop_hook_active;
    let permission = if block {
        Permission::Deny
    } else if clean {
        Permission::Allow
    } else {
        Permission::Ask
    };

    let host_stdout = if clean {
        "{}\n".to_string()
    } else if block {
        json!({ "decision": "block", "reason": block_reason(claim, &missed, &text_refs) })
            .to_string()
            + "\n"
    } else {
        json!({
            "systemMessage": notice(claim, &missed, &text_refs, inputs.stop_hook_active && compiler > 0)
        })
        .to_string()
            + "\n"
    };

    let mut targets: BTreeSet<String> = missed.iter().map(|m| m.target_fqn.clone()).collect();
    targets.extend(text_refs.iter().map(|r| r.name.clone()));
    if targets.is_empty() {
        targets.extend(changed.iter().map(|c| c.name.clone()));
    }
    let decision = Decision {
        permission,
        reason: if clean {
            format!(
                "claim checked: {} changed symbol(s), no missed callers",
                changed.len()
            )
        } else {
            format!(
                "{} caller(s) not updated ({compiler} compiler-witnessed), {} text match(es)",
                missed.len(),
                text_refs.len()
            )
        },
        scan_id: inputs.scan_id.clone(),
        symbols: targets
            .iter()
            .map(|t| SymbolVerdict {
                fqn: t.clone(),
                claim: claim.to_string(),
                claim_verdict: if missed.iter().any(|m| &m.target_fqn == t)
                    || text_refs.iter().any(|r| &r.name == t)
                {
                    "refuted".into()
                } else {
                    "verified".into()
                },
                witnesses: Vec::new(),
                witness_class: tier.into(),
                permission,
            })
            .collect(),
        tier: tier.into(),
        witnesses: missed
            .iter()
            .map(|m| m.caller_fqn.clone())
            .chain(
                text_refs
                    .iter()
                    .map(|r| format!("text:{}:{}", r.file, r.line)),
            )
            .take(MAX_LISTED)
            .collect(),
        override_used: inputs.stop_hook_active && compiler > 0,
        host: "claude-stop".into(),
        file: String::new(),
        duration_ms: 0,
        extract_source: "git-diff".into(),
    };
    StopOutcome {
        host_stdout,
        decision: Some(decision),
    }
}

fn block_reason(claim: &str, missed: &[MissedCaller], text_refs: &[TextRef]) -> String {
    let mut lines = vec![
        format!("GridSeak claim check: your summary says \"{claim}\", but the call graph shows callers you did not update."),
        String::new(),
    ];
    lines.extend(listing(missed));
    if !text_refs.is_empty() {
        lines.push(String::new());
        lines.push(
            "Text matches for the old name outside the call graph (heuristic, may include false positives):"
                .into(),
        );
        lines.extend(text_listing(text_refs));
    }
    lines.push(String::new());
    lines.push(
        "Update these call sites, or say explicitly which ones you are leaving and why. \
         Compiler-witnessed edges come from the compiler index of the last scan; \
         tree-sitter edges may include false positives."
            .into(),
    );
    lines.join("\n")
}

fn notice(claim: &str, missed: &[MissedCaller], text_refs: &[TextRef], held_back: bool) -> String {
    let lead = if held_back {
        "GridSeak claim check (already asked the agent once, not blocking again)"
    } else if !missed.is_empty() {
        "GridSeak claim check (tree-sitter tier only, not blocking)"
    } else {
        "GridSeak claim check (text matches only, not blocking)"
    };
    let mut lines = vec![format!("{lead}: \"{claim}\" may be incomplete.")];
    lines.extend(listing(missed));
    if !text_refs.is_empty() {
        lines.push("Text matches for the old name outside the call graph (heuristic):".into());
        lines.extend(text_listing(text_refs));
    }
    lines.join("\n")
}

fn text_listing(refs: &[TextRef]) -> Vec<String> {
    let mut lines: Vec<String> = refs
        .iter()
        .take(MAX_LISTED)
        .map(|r| {
            let snippet: String = r.snippet.trim().chars().take(100).collect();
            match r.kind {
                ChangeKind::Removed => {
                    format!("- {}:{} mentions `{}`: {}", r.file, r.line, r.name, snippet)
                }
                ChangeKind::Signature => format!(
                    "- {}:{} uses `{}`, file not changed, may still pass the old arguments: {}",
                    r.file, r.line, r.name, snippet
                ),
            }
        })
        .collect();
    if refs.len() > MAX_LISTED {
        lines.push(format!("- … and {} more", refs.len() - MAX_LISTED));
    }
    lines
}

fn listing(missed: &[MissedCaller]) -> Vec<String> {
    let mut lines: Vec<String> = missed
        .iter()
        .take(MAX_LISTED)
        .map(|m| {
            format!(
                "- {} ({}:{}) calls {} [{}], {}; {} edge",
                m.caller_fqn,
                m.caller_file,
                m.caller_line,
                m.target_fqn,
                m.target_kind.as_str(),
                m.why,
                m.class.as_str()
            )
        })
        .collect();
    if missed.len() > MAX_LISTED {
        lines.push(format!("- … and {} more", missed.len() - MAX_LISTED));
    }
    lines
}

fn no_scan_notice() -> StopOutcome {
    StopOutcome {
        host_stdout: json!({
            "systemMessage": "GridSeak claim check skipped: no scan for this repo. Run `gridseak scan .`."
        })
        .to_string()
            + "\n",
        decision: None,
    }
}

// ---------------------------------------------------------------- claims

const QUANTIFIERS: &[&str] = &[
    "all ",
    "every ",
    "each ",
    "everywhere",
    "no other",
    "no remaining",
    "none of the other",
    "the only ",
    "nothing else",
    "no more ",
];
const USAGE_NOUNS: &[&str] = &[
    "caller",
    "call site",
    "callsite",
    "call-site",
    "usage",
    "reference",
    "use site",
    "invocation",
    "occurrence",
    "consumer",
    "dependent",
    "places that call",
    "places it",
    "where it",
    "calls ",
    "everywhere",
];
const HEDGES: &[&str] = &[
    "couldn't",
    "could not",
    "didn't",
    "did not",
    "not all",
    "unable",
    "may not",
    "might not",
    "haven't",
    "have not",
    "not yet",
    "i'll ",
    "i will",
    "let me",
    "next,",
    "todo",
    "?",
    "not sure",
    "unclear",
    "verify that",
    "please check",
];

/// The sentence in `message` that claims a complete set of call sites
/// was handled, if any. Hedged or future-tense sentences don't count.
pub fn find_completeness_claim(message: &str) -> Option<String> {
    for sentence in sentences(message) {
        let lower = sentence.to_lowercase();
        if HEDGES.iter().any(|h| lower.contains(h)) {
            continue;
        }
        let quantified = QUANTIFIERS.iter().any(|q| lower.contains(q));
        let about_usage = USAGE_NOUNS.iter().any(|n| lower.contains(n));
        if quantified && about_usage {
            let trimmed: String = sentence.trim().chars().take(200).collect();
            return Some(trimmed);
        }
    }
    None
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        cur.push(c);
        let next_is_space = chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if matches!(c, '.' | '!') && next_is_space {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Text of the last assistant message in a Claude Code transcript.
pub fn last_assistant_text(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut last = None;
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(content) = v.pointer("/message/content") else {
            continue;
        };
        let text = match content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => items
                .iter()
                .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if !text.trim().is_empty() {
            last = Some(text);
        }
    }
    last
}

// ---------------------------------------------------------------- diff

/// Functions in `file` that the edit removed (or renamed away) or whose
/// signature text changed. Same-named definitions are paired in file order.
pub fn changed_symbols_in(file: &str, old: &str, new: &str) -> Vec<ChangedSymbol> {
    let (old_names, _) = extract_names(file, old);
    let (new_names, _) = extract_names(file, new);
    let old_by = group(&old_names);
    let new_by = group(&new_names);
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut out = Vec::new();
    let mut names: Vec<&String> = old_by.keys().collect();
    names.sort();
    for name in names {
        let olds = &old_by[name];
        let symbol = |kind, old_line| ChangedSymbol {
            file: file.to_string(),
            name: name.clone(),
            kind,
            old_line,
        };
        match new_by.get(name) {
            None => out.extend(olds.iter().map(|&l| symbol(ChangeKind::Removed, l))),
            // Same count on both sides: pair definitions in file order.
            // A differing count means one of several same-named
            // definitions went away; we can't tell which, so skip.
            Some(news) if olds.len() == news.len() => {
                for (&o, &n) in olds.iter().zip(news.iter()) {
                    let a = signature_text(&old_lines, o);
                    let b = signature_text(&new_lines, n);
                    if !a.is_empty() && !b.is_empty() && a != b {
                        out.push(symbol(ChangeKind::Signature, o));
                    }
                }
            }
            Some(_) => {}
        }
    }
    out
}

fn group(names: &[ExtractedName]) -> HashMap<String, Vec<u32>> {
    let mut m: HashMap<String, Vec<u32>> = HashMap::new();
    for n in names {
        m.entry(n.name.clone()).or_default().push(n.start_line);
    }
    for lines in m.values_mut() {
        lines.sort_unstable();
    }
    m
}

/// Whitespace-normalized declaration text from the start line up to the
/// body opener (`{`, or a trailing `:` for Python), at most 8 lines.
fn signature_text(lines: &[&str], start_line: u32) -> String {
    let start = start_line.saturating_sub(1) as usize;
    let mut sig = String::new();
    for line in lines.iter().skip(start).take(8) {
        if let Some(idx) = line.find('{') {
            sig.push_str(&line[..idx]);
            break;
        }
        sig.push_str(line);
        sig.push(' ');
        if line.trim_end().ends_with(':') || line.trim_end().ends_with(';') {
            break;
        }
    }
    sig.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- graph

struct LocatedCaller {
    fqn: String,
    file: String,
    line: u32,
    class: WitnessClass,
}

fn callers_located(conn: &Connection, target_id: &str) -> Result<Vec<LocatedCaller>> {
    let mut stmt = conn.prepare(
        "SELECT n.fqn, n.location, e.provenance FROM edges e
         JOIN nodes n ON n.id = e.from_id
         WHERE e.to_id = ?1 AND json_extract(e.kind, '$.kind') = 'Call'
         ORDER BY n.fqn ASC",
    )?;
    let rows = stmt
        .query_map([target_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(fqn, loc, prov)| {
            let (file, line) = parse_location(&loc);
            LocatedCaller {
                fqn,
                file,
                line,
                class: class_from_provenance(&prov),
            }
        })
        .collect())
}

fn short_name(fqn: &str) -> &str {
    fqn.rsplit([':', '.', '/'])
        .find(|s| !s.is_empty())
        .unwrap_or(fqn)
}

fn relative_to(file: &str, root: &Path) -> String {
    let norm = file.replace('\\', "/");
    let root_s = root.to_string_lossy().replace('\\', "/");
    norm.strip_prefix(&format!("{}/", root_s.trim_end_matches('/')))
        .unwrap_or(&norm)
        .trim_start_matches("./")
        .to_string()
}

fn file_in_set(file: &str, set: &HashSet<String>) -> bool {
    set.contains(file)
        || set
            .iter()
            .any(|c| c.ends_with(&format!("/{file}")) || file.ends_with(&format!("/{c}")))
}

fn contains_word(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

// ---------------------------------------------------------------- git / store

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("run git")?;
    anyhow::ensure!(out.status.success(), "git {:?} failed", args);
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_toplevel(cwd: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        git(cwd, &["rev-parse", "--show-toplevel"])?.trim(),
    ))
}

/// Tracked files changed against HEAD (staged or not) plus untracked
/// files, repo-relative.
fn git_changed_files(root: &Path) -> Result<Vec<String>> {
    let mut files: BTreeSet<String> = BTreeSet::new();
    for line in git(root, &["diff", "--name-only", "HEAD"])?.lines() {
        files.insert(line.trim().to_string());
    }
    for line in git(root, &["ls-files", "--others", "--exclude-standard"])?.lines() {
        files.insert(line.trim().to_string());
    }
    files.remove("");
    Ok(files.into_iter().collect())
}

/// `git grep -w` over tracked and untracked (not ignored) files.
fn git_grep_word(root: &Path, word: &str, kind: ChangeKind) -> Vec<TextRef> {
    let Ok(out) = git(
        root,
        &["grep", "-n", "-w", "-I", "-F", "--untracked", "-e", word],
    ) else {
        // Exit 1 means no matches.
        return Vec::new();
    };
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, ':');
            let file = parts.next()?.to_string();
            let line = parts.next()?.parse().ok()?;
            Some(TextRef {
                name: word.to_string(),
                kind,
                file,
                line,
                snippet: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

fn git_show_head(root: &Path, file: &str) -> Option<String> {
    git(root, &["show", &format!("HEAD:{file}")]).ok()
}

fn artifact_for(store: &ProjectStore, repo_root: &Path) -> Option<(PathBuf, String)> {
    let mut cur = repo_root.to_path_buf();
    loop {
        if let Ok(Some(project)) = store.project_for_root_path(&cur) {
            let scan = project.latest_scan?;
            let artifact = scan.graph_artifact_path?;
            return Some((PathBuf::from(artifact), scan.id));
        }
        if !cur.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn claim_detection_hits_completion_language() {
        for msg in [
            "Renamed the function. I updated all callers to the new name.",
            "Done: every call site now passes the context.",
            "All usages of ParseConfig have been migrated.",
            "There are no other callers of this method.",
            "The only caller was main, which I fixed.",
        ] {
            assert!(find_completeness_claim(msg).is_some(), "{msg}");
        }
    }

    #[test]
    fn claim_detection_skips_hedges_and_plans() {
        for msg in [
            "Let me find all callers first.",
            "I couldn't update all callers; two are in generated code.",
            "Should I update every call site?",
            "Fixed the bug in the parser.",
            "I'll update all usages next.",
        ] {
            assert!(find_completeness_claim(msg).is_none(), "{msg}");
        }
    }

    #[test]
    fn transcript_last_assistant_text() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("t.jsonl");
        std::fs::write(
            &p,
            [
                r#"{"type":"user","message":{"role":"user","content":"rename it"}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking."}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{}}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Updated all callers."}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        assert_eq!(
            last_assistant_text(&p).as_deref(),
            Some("Updated all callers.")
        );
    }

    #[test]
    fn diff_finds_removed_and_resignatured() {
        let old = "package chi\n\nfunc NewRouter() *Mux {\n\treturn nil\n}\n\nfunc Use(a int) {\n}\n\nfunc Keep() {\n}\n";
        let new = "package chi\n\nfunc Use(a int, b int) {\n}\n\nfunc Keep() {\n}\n";
        let got = changed_symbols_in("chi.go", old, new);
        assert!(got.contains(&ChangedSymbol {
            file: "chi.go".into(),
            name: "NewRouter".into(),
            kind: ChangeKind::Removed,
            old_line: 3,
        }));
        assert!(got.contains(&ChangedSymbol {
            file: "chi.go".into(),
            name: "Use".into(),
            kind: ChangeKind::Signature,
            old_line: 7,
        }));
        assert!(!got.iter().any(|s| s.name == "Keep"));
    }

    #[test]
    fn python_signature_change_and_removal() {
        let old = "class App:\n    def dispatch(self, req):\n        pass\n\n    @property\n    def name(self):\n        return 1\n\n\ndef make_app(config):\n    return App()\n";
        let new = "class App:\n    def dispatch(self, req, ctx):\n        pass\n\n    @property\n    def name(self):\n        return 1\n\n\ndef build_app(config):\n    return App()\n";
        let got = changed_symbols_in("app.py", old, new);
        assert_eq!(
            got,
            vec![
                ChangedSymbol {
                    file: "app.py".into(),
                    name: "dispatch".into(),
                    kind: ChangeKind::Signature,
                    old_line: 2,
                },
                ChangedSymbol {
                    file: "app.py".into(),
                    name: "make_app".into(),
                    kind: ChangeKind::Removed,
                    old_line: 10,
                },
            ]
        );
    }

    /// Python end to end: `make_app` renamed, `wsgi.py` still calls it.
    #[test]
    fn python_rename_blocks_on_untouched_caller() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        sh(&root, &["init", "-q"]);
        sh(&root, &["config", "user.email", "t@example.com"]);
        sh(&root, &["config", "user.name", "t"]);
        std::fs::write(
            root.join("app.py"),
            "def make_app(config):\n    return config\n",
        )
        .unwrap();
        std::fs::write(
            root.join("wsgi.py"),
            "from app import make_app\n\ndef main():\n    return make_app({})\n",
        )
        .unwrap();
        std::fs::write(
            root.join("cli.py"),
            "from app import make_app\n\ndef run():\n    return make_app({})\n",
        )
        .unwrap();
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-qm", "init"]);

        let artifact = dir.path().join("graph.sqlite");
        let conn = Connection::open(&artifact).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT);
            CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{}', PRIMARY KEY (from_id, to_id, kind));
            CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO metadata VALUES ('language', 'python');
            INSERT INTO nodes (id, kind, fqn, location) VALUES
              ('mk', 'Function', 'app.make_app', '{"file":"app.py","start_line":1}'),
              ('main', 'Function', 'wsgi.main', '{"file":"wsgi.py","start_line":3}'),
              ('run', 'Function', 'cli.run', '{"file":"cli.py","start_line":3}');
            INSERT INTO edges VALUES
              ('main', 'mk', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}'),
              ('run', 'mk', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}');
            "#,
        )
        .unwrap();

        std::fs::write(
            root.join("app.py"),
            "def build_app(config):\n    return config\n",
        )
        .unwrap();
        std::fs::write(
            root.join("cli.py"),
            "from app import build_app\n\ndef run():\n    return build_app({})\n",
        )
        .unwrap();
        let out = evaluate(&StopInputs {
            message: "Renamed make_app to build_app and updated every caller.".into(),
            repo_root: root,
            artifact,
            scan_id: "fixture".into(),
            stop_hook_active: false,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert_eq!(v["decision"], "block", "{}", out.host_stdout);
        let reason = v["reason"].as_str().unwrap();
        assert!(reason.contains("wsgi.main (wsgi.py:3)"), "{reason}");
        assert!(!reason.contains("cli.run"), "{reason}");
    }

    #[test]
    fn import_only_mentions() {
        let py = "from .helpers import send_from_directory\nfrom .x import (\n    a,\n    send_from_directory,\n)\n\nx = 1\n";
        assert!(only_import_mentions(py, "send_from_directory"));
        let used = "from .helpers import send_from_directory\n\nsend_from_directory('a', 'b')\n";
        assert!(!only_import_mentions(used, "send_from_directory"));
        let ts = "import {\n  parse,\n  other,\n} from './p'\n";
        assert!(only_import_mentions(ts, "parse"));
        assert!(!only_import_mentions("x = 1\n", "parse"));
    }

    /// Flask: a module-level import is not a caller that needs updating
    /// when the signature changes, and an untouched test that the graph
    /// couldn't resolve still shows up as a text match.
    #[test]
    fn python_signature_skips_imports_and_warns_on_unresolved_use() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("tests")).unwrap();
        sh(&root, &["init", "-q"]);
        sh(&root, &["config", "user.email", "t@example.com"]);
        sh(&root, &["config", "user.name", "t"]);
        std::fs::write(
            root.join("helpers.py"),
            "def send_from_directory(directory, path):\n    return path\n",
        )
        .unwrap();
        std::fs::write(
            root.join("blueprints.py"),
            "from .helpers import send_from_directory\n\ndef send_static(f):\n    return send_from_directory('s', f)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("__init__.py"),
            "from .helpers import send_from_directory as send_from_directory\n",
        )
        .unwrap();
        std::fs::write(
            root.join("tests/test_helpers.py"),
            "import flask\n\ndef test_send(app):\n    flask.send_from_directory('static', 'a.txt')\n",
        )
        .unwrap();
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-qm", "init"]);

        let artifact = dir.path().join("graph.sqlite");
        let conn = Connection::open(&artifact).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT);
            CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{}', PRIMARY KEY (from_id, to_id, kind));
            CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO metadata VALUES ('language', 'python');
            INSERT INTO nodes (id, kind, fqn, location) VALUES
              ('sfd', 'Function', 'flask::helpers::send_from_directory', '{"file":"helpers.py","start_line":1}'),
              ('bpm', 'Function', 'flask::blueprints::__file_module__', '{"file":"blueprints.py","start_line":1}'),
              ('ss', 'Function', 'flask::blueprints::send_static', '{"file":"blueprints.py","start_line":3}'),
              ('initm', 'Function', 'flask::__init__::__file_module__', '{"file":"__init__.py","start_line":1}');
            INSERT INTO edges VALUES
              ('bpm', 'sfd', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}'),
              ('ss', 'sfd', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}'),
              ('initm', 'sfd', '{"kind":"Call"}', '{"source":"Compiler","confidence":"High"}');
            "#,
        )
        .unwrap();

        std::fs::write(
            root.join("helpers.py"),
            "def send_from_directory(directory, path, etag):\n    return path\n",
        )
        .unwrap();
        let out = evaluate(&StopInputs {
            message: "Added etag to send_from_directory and updated all callers.".into(),
            repo_root: root,
            artifact,
            scan_id: "fixture".into(),
            stop_hook_active: false,
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert_eq!(v["decision"], "block", "{}", out.host_stdout);
        let reason = v["reason"].as_str().unwrap();
        assert!(
            reason.contains("flask::blueprints::send_static "),
            "{reason}"
        );
        assert!(!reason.contains("__file_module__"), "{reason}");
        assert!(
            reason.contains("tests/test_helpers.py:4 uses `send_from_directory`"),
            "{reason}"
        );
        assert!(!reason.contains("__init__.py"), "{reason}");
    }

    /// chi's context.go: `func URLParam` and `func (x *Context) URLParam`.
    /// Changing only the function must be seen, and only the function.
    #[test]
    fn same_named_func_and_method_pair_by_position() {
        let old = "package chi\n\nfunc URLParam(r *http.Request, key string) string {\n\treturn \"\"\n}\n\nfunc (x *Context) URLParam(key string) string {\n\treturn \"\"\n}\n";
        let new = "package chi\n\nfunc URLParam(r *http.Request, key string, fallback string) string {\n\treturn \"\"\n}\n\nfunc (x *Context) URLParam(key string) string {\n\treturn \"\"\n}\n";
        let got = changed_symbols_in("context.go", old, new);
        assert_eq!(
            got,
            vec![ChangedSymbol {
                file: "context.go".into(),
                name: "URLParam".into(),
                kind: ChangeKind::Signature,
                old_line: 3,
            }]
        );
    }

    #[test]
    fn word_boundaries() {
        assert!(contains_word("x := NewRouter()", "NewRouter"));
        assert!(!contains_word("x := NewRouterV2()", "NewRouter"));
        assert!(!contains_word("myNewRouter()", "NewRouter"));
    }

    // ---- end to end over a real git repo + synthetic graph

    fn sh(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// Repo: chi.go defines NewRouter; app.go and other.go call it.
    /// Graph: app.go caller is compiler-witnessed, other.go tree-sitter.
    fn fixture(other_class: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        sh(&root, &["init", "-q"]);
        sh(&root, &["config", "user.email", "t@example.com"]);
        sh(&root, &["config", "user.name", "t"]);
        std::fs::write(
            root.join("chi.go"),
            "package chi\n\nfunc NewRouter() *Mux {\n\treturn nil\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("app.go"),
            "package chi\n\nfunc wire() {\n\tNewRouter()\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("other.go"),
            "package chi\n\nfunc boot() {\n\tNewRouter()\n}\n",
        )
        .unwrap();
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-qm", "init"]);

        let artifact = dir.path().join("graph.sqlite");
        let conn = Connection::open(&artifact).unwrap();
        conn.execute_batch(&format!(
            r#"
            CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{{}}', provenance TEXT NOT NULL DEFAULT '{{}}',
                properties TEXT NOT NULL DEFAULT '{{}}', trait_metadata TEXT);
            CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{{}}', PRIMARY KEY (from_id, to_id, kind));
            CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO metadata VALUES ('language', 'go');
            INSERT INTO nodes (id, kind, fqn, location) VALUES
              ('nr', 'Function', 'chi::NewRouter', '{{"file":"chi.go","start_line":3}}'),
              ('wire', 'Function', 'chi::wire', '{{"file":"app.go","start_line":3}}'),
              ('boot', 'Function', 'chi::boot', '{{"file":"other.go","start_line":3}}');
            INSERT INTO edges VALUES
              ('wire', 'nr', '{{"kind":"Call"}}', '{{"source":"Compiler","confidence":"High"}}'),
              ('boot', 'nr', '{{"kind":"Call"}}', '{{"source":"{other_class}","confidence":"Medium"}}');
            "#
        ))
        .unwrap();
        (dir, artifact)
    }

    fn inputs(dir: &TempDir, artifact: &Path, msg: &str, active: bool) -> StopInputs {
        StopInputs {
            message: msg.into(),
            repo_root: dir.path().join("repo"),
            artifact: artifact.to_path_buf(),
            scan_id: "fixture".into(),
            stop_hook_active: active,
        }
    }

    fn rename(root: &Path, update_app: bool, update_other: bool) {
        std::fs::write(
            root.join("chi.go"),
            "package chi\n\nfunc NewMux() *Mux {\n\treturn nil\n}\n",
        )
        .unwrap();
        if update_app {
            std::fs::write(
                root.join("app.go"),
                "package chi\n\nfunc wire() {\n\tNewMux()\n}\n",
            )
            .unwrap();
        }
        if update_other {
            std::fs::write(
                root.join("other.go"),
                "package chi\n\nfunc boot() {\n\tNewMux()\n}\n",
            )
            .unwrap();
        }
    }

    const CLAIM: &str = "Renamed NewRouter to NewMux and updated all callers.";

    #[test]
    fn blocks_when_compiler_caller_untouched() {
        let (dir, artifact) = fixture("Compiler");
        rename(&dir.path().join("repo"), false, true);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert_eq!(v["decision"], "block", "{}", out.host_stdout);
        let reason = v["reason"].as_str().unwrap();
        assert!(reason.contains("chi::wire"), "{reason}");
        assert!(reason.contains("app.go"), "{reason}");
        assert!(!reason.contains("chi::boot"), "{reason}");
        assert_eq!(out.decision.unwrap().permission, Permission::Deny);
    }

    #[test]
    fn blocks_when_touched_file_still_references_old_name() {
        let (dir, artifact) = fixture("Compiler");
        let root = dir.path().join("repo");
        rename(&root, true, true);
        // Touch app.go but leave a stale reference behind.
        std::fs::write(
            root.join("app.go"),
            "package chi\n\nfunc wire() {\n\tNewMux()\n\tNewRouter()\n}\n",
        )
        .unwrap();
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert_eq!(v["decision"], "block", "{}", out.host_stdout);
        assert!(v["reason"]
            .as_str()
            .unwrap()
            .contains("still references `NewRouter`"));
    }

    /// The graph has no edge from `_examples/` (Go tooling skips `_`
    /// dirs), so a stale call there only shows up as a text match.
    #[test]
    fn text_fallback_reports_reference_outside_graph() {
        let (dir, artifact) = fixture("Compiler");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("_examples")).unwrap();
        std::fs::write(
            root.join("_examples/main.go"),
            "package main\n\n// NewRouter is the old name\nfunc main() {\n\tchi.NewRouter()\n}\n",
        )
        .unwrap();
        sh(&root, &["add", "."]);
        sh(&root, &["commit", "-qm", "example"]);
        rename(&root, true, true);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert!(v.get("decision").is_none(), "{}", out.host_stdout);
        let msg = v["systemMessage"].as_str().unwrap();
        assert!(msg.contains("text matches only"), "{msg}");
        assert!(msg.contains("_examples/main.go:5"), "{msg}");
        assert!(!msg.contains(":3 "), "comment line reported: {msg}");
        assert_eq!(out.decision.unwrap().tier, "text");
    }

    #[test]
    fn text_matches_ride_along_on_a_block() {
        let (dir, artifact) = fixture("Compiler");
        let root = dir.path().join("repo");
        std::fs::write(root.join("extra.go"), "package chi\n\nvar f = NewRouter\n").unwrap();
        rename(&root, false, true);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert_eq!(v["decision"], "block", "{}", out.host_stdout);
        let reason = v["reason"].as_str().unwrap();
        assert!(reason.contains("extra.go:3"), "{reason}");
        // app.go is already reported by the graph; not repeated as text.
        assert_eq!(reason.matches("app.go").count(), 1, "{reason}");
    }

    #[test]
    fn text_fallback_skips_names_still_defined_elsewhere() {
        let (dir, artifact) = fixture("Compiler");
        let conn = Connection::open(&artifact).unwrap();
        conn.execute(
            "INSERT INTO nodes (id, kind, fqn, location) VALUES ('nr2', 'Method', 'other::T.NewRouter', '{\"file\":\"other/t.go\",\"start_line\":1}')",
            [],
        )
        .unwrap();
        let root = dir.path().join("repo");
        std::fs::write(root.join("extra.go"), "package chi\n\nvar f = NewRouter\n").unwrap();
        rename(&root, true, true);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        assert_eq!(out.host_stdout.trim(), "{}", "{}", out.host_stdout);
    }

    #[test]
    fn quiet_when_every_caller_updated() {
        let (dir, artifact) = fixture("Compiler");
        rename(&dir.path().join("repo"), true, true);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        assert_eq!(out.host_stdout.trim(), "{}");
        assert_eq!(out.decision.unwrap().permission, Permission::Allow);
    }

    #[test]
    fn treesitter_only_miss_warns_without_blocking() {
        let (dir, artifact) = fixture("TreeSitter");
        rename(&dir.path().join("repo"), true, false);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert!(v.get("decision").is_none(), "{}", out.host_stdout);
        assert!(v["systemMessage"].as_str().unwrap().contains("chi::boot"));
    }

    #[test]
    fn never_blocks_twice() {
        let (dir, artifact) = fixture("Compiler");
        rename(&dir.path().join("repo"), false, true);
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, true)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.host_stdout).unwrap();
        assert!(v.get("decision").is_none(), "{}", out.host_stdout);
        assert!(v["systemMessage"]
            .as_str()
            .unwrap()
            .contains("not blocking again"));
    }

    #[test]
    fn no_claim_no_work() {
        let (dir, artifact) = fixture("Compiler");
        rename(&dir.path().join("repo"), false, false);
        let out = evaluate(&inputs(&dir, &artifact, "Renamed the constructor.", false)).unwrap();
        assert_eq!(out.host_stdout.trim(), "{}");
        assert!(out.decision.is_none());
    }

    #[test]
    fn moved_function_is_not_a_removal() {
        let (dir, artifact) = fixture("Compiler");
        let root = dir.path().join("repo");
        std::fs::write(root.join("chi.go"), "package chi\n").unwrap();
        std::fs::write(
            root.join("router.go"),
            "package chi\n\nfunc NewRouter() *Mux {\n\treturn nil\n}\n",
        )
        .unwrap();
        let out = evaluate(&inputs(&dir, &artifact, CLAIM, false)).unwrap();
        assert_eq!(out.host_stdout.trim(), "{}", "{}", out.host_stdout);
    }

    #[test]
    fn host_payload_without_claim_is_quiet() {
        let dir = TempDir::new().unwrap();
        let store = ProjectStore::open(
            dir.path().join("projects.sqlite"),
            dir.path().join("reports"),
            dir.path().join("graphs"),
        )
        .unwrap();
        let raw = r#"{"hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"Fixed the typo."}"#;
        assert_eq!(run_from_host(&store, raw, None).host_stdout.trim(), "{}");
        assert_eq!(
            run_from_host(&store, "not json", None).host_stdout.trim(),
            "{}"
        );
    }
}
