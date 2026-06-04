//! Visualforce `.page` reader (TR-A.5).
//!
//! # Scope
//!
//! Attribute-only. This module extracts, from a single `.page` file:
//!
//!   1. The `controller="X"` attribute on `<apex:page>`.
//!   2. The `extensions="A, B, C"` attribute on `<apex:page>` — split and
//!      trimmed so the resolver sees the list in declared order.
//!   3. Every `{!identifier}` / `{!identifier(...)}` binding that appears
//!      **inside an attribute value** anywhere in the document.
//!
//! Text-node VF expressions (`<p>{!foo}</p>`), `rerender` targets, and
//! `<apex:actionFunction>` nested elements are **out of scope**. They are
//! Phase C TR-C.3 territory, and mixing them in here would leak scope
//! beyond the "Visualforce extensions binding" contract §7 of
//! `PHASE_A_EXECUTION_PLAN.md`.
//!
//! # Why quick-xml and not tree-sitter
//!
//! Visualforce is not registered as a tree-sitter grammar in this engine
//! (`LanguageConfig` carries a `queries` / `kind_mappings` contract that
//! a `.page` file cannot meaningfully satisfy — see the header of
//! `configs/visualforce.yaml`). The `quick-xml` crate is already a
//! dependency of `graphengine-parsing` via the Apex metadata readers;
//! TR-A.5 adds zero new crates.
//!
//! # Why XML-parse the entire document instead of regex-scanning
//!
//! VF attribute values carry `{!...}` expressions that are only correctly
//! scoped inside XML attributes. A text regex would happily match `{!foo}`
//! inside a comment or a CDATA block. Using quick-xml gives us the XML
//! event stream (`Event::Start`, `Event::Empty`) so every attribute we
//! inspect is guaranteed to be an actual attribute on an element, not a
//! literal fragment inside free text. Attribute *values* are still
//! scanned with a regex — that scan is narrowly bounded to one attribute
//! value at a time, so the comment/CDATA risk does not re-surface.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Deserialize;

/// Discovery-layer config for Visualforce. Loaded from
/// `configs/visualforce.yaml` by the VF pass — NOT by
/// `infrastructure::config::load_config` (which enforces the tree-sitter
/// `LanguageConfig` schema and would reject this file).
#[derive(Debug, Clone, Deserialize)]
pub struct VfConfig {
    /// The `.page` suffix, listed exactly as it appears in the YAML so
    /// downstream logs and diagnostics speak in the same vocabulary the
    /// user's config does.
    pub file_extensions: Vec<String>,
}

/// A single `{!identifier}` / `{!identifier(...)}` binding surfaced from
/// an attribute value. `is_invocation = true` when followed by `(`;
/// non-invocations (`{!foo.bar}`, plain property accessors) are recorded
/// with `is_invocation = false` so the resolver can drop them at lookup
/// time without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfBinding {
    /// Top-level identifier — everything before the first `.` or `(` in
    /// the binding expression. `{!someCtrl.doSave}` yields `someCtrl`
    /// here, which is handled downstream (not a method on the controller
    /// class; probably a property dereference the resolver will drop).
    pub identifier: String,
    /// `true` when the identifier is immediately followed by `(`. Stored
    /// for Phase C TR-C.3 which will want to distinguish expression
    /// references from invocations; the Phase A resolver itself treats
    /// both the same way (attempts method lookup).
    pub is_invocation: bool,
    /// Byte offset of the element carrying the attribute, reported by
    /// quick-xml's `Reader::buffer_position`. Used to build a stable,
    /// per-binding `Range` so two call-edges from the same page to the
    /// same method don't collapse to an identical node id. Not a human-
    /// oriented line number; treat it as an opaque ordinal.
    pub offset: u64,
}

/// Parsed Visualforce page. All fields are declaration-order for
/// deterministic downstream iteration (see R35 / WS-TRUTH-R35).
#[derive(Debug, Clone)]
pub struct VfPage {
    /// Page name — filename sans `.page` suffix (case-preserving).
    /// `UTIL_JobProgress.page` -> `"UTIL_JobProgress"`. Used verbatim
    /// in FQN composition and as the synthetic container node's name.
    pub name: String,
    /// Absolute path to the source `.page` file. Fed into
    /// `build_simple_fqn` for the `<repo_path>` prefix of synthesised
    /// node FQNs.
    pub source_path: PathBuf,
    /// `<apex:page controller="X">`. Canonical-case, trimmed. `None`
    /// when absent (pages may rely on `extensions` alone or carry no
    /// controller — rare but legal).
    pub controller: Option<String>,
    /// `<apex:page extensions="A, B, C">` split on commas and trimmed,
    /// preserving declaration order. Salesforce resolves against the
    /// controller first, then each extension in this order — any
    /// reordering here silently breaks first-match semantics.
    pub extensions: Vec<String>,
    /// Every `{!...}` binding seen in attribute values across the
    /// document, in document order. Duplicates are preserved because
    /// two different attribute sites on two different elements are
    /// independent call sites with independent source ranges.
    pub bindings: Vec<VfBinding>,
}

