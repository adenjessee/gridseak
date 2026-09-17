//! Verdict: Compiler/LSP callers → deny; tree-sitter/heuristic → ask;
//! none/unknown → allow; stale ∩ dirty → ask. Wrong-twin under
//! `deno/` or `_examples/` → deny with the canonical file.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::gate::artifact::{
    callers_with_tier, find_symbols, list_same_short_name, CallerHit, SymbolHit, WitnessClass,
};
use crate::gate::parse_edit::PendingEdit;
use crate::graph_queries::agent_tools::{verify_claim, ClaimVerdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Deny,
    Ask,
    Allow,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolVerdict {
    pub fqn: String,
    pub claim: String,
    pub claim_verdict: String,
    pub witnesses: Vec<String>,
    pub witness_class: String,
    pub permission: Permission,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub permission: Permission,
    pub reason: String,
    pub scan_id: String,
    pub symbols: Vec<SymbolVerdict>,
    pub tier: String,
    pub witnesses: Vec<String>,
    pub override_used: bool,
    pub host: String,
    pub file: String,
    pub duration_ms: u64,
    pub extract_source: String,
}

pub struct Request {
    pub edit: PendingEdit,
    pub artifact: PathBuf,
    pub scan_id: String,
    pub stale: bool,
    pub dirty_paths: Vec<String>,
    pub extra_symbols: Vec<String>,
}

pub fn decide(req: &Request) -> Result<Decision> {
    let file = req.edit.file.as_str();
    if req.stale && path_is_dirty(file, &req.dirty_paths) {
        return Ok(Decision {
            permission: Permission::Ask,
            reason: format!(
                "scan is stale and dirty path intersects `{file}` — run `gridseak scan` before allowing this edit"
            ),
            scan_id: req.scan_id.clone(),
            symbols: Vec::new(),
            tier: "stale".into(),
            witnesses: Vec::new(),
            override_used: false,
            host: String::new(),
            file: file.to_string(),
            duration_ms: 0,
            extract_source: req.edit.extract_source.clone(),
        });
    }

    if let Some(mut twin) = wrong_twin(&req.artifact, file, &req.edit, &req.scan_id)? {
        twin.scan_id = req.scan_id.clone();
        return Ok(twin);
    }

    if let Some(mut preferred) =
        crate::gate::preferred::same_name_preferred(&req.artifact, file, &req.edit, &req.scan_id)?
    {
        preferred.scan_id = req.scan_id.clone();
        return Ok(preferred);
    }

    let mut needles = req.edit.removed_names.clone();
    needles.extend(req.extra_symbols.iter().cloned());
    let hits = find_symbols(
        &req.artifact,
        file,
        &needles,
        &req.edit.deleted_ranges,
        req.edit.whole_file_delete,
    )?;

    if hits.is_empty() && needles.is_empty() {
        return Ok(allow(
            &req.scan_id,
            "no function/method symbols removed or renamed",
        ));
    }

    let mut rows = Vec::new();
    let mut worst = Permission::Allow;
    let mut worst_tier = "none".to_string();
    let mut all_witnesses = Vec::new();

    let lookup: Vec<SymbolHit> = if hits.is_empty() {
        needles
            .iter()
            .map(|n| SymbolHit {
                id: String::new(),
                fqn: n.clone(),
                file: file.to_string(),
                start_line: 0,
            })
            .collect()
    } else {
        hits
    };

    for hit in lookup {
        let claim = format!("no_callers({})", hit.fqn);
        let view = verify_claim(&req.artifact, &claim)?;
        let callers = if hit.id.is_empty() {
            Vec::new()
        } else {
            callers_with_tier(&req.artifact, &hit.id).unwrap_or_default()
        };
        let class = worst_class(&callers);
        let permission = match view.verdict {
            ClaimVerdict::Refuted => match class {
                WitnessClass::Compiler => Permission::Deny,
                WitnessClass::TreeSitter => Permission::Ask,
                WitnessClass::None => Permission::Allow,
            },
            ClaimVerdict::Verified | ClaimVerdict::Unknown => Permission::Allow,
        };
        let class_label = class.as_str().to_string();
        if rank(permission) > rank(worst) {
            worst = permission;
            worst_tier = class_label.clone();
        }
        all_witnesses.extend(view.witnesses.iter().cloned());
        rows.push(SymbolVerdict {
            fqn: hit.fqn,
            claim,
            claim_verdict: format!("{:?}", view.verdict).to_lowercase(),
            witnesses: view.witnesses,
            witness_class: class_label,
            permission,
        });
    }

    let reason = match worst {
        Permission::Deny => format_deny(&rows),
        Permission::Ask => {
            "callers exist but only TreeSitter/Heuristic witnesses — ask the user".into()
        }
        Permission::Allow => "no compiler/LSP callers for removed symbols".into(),
    };

    Ok(Decision {
        permission: worst,
        reason,
        scan_id: req.scan_id.clone(),
        symbols: rows,
        tier: worst_tier,
        witnesses: all_witnesses.into_iter().take(16).collect(),
        override_used: false,
        host: String::new(),
        file: file.to_string(),
        duration_ms: 0,
        extract_source: req.edit.extract_source.clone(),
    })
}

fn wrong_twin(
    artifact: &Path,
    file: &str,
    edit: &PendingEdit,
    scan_id: &str,
) -> Result<Option<Decision>> {
    if !is_republish_path(file) {
        return Ok(None);
    }
    let names = if edit.removed_names.is_empty() {
        // Editing a twin file as if it were canonical — use any
        // function name that appears in the new or old text.
        crate::gate::parse_edit::function_names_in(&edit.old_text)
    } else {
        edit.removed_names.clone()
    };
    for name in names {
        let short = short_name(&name);
        if let Some(canonical) = list_same_short_name(artifact, &short)?
            .into_iter()
            .find(|n| !is_republish_path(&n.file))
        {
            return Ok(Some(Decision {
                permission: Permission::Deny,
                reason: format!(
                    "wrong-twin edit: `{file}` looks like a republish; canonical `{short}` lives in `{}`",
                    canonical.file
                ),
                scan_id: scan_id.to_string(),
                symbols: vec![SymbolVerdict {
                    fqn: canonical.fqn.clone(),
                    claim: format!("twin({short})"),
                    claim_verdict: "refuted".into(),
                    witnesses: vec![canonical.file.clone()],
                    witness_class: "compiler".into(),
                    permission: Permission::Deny,
                }],
                tier: "twin".into(),
                witnesses: vec![canonical.file],
                override_used: false,
                host: String::new(),
                file: file.to_string(),
                duration_ms: 0,
                extract_source: edit.extract_source.clone(),
            }));
        }
    }
    Ok(None)
}

pub fn is_republish_path(file: &str) -> bool {
    let n = file.replace('\\', "/");
    n.contains("/deno/")
        || n.starts_with("deno/")
        || n.contains("/_examples/")
        || n.contains("_examples/")
}

fn short_name(fqn: &str) -> String {
    fqn.rsplit([':', '.', '/'])
        .find(|s| !s.is_empty())
        .unwrap_or(fqn)
        .to_string()
}

fn path_is_dirty(file: &str, dirty: &[String]) -> bool {
    if file.is_empty() {
        return !dirty.is_empty();
    }
    let norm = file.replace('\\', "/");
    dirty.iter().any(|d| {
        let d = d.replace('\\', "/");
        norm.ends_with(&d) || d.ends_with(&norm) || norm.contains(&d) || d.contains(&norm)
    })
}

fn worst_class(hits: &[CallerHit]) -> WitnessClass {
    if hits
        .iter()
        .any(|h| matches!(h.class, WitnessClass::Compiler))
    {
        return WitnessClass::Compiler;
    }
    if hits
        .iter()
        .any(|h| matches!(h.class, WitnessClass::TreeSitter))
    {
        return WitnessClass::TreeSitter;
    }
    WitnessClass::None
}

fn format_deny(rows: &[SymbolVerdict]) -> String {
    let mut parts = Vec::new();
    for row in rows
        .iter()
        .filter(|r| r.permission == Permission::Deny)
        .take(5)
    {
        let w = if row.witnesses.is_empty() {
            String::new()
        } else {
            format!(
                " (callers: {})",
                row.witnesses
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        parts.push(format!("{} [{}]{w}", row.fqn, row.witness_class));
    }
    format!(
        "deny: compiler/LSP callers exist for removed symbols: {}",
        parts.join("; ")
    )
}

pub fn allow_unscanned(reason: &str, file: &str, extract_source: &str) -> Decision {
    Decision {
        permission: Permission::Allow,
        reason: reason.into(),
        scan_id: String::new(),
        symbols: Vec::new(),
        tier: "none".into(),
        witnesses: Vec::new(),
        override_used: false,
        host: String::new(),
        file: file.into(),
        duration_ms: 0,
        extract_source: extract_source.into(),
    }
}

pub fn ask_unresolved(reason: &str, file: &str, extract_source: &str) -> Decision {
    Decision {
        permission: Permission::Ask,
        reason: reason.into(),
        scan_id: String::new(),
        symbols: Vec::new(),
        tier: "unresolved".into(),
        witnesses: Vec::new(),
        override_used: false,
        host: String::new(),
        file: file.into(),
        duration_ms: 0,
        extract_source: extract_source.into(),
    }
}

fn allow(scan_id: &str, reason: &str) -> Decision {
    Decision {
        permission: Permission::Allow,
        reason: reason.into(),
        scan_id: scan_id.into(),
        symbols: Vec::new(),
        tier: "none".into(),
        witnesses: Vec::new(),
        override_used: false,
        host: String::new(),
        file: String::new(),
        duration_ms: 0,
        extract_source: String::new(),
    }
}

fn rank(p: Permission) -> u8 {
    match p {
        Permission::Allow => 0,
        Permission::Ask => 1,
        Permission::Deny => 2,
    }
}
