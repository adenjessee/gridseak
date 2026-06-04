//! Shared Apex class-symbols JSON codec.
//!
//! Both Apex post-syntax stages (`vf_extraction_stage`,
//! `framework_entry_point_stage`) consume
//! `syntax_results.class_symbols: &[(String, String)]` — an `(api_name,
//! json)` payload populated by the Apex tree-sitter pass — and need
//! to deserialise it into a `BTreeMap<String, ApexClassSymbols>`
//! registry before walking it. Before this module existed, each
//! stage carried a byte-identical copy of the deserialise helper
//! (tracked as UF-FU-001 in
//! `docs/workstreams/universal-fidelity/FOLLOWUPS.md`). Extracting
//! the helper here collapses the duplication into one place with one
//! test.
//!
//! # Log-context parameter
//!
//! The two original copies differed only in the tracing prefix used
//! for the "malformed row dropped" warning — one said `"VF extraction:
//! ..."`, the other `"framework entry-point propagation: ..."`. That
//! observable log shape is load-bearing: the T5 integration tests
//! (`t5_orchestrator_language_agnosticism`) assert on the substring
//! the Apex post-syntax hook formats for the orchestrator. Exposing
//! the prefix as a `log_context: &str` parameter preserves that
//! shape; each caller passes its stage name and downstream log
//! consumers still see the stage-disambiguated warning.
//!
//! # Fail-open semantics
//!
//! Rows whose JSON fails to parse are dropped with a `warn!` and
//! counted as "malformed". The VF resolver never sees a partial
//! `ApexClassSymbols` struct. No row-level failure aborts the stage
//! — the parse continues with whatever class_symbols did deserialise.

use std::collections::BTreeMap;

use tracing::warn;

use crate::domain::apex::class_symbols::ApexClassSymbols;

/// Deserialise the `(api_name, json)` payload populated by the Apex
/// tree-sitter pass into a case-preserving `BTreeMap` keyed by the
/// class's dotted api-name.
///
/// Rows whose JSON fails to parse are skipped with a warning prefixed
/// by `log_context` — each caller passes its stage name so the
/// orchestrator-level log stream remains disambiguated. The shape of
/// the warning message is part of the observable contract;
/// downstream filters (CI grep, log-assertion tests) rely on the
/// `"<log_context>: dropping malformed class_symbols row for
/// `<api_name>`: <err>"` format.
pub fn deserialise_class_symbols(
    raw: &[(String, String)],
    log_context: &str,
) -> BTreeMap<String, ApexClassSymbols> {
    let mut out = BTreeMap::new();
    for (api_name, json) in raw {
        match serde_json::from_str::<ApexClassSymbols>(json) {
            Ok(symbols) => {
                out.insert(api_name.clone(), symbols);
            }
            Err(e) => {
                warn!("{log_context}: dropping malformed class_symbols row for `{api_name}`: {e}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{
        Access, ApexClassSymbols, ApexMethod, ApexParameter, ApexTypeRef,
    };

    fn apex_primitive(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive { name: name.into() }
    }

    fn symbols_with_one_method(method_name: &str) -> ApexClassSymbols {
        ApexClassSymbols {
            methods: vec![ApexMethod {
                name: method_name.into(),
                parameters: vec![ApexParameter {
                    name: "x".into(),
                    ty: apex_primitive("String"),
                }],
                return_type: None,
                access: Access::Public,
                is_static: false,
                is_virtual: false,
                is_abstract: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn deserialise_class_symbols_tolerates_malformed_rows() {
        // Relocated from `vf_extraction_stage.rs` as part of UF-FU-001.
        // Behaviour unchanged: good rows land in the map, malformed
        // rows are skipped with a warning (observable via `tracing`
        // subscribers).
        let good_json = serde_json::to_string(&symbols_with_one_method("save")).unwrap();
        let raw = vec![
            ("GoodClass".into(), good_json),
            ("BadClass".into(), "not json".into()),
        ];
        let out = deserialise_class_symbols(&raw, "vf extraction");
        assert_eq!(out.len(), 1);
        assert!(out.contains_key("GoodClass"));
        assert!(!out.contains_key("BadClass"));
    }

    #[test]
    fn deserialise_class_symbols_is_empty_on_empty_input() {
        let out = deserialise_class_symbols(&[], "vf extraction");
        assert!(out.is_empty());
    }
}