impl VfPage {
    /// Iterate the resolver's candidate-class chain: controller first,
    /// then each extension in declared order. Returns the api-names
    /// exactly as declared so the caller does case-insensitive matching
    /// against the Apex class registry.
    pub fn candidate_chain(&self) -> impl Iterator<Item = &str> {
        self.controller
            .as_deref()
            .into_iter()
            .chain(self.extensions.iter().map(|s| s.as_str()))
    }
}

/// Read one `.page` file, extracting VF bindings per the scope above.
/// Returns a `VfPage` on any parseable input; malformed XML produces a
/// descriptive error. Callers typically iterate one `VfPage` per page
/// and feed it into `vf_page_resolver::resolve_vf_page`.
pub fn read_vf_page(path: &Path) -> Result<VfPage> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading VF page {}", path.display()))?;
    read_vf_page_from_str(&raw, path).with_context(|| format!("parsing VF page {}", path.display()))
}

/// In-memory variant of [`read_vf_page`]. Exposed for unit testing and
/// for callers that already have the source in a `String` (rare in
/// production; common in tests).
pub fn read_vf_page_from_str(raw: &str, path: &Path) -> Result<VfPage> {
    let name = page_name_from_path(path).with_context(|| {
        format!(
            "cannot derive VF page name from path {} (expected a *.page suffix)",
            path.display()
        )
    })?;

    let mut page = VfPage {
        name,
        source_path: path.to_path_buf(),
        controller: None,
        extensions: Vec::new(),
        bindings: Vec::new(),
    };

    let mut reader = Reader::from_str(raw);
    let cfg = reader.config_mut();
    cfg.trim_text(true);
    cfg.expand_empty_elements = false;
    cfg.check_end_names = true;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) => {
                let is_apex_page = local_tag_name(e.name().as_ref()) == "page";
                // quick-xml's `buffer_position()` is a monotonically
                // increasing byte offset into the source. Bindings on
                // different elements always receive distinct offsets,
                // which is the property we actually need for node-id
                // stability (see VfBinding::offset).
                let offset = reader.buffer_position();
                for attr in e.attributes().with_checks(false).flatten() {
                    let attr_name = local_tag_name(attr.key.as_ref());
                    let value = attr
                        .unescape_value()
                        .with_context(|| format!("unescaping attribute {attr_name}"))?
                        .into_owned();

                    if is_apex_page {
                        match attr_name.as_str() {
                            "controller" => {
                                let trimmed = value.trim();
                                if !trimmed.is_empty() {
                                    page.controller = Some(trimmed.to_string());
                                }
                            }
                            "extensions" => {
                                for ext in value.split(',') {
                                    let t = ext.trim();
                                    if !t.is_empty() {
                                        page.extensions.push(t.to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Every attribute on every element — including the
                    // controller/extensions attributes themselves — is
                    // scanned for `{!...}` bindings. The controller /
                    // extensions values do not contain `{!...}` in
                    // well-formed VF so this is a no-op there, but
                    // attempting the scan uniformly keeps the reader
                    // free of "which attributes are scannable" special
                    // cases.
                    collect_bindings_into(&value, offset, &mut page.bindings);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(page)
}

/// Extract every `{!identifier}` / `{!identifier(...)}` binding from an
/// attribute value. Public for unit tests; used internally by
/// [`read_vf_page_from_str`]. The `offset` argument is forwarded
/// verbatim to every produced [`VfBinding::offset`].
///
/// Hand-rolled scanner (no `regex` crate dependency). The recogniser
/// is narrow: opening `{!`, optional whitespace, one Apex identifier
/// (top-level only — dotted paths capture the head), optional
/// whitespace, optional `(`. Anything after is ignored for the
/// current binding; the outer loop continues scanning. Rejecting
/// non-matches after `{!` mirrors the regex's `[A-Za-z_]` lead
/// requirement — `{!123}` is not a valid Apex VF expression.
pub fn collect_bindings_into(value: &str, offset: u64, out: &mut Vec<VfBinding>) {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'{' || bytes[i + 1] != b'!' {
            i += 1;
            continue;
        }
        let mut cursor = i + 2;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        // Identifier head: `[A-Za-z_]`.
        if cursor >= bytes.len() || !is_apex_ident_head(bytes[cursor]) {
            i += 1;
            continue;
        }
        let id_start = cursor;
        while cursor < bytes.len() && is_apex_ident_tail(bytes[cursor]) {
            cursor += 1;
        }
        let identifier = std::str::from_utf8(&bytes[id_start..cursor])
            .unwrap_or("")
            .to_string();
        // Optional whitespace between identifier and `(`.
        let mut peek = cursor;
        while peek < bytes.len() && bytes[peek].is_ascii_whitespace() {
            peek += 1;
        }
        let is_invocation = peek < bytes.len() && bytes[peek] == b'(';
        out.push(VfBinding {
            identifier,
            is_invocation,
            offset,
        });
        i = cursor;
    }
}

#[inline]
fn is_apex_ident_head(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

#[inline]
fn is_apex_ident_tail(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Derive the page name from a path by stripping the `.page` (or
/// `.Page` / `.PAGE`) suffix on the filename. Returns `None` on any
/// path that doesn't end in that suffix — callers should treat that
/// as a bug (the classifier already guarantees the suffix).
pub fn page_name_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".page") {
        return None;
    }
    let stem_len = name.len() - ".page".len();
    Some(name[..stem_len].to_string())
}

/// Load `configs/visualforce.yaml`. Accepts the same directory the
/// tree-sitter loader uses so `GRAPHENGINE_CONFIGS_DIR` overrides
/// flow through uniformly. Returns a `VfConfig` on success; absence
/// of the file is a hard error because the VF pass should not run
/// against an un-configured engine.
pub fn load_vf_config(configs_dir: &Path) -> Result<VfConfig> {
    let path = configs_dir.join("visualforce.yaml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading VF config {}", path.display()))?;
    let cfg: VfConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("parsing VF config {}", path.display()))?;
    if cfg.file_extensions.is_empty() {
        anyhow::bail!(
            "VF config at {} lists no file_extensions (expected at least `.page`)",
            path.display()
        );
    }
    Ok(cfg)
}

/// Strip any `ns:` prefix from an XML local name. VF documents open
/// with `<apex:page ...>` and every Salesforce-authored VF tag carries
/// the `apex:` prefix, so stripping the prefix and comparing on the
/// bare local name is the idiomatic shape.
fn local_tag_name(bytes: &[u8]) -> String {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    match s.rfind(':') {
        Some(idx) => s[idx + 1..].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn page_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/fake/{name}"))
    }

    #[test]
    fn page_name_strips_page_suffix_case_insensitively() {
        assert_eq!(
            page_name_from_path(&page_path("UTIL_JobProgress.page")).as_deref(),
            Some("UTIL_JobProgress")
        );
        assert_eq!(
            page_name_from_path(&page_path("mixed.Page")).as_deref(),
            Some("mixed")
        );
        assert!(page_name_from_path(&page_path("nope.cls")).is_none());
    }

    #[test]
    fn collect_bindings_extracts_bare_and_invocation_forms() {
        let mut out = Vec::new();
        collect_bindings_into("{!save} {!compute()}", 5, &mut out);
        assert_eq!(
            out,
            vec![
                VfBinding {
                    identifier: "save".into(),
                    is_invocation: false,
                    offset: 5
                },
                VfBinding {
                    identifier: "compute".into(),
                    is_invocation: true,
                    offset: 5
                },
            ]
        );
    }

    #[test]
    fn collect_bindings_skips_literal_text_without_braces() {
        let mut out = Vec::new();
        collect_bindings_into("plain text with no bindings", 1, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_bindings_captures_only_head_of_dotted_path() {
        // `{!ctrl.save}` -> head `ctrl`, downstream will fail to match
        // a method by that name and drop. Phase C TR-C.3 will handle
        // dotted-path dispatch explicitly.
        let mut out = Vec::new();
        collect_bindings_into("{!ctrl.save}", 2, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].identifier, "ctrl");
        assert!(!out[0].is_invocation);
    }

    #[test]
    fn read_vf_page_extracts_controller_extensions_and_bindings() {
        let src = r#"<apex:page controller="FooCtrl" extensions="Ext1, Ext2">
            <apex:commandButton action="{!save}" rerender="block"/>
            <apex:outputText value="hello {!user.name}"/>
        </apex:page>"#;
        let page = read_vf_page_from_str(src, &page_path("Page.page")).expect("parse ok");
        assert_eq!(page.name, "Page");
        assert_eq!(page.controller.as_deref(), Some("FooCtrl"));
        assert_eq!(
            page.extensions,
            vec!["Ext1".to_string(), "Ext2".to_string()]
        );
        // `{!save}` (from action=) and `{!user.name}` (head `user`) —
        // the rerender="block" literal carries no binding.
        let idents: Vec<&str> = page
            .bindings
            .iter()
            .map(|b| b.identifier.as_str())
            .collect();
        assert_eq!(idents, vec!["save", "user"]);
    }

    #[test]
    fn candidate_chain_places_controller_before_extensions() {
        let page = VfPage {
            name: "P".into(),
            source_path: page_path("P.page"),
            controller: Some("C".into()),
            extensions: vec!["E1".into(), "E2".into()],
            bindings: Vec::new(),
        };
        let chain: Vec<&str> = page.candidate_chain().collect();
        assert_eq!(chain, vec!["C", "E1", "E2"]);
    }

    #[test]
    fn candidate_chain_handles_missing_controller() {
        let page = VfPage {
            name: "P".into(),
            source_path: page_path("P.page"),
            controller: None,
            extensions: vec!["E1".into()],
            bindings: Vec::new(),
        };
        let chain: Vec<&str> = page.candidate_chain().collect();
        assert_eq!(chain, vec!["E1"]);
    }

    #[test]
    fn empty_extensions_attribute_produces_no_entries() {
        let src = r#"<apex:page controller="C" extensions="">
        </apex:page>"#;
        let page = read_vf_page_from_str(src, &page_path("X.page")).expect("parse ok");
        assert_eq!(page.controller.as_deref(), Some("C"));
        assert!(page.extensions.is_empty());
    }
}
