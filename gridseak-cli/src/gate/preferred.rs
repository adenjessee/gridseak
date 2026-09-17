//! Same-name preferred path.
//!
//! Republish twins (`deno/`, `_examples/`) are handled in `wrong_twin`.
//! This covers the Go interface / implementation miss: renaming
//! `Router.Use` in `chi.go` while `Mux.Use` in `mux.go` still has
//! compiler callers.

use std::path::Path;

use anyhow::Result;

use super::artifact::{callers_with_tier, list_same_short_name, WitnessClass};
use super::decision::{Decision, Permission, SymbolVerdict};
use super::parse_edit::PendingEdit;

pub fn same_name_preferred(
    artifact: &Path,
    file: &str,
    edit: &PendingEdit,
    scan_id: &str,
) -> Result<Option<Decision>> {
    if edit.removed_names.is_empty() {
        return Ok(None);
    }
    let edit_norm = file.replace('\\', "/");
    for name in &edit.removed_names {
        let short = name
            .rsplit([':', '.', '/'])
            .find(|s| !s.is_empty())
            .unwrap_or(name);
        for hit in list_same_short_name(artifact, short)? {
            let hit_norm = hit.file.replace('\\', "/");
            if same_path(&edit_norm, &hit_norm) || hit.id.is_empty() {
                continue;
            }
            let callers = callers_with_tier(artifact, &hit.id).unwrap_or_default();
            if !callers
                .iter()
                .any(|c| matches!(c.class, WitnessClass::Compiler))
            {
                continue;
            }
            return Ok(Some(Decision {
                permission: Permission::Deny,
                reason: format!(
                    "wrong-twin edit: `{file}` removes `{short}` but the same name with compiler callers lives in `{}`",
                    hit.file
                ),
                scan_id: scan_id.to_string(),
                symbols: vec![SymbolVerdict {
                    fqn: hit.fqn.clone(),
                    claim: format!("twin({short})"),
                    claim_verdict: "refuted".into(),
                    witnesses: vec![hit.file.clone()],
                    witness_class: "compiler".into(),
                    permission: Permission::Deny,
                }],
                tier: "twin".into(),
                witnesses: vec![hit.file],
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

fn same_path(a: &str, b: &str) -> bool {
    a == b || a.ends_with(b) || b.ends_with(a)
}
