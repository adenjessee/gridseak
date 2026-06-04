//! Per-node and repo-wide language detection.
//!
//! Two surfaces live here:
//!
//! 1. [`propagate_file_metadata_to_descendants`] — internal-only,
//!    invoked by [`super::analysis_graph::AnalysisGraph::build`] during
//!    construction so every non-File node inherits its parent File's
//!    `language` / `frameworks`. Replaces the pre-Wave-2 pattern of
//!    dispatching dead-code classification on a single repo-wide
//!    [`Ecosystem`](crate::health::config::Ecosystem); polyglot repos
//!    (NPSP Apex + LWC JS + Python tooling) need per-node language to
//!    dispatch classifier rules correctly.
//! 2. [`detect_ecosystem`] / [`detect_primary_language`] — public,
//!    read-only views of the parse DB that produce the analyzer's
//!    authoritative answer for "what is this repo, mostly?". The two
//!    must stay in lockstep (the >= 60% File-majority rule is shared)
//!    so `HealthReport.primary_language` and the dispatched
//!    `Ecosystem` cannot disagree.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;

use super::types::{GraphNode, NodeKind};

/// Propagate `language` and `frameworks` from every `File` node down
/// to its descendants. Nodes that already carry a language (e.g. File
/// nodes, or rare cases where the extractor stamped `language` on a
/// function) are left untouched. For any other node with a
/// `file_path` that resolves to a known File in `file_path_index`,
/// the File's language and frameworks are copied verbatim.
///
/// Why this exists. Pre-Wave-2, dead-code classification dispatched
/// on a single global `Ecosystem` inferred from the database. That
/// collapses polyglot repos (NPSP Apex + LWC JS + Python tooling)
/// to a single language and silently misclassifies everything
/// outside the majority. Materialising language per node lets the
/// framework-keyed classifier dispatch on the *file's* language,
/// not the repo's majority.
pub(super) fn propagate_file_metadata_to_descendants(
    nodes: &mut BTreeMap<String, GraphNode>,
    file_path_index: &HashMap<String, String>,
) {
    let file_meta: HashMap<String, (Option<String>, Vec<String>)> = nodes
        .iter()
        .filter(|(_, n)| n.kind == NodeKind::File)
        .map(|(id, n)| (id.clone(), (n.language.clone(), n.frameworks.clone())))
        .collect();

    for (_id, node) in nodes.iter_mut() {
        if node.kind == NodeKind::File {
            continue;
        }

        let needs_language = node.language.is_none();
        let needs_frameworks = node.frameworks.is_empty();
        if !needs_language && !needs_frameworks {
            continue;
        }

        let file_path = match node.file_path.as_deref() {
            Some(fp) => fp,
            None => continue,
        };
        let file_id = match file_path_index.get(file_path) {
            Some(id) => id,
            None => continue,
        };
        let (lang, fws) = match file_meta.get(file_id) {
            Some(m) => m,
            None => continue,
        };

        if needs_language {
            if let Some(l) = lang.as_deref() {
                node.language = Some(l.to_string());
            }
        }
        if needs_frameworks && !fws.is_empty() {
            node.frameworks = fws.clone();
        }
    }
}

/// Auto-detect ecosystem from the database by reading the `language` property
/// from Project or File nodes. Returns `Unknown` if no language metadata found.
///
/// Decision rules (in priority order):
///
/// 1. If there is a clear File-node majority (>= 60% of files share one
///    language), use that. This is the most reliable signal because each
///    File node's language is emitted by the language-specific extractor
///    that actually parsed the file. (NPSP demonstrates why this matters:
///    the Project node wrote `language=javascript` but 74% of File nodes
///    are Apex, so Apex-specific dead-code classification must dispatch on
///    "apex" regardless of the Project label.)
///
/// 2. Otherwise, fall back to the Project node's language property when
///    present and recognised.
///
/// 3. Otherwise, fall back to the plurality language (any winner, no
///    minimum threshold).
///
/// Emits a `[ge-analyze] WARNING:` line whenever Project and File-majority
/// disagree to keep polyglot mis-tagging visible in logs.
pub fn detect_ecosystem(conn: &Connection) -> super::super::config::Ecosystem {
    let project_lang: Option<String> = conn
        .query_row(
            "SELECT properties FROM nodes WHERE kind = 'Project' LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|json| {
            serde_json::from_str::<serde_json::Value>(&json)
                .ok()?
                .get("language")?
                .as_str()
                .map(String::from)
        });

    let mut lang_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut total_files: usize = 0;
    if let Ok(mut stmt) = conn.prepare("SELECT properties FROM nodes WHERE kind = 'File'") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for row in rows.flatten() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&row) {
                    if let Some(lang) = val.get("language").and_then(|v| v.as_str()) {
                        *lang_counts.entry(lang.to_lowercase()).or_default() += 1;
                        total_files += 1;
                    }
                }
            }
        }
    }

    let mut lang_sorted: Vec<_> = lang_counts.into_iter().collect();
    lang_sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    if let Some((lang, count)) = lang_sorted.first() {
        if total_files > 0 && *count as f64 / total_files as f64 >= 0.60 {
            let eco = super::super::config::Ecosystem::from_language_str(lang);
            if eco != super::super::config::Ecosystem::Unknown {
                if let Some(ref plang) = project_lang {
                    let pl = plang.to_lowercase();
                    if &pl != lang {
                        eprintln!(
                            "[ge-analyze] WARNING: Project node language='{}' disagrees with File-majority language='{}' ({:.0}% of {} files). Using File-majority; Project label is likely a parser default.",
                            plang,
                            lang,
                            100.0 * (*count as f64 / total_files as f64),
                            total_files
                        );
                    }
                }
                return eco;
            }
        }
    }

    if let Some(lang) = project_lang.as_deref() {
        let eco = super::super::config::Ecosystem::from_language_str(lang);
        if eco != super::super::config::Ecosystem::Unknown {
            return eco;
        }
    }

    if let Some((lang, _)) = lang_sorted.first() {
        let eco = super::super::config::Ecosystem::from_language_str(lang);
        if eco != super::super::config::Ecosystem::Unknown {
            return eco;
        }
    }

    eprintln!("[ge-analyze] WARNING: No language metadata in database, using Unknown profile");
    super::super::config::Ecosystem::Unknown
}

