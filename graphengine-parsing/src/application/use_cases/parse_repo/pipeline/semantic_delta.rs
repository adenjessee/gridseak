//! Semantic-delta planner: resolve only changed files + sites whose target moved.

use crate::application::ports::{ResolvedEdges, UnresolvedReference};
use crate::domain::{Edge, Graph};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A persisted call-site → target-file binding from the previous scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSiteBinding {
    pub caller_file: PathBuf,
    pub target_file: PathBuf,
}

/// Files whose call sites must be re-resolved this scan.
pub fn files_needing_resolution(
    changed_files: &[PathBuf],
    bindings: &[CallSiteBinding],
) -> HashSet<PathBuf> {
    let changed: HashSet<&Path> = changed_files.iter().map(PathBuf::as_path).collect();
    let mut out: HashSet<PathBuf> = changed_files.iter().cloned().collect();
    for b in bindings {
        if changed.contains(b.target_file.as_path()) {
            out.insert(b.caller_file.clone());
        }
        if changed.contains(b.caller_file.as_path()) {
            out.insert(b.caller_file.clone());
        }
    }
    out
}

/// Keep only references whose call-site file is in `resolve_files`.
pub fn filter_refs_to_files(
    refs: Vec<UnresolvedReference>,
    resolve_files: &HashSet<PathBuf>,
) -> Vec<UnresolvedReference> {
    if resolve_files.is_empty() {
        return refs;
    }
    refs.into_iter()
        .filter(|r| {
            let file = PathBuf::from(&r.call_site().location.file);
            resolve_files.contains(&file)
                || resolve_files
                    .iter()
                    .any(|p| paths_equivalent(p, file.as_path()))
        })
        .collect()
}

fn paths_equivalent(a: &Path, b: &Path) -> bool {
    a == b
        || a.file_name() == b.file_name() && a.to_string_lossy().ends_with(&*b.to_string_lossy())
        || b.to_string_lossy().ends_with(&*a.to_string_lossy())
}

/// Merge prior Call edges whose caller file is not being re-resolved.
pub fn merge_prior_call_edges(
    resolved: &mut ResolvedEdges,
    prior: Vec<Edge>,
    symbols: &[crate::domain::Node],
    resolve_files: &HashSet<PathBuf>,
) {
    if prior.is_empty() || resolve_files.is_empty() {
        return;
    }
    let id_to_file: HashMap<&str, &str> = symbols
        .iter()
        .map(|n| (n.id.as_str(), n.location.file.as_str()))
        .collect();
    let existing: HashSet<(String, String)> = resolved
        .call_edges
        .iter()
        .map(|e| (e.from_id.clone(), e.to_id.clone()))
        .collect();
    for edge in prior {
        let Some(file) = id_to_file.get(edge.from_id.as_str()) else {
            continue;
        };
        let pb = PathBuf::from(*file);
        let re_resolving = resolve_files.contains(&pb)
            || resolve_files
                .iter()
                .any(|p| paths_equivalent(p, pb.as_path()));
        if re_resolving {
            continue;
        }
        if existing.contains(&(edge.from_id.clone(), edge.to_id.clone())) {
            continue;
        }
        resolved.add_call_edge(edge);
    }
}

/// Persistable caller_file → target_file pairs from the built graph.
pub fn bindings_from_graph(graph: &Graph) -> Vec<(String, String)> {
    let id_to_file: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.location.file.as_str()))
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in &graph.edges {
        if !matches!(e.kind, crate::domain::EdgeKind::Call) {
            continue;
        }
        let Some(caller) = id_to_file.get(e.from_id.as_str()) else {
            continue;
        };
        let Some(target) = id_to_file.get(e.to_id.as_str()) else {
            continue;
        };
        if seen.insert((*caller, *target)) {
            out.push(((*caller).to_string(), (*target).to_string()));
        }
    }
    out
}

pub fn bindings_from_rows(rows: Vec<(String, String)>) -> Vec<CallSiteBinding> {
    rows.into_iter()
        .map(|(caller_file, target_file)| CallSiteBinding {
            caller_file: PathBuf::from(caller_file),
            target_file: PathBuf::from(target_file),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependent_sites_are_included() {
        let changed = vec![PathBuf::from("lib.ts")];
        let bindings = vec![CallSiteBinding {
            caller_file: PathBuf::from("app.ts"),
            target_file: PathBuf::from("lib.ts"),
        }];
        let set = files_needing_resolution(&changed, &bindings);
        assert!(set.contains(&PathBuf::from("app.ts")));
        assert!(set.contains(&PathBuf::from("lib.ts")));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn unrelated_file_is_skipped() {
        let changed = vec![PathBuf::from("a.ts")];
        let bindings = vec![CallSiteBinding {
            caller_file: PathBuf::from("b.ts"),
            target_file: PathBuf::from("c.ts"),
        }];
        let set = files_needing_resolution(&changed, &bindings);
        assert_eq!(set.len(), 1);
        assert!(set.contains(&PathBuf::from("a.ts")));
    }

    #[test]
    fn filter_drops_unrelated_call_sites() {
        use crate::application::ports::CallSite;
        use crate::domain::Range;
        let refs = vec![
            UnresolvedReference::Call(CallSite {
                location: Range::with_file(1, 0, 1, 4, "app.ts"),
                function_name: "f".into(),
                receiver_range: None,
                receiver_text: None,
                arg_types: vec![],
            }),
            UnresolvedReference::Call(CallSite {
                location: Range::with_file(1, 0, 1, 4, "other.ts"),
                function_name: "g".into(),
                receiver_range: None,
                receiver_text: None,
                arg_types: vec![],
            }),
        ];
        let mut resolve = HashSet::new();
        resolve.insert(PathBuf::from("app.ts"));
        let filtered = filter_refs_to_files(refs, &resolve);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].call_site().function_name, "f");
    }
}