/// Compute the canonical primary language for the scan from File-node
/// language counts. This is the analyzer's authoritative answer for
/// "what is this repo, mostly?" and is the value that flows out as
/// `HealthReport.primary_language` and back into `scan_runs.primary_language`.
///
/// Rules (mirrors [`detect_ecosystem`] step 1 so the two cannot drift):
///
/// 1. If one language covers >= 60% of File nodes that have a `language`
///    property, return that language verbatim (lowercased).
/// 2. Otherwise, if any File-node language exists at all, return the
///    plurality winner (lexicographic tiebreak — same ordering as
///    `detect_ecosystem` to keep the two answers consistent).
/// 3. Otherwise return `None`.
///
/// We deliberately do not fall back to the `Project` node's
/// `properties.language` — that field is what the parser writes per
/// pass and is the very value this function is supposed to correct.
/// Falling back to it would re-introduce the A3 bug.
pub fn detect_primary_language(conn: &Connection) -> Option<String> {
    let mut lang_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut total_files: usize = 0;
    if let Ok(mut stmt) = conn.prepare("SELECT properties FROM nodes WHERE kind = 'File'") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for row in rows.flatten() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&row) {
                    if let Some(lang) = val.get("language").and_then(|v| v.as_str()) {
                        *lang_counts.entry(lang.to_lowercase()).or_default() += 1;
                        total_files += 1;
                    }
                }
            }
        }
    }
    if total_files == 0 {
        return None;
    }
    let mut sorted: Vec<_> = lang_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted.first().map(|(lang, _)| lang.clone())
}

#[cfg(test)]
mod propagation_tests {
    use super::super::analysis_graph::AnalysisGraph;
    use super::*;

    fn file_node(id: &str, path: &str, language: Option<&str>, frameworks: &[&str]) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: NodeKind::File,
            fqn: path.to_string(),
            name: path.to_string(),
            file_path: Some(path.to_string()),
            path_repo_rel: Some(path.to_string()),
            language: language.map(String::from),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn fn_node(id: &str, file_path: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: NodeKind::Function,
            fqn: format!("test::{id}"),
            name: id.to_string(),
            file_path: Some(file_path.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn file_language_propagates_to_function_descendant() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file1".into(),
            file_node("file1", "src/Foo.cls", Some("apex"), &["tdtm"]),
        );
        nodes.insert("fn1".into(), fn_node("fn1", "src/Foo.cls"));

        let g = AnalysisGraph::build(nodes, vec![]);
        let fn_node = g.nodes.get("fn1").unwrap();
        assert_eq!(fn_node.language.as_deref(), Some("apex"));
        assert_eq!(fn_node.frameworks, vec!["tdtm".to_string()]);
    }

    #[test]
    fn function_without_file_match_keeps_unknown_language() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file1".into(),
            file_node("file1", "src/Foo.cls", Some("apex"), &[]),
        );
        nodes.insert(
            "fn_orphan".into(),
            fn_node("fn_orphan", "src/DoesNotExist.cls"),
        );

        let g = AnalysisGraph::build(nodes, vec![]);
        let orphan = g.nodes.get("fn_orphan").unwrap();
        assert_eq!(orphan.language, None);
        assert!(orphan.frameworks.is_empty());
    }

    #[test]
    fn explicit_node_language_is_not_overwritten() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "file1".into(),
            file_node("file1", "src/Foo.cls", Some("apex"), &["tdtm"]),
        );
        let mut fn1 = fn_node("fn1", "src/Foo.cls");
        fn1.language = Some("javascript".into());
        fn1.frameworks = vec!["lwc".into()];
        nodes.insert("fn1".into(), fn1);

        let g = AnalysisGraph::build(nodes, vec![]);
        let fn_node = g.nodes.get("fn1").unwrap();
        assert_eq!(fn_node.language.as_deref(), Some("javascript"));
        assert_eq!(fn_node.frameworks, vec!["lwc".to_string()]);
    }

    #[test]
    fn polyglot_repo_has_per_node_language() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "apex_file".into(),
            file_node("apex_file", "force-app/classes/Foo.cls", Some("apex"), &[]),
        );
        nodes.insert(
            "lwc_file".into(),
            file_node(
                "lwc_file",
                "force-app/lwc/bar/bar.js",
                Some("javascript"),
                &["lwc"],
            ),
        );
        nodes.insert(
            "apex_fn".into(),
            fn_node("apex_fn", "force-app/classes/Foo.cls"),
        );
        nodes.insert(
            "lwc_fn".into(),
            fn_node("lwc_fn", "force-app/lwc/bar/bar.js"),
        );

        let g = AnalysisGraph::build(nodes, vec![]);
        assert_eq!(
            g.nodes.get("apex_fn").unwrap().language.as_deref(),
            Some("apex")
        );
        assert_eq!(
            g.nodes.get("lwc_fn").unwrap().language.as_deref(),
            Some("javascript")
        );
        assert_eq!(
            g.nodes.get("lwc_fn").unwrap().frameworks,
            vec!["lwc".to_string()]
        );
        assert!(g.nodes.get("apex_fn").unwrap().frameworks.is_empty());
    }
}
