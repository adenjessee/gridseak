//! Fuse one coverage format into the in-memory graph before persist.

use crate::domain::{Confidence, Graph, ProvenanceSource};
use graphengine_runtime_witness::{ingest_go_cover, ingest_istanbul, RuntimeEvidence};
use std::path::Path;

pub fn load_workspace_coverage(root: &Path) -> Option<RuntimeEvidence> {
    if let Ok(path) = std::env::var("GRIDSEAK_COVERAGE_PATH") {
        return load_path(Path::new(&path));
    }
    let istanbul = root.join(".gridseak").join("coverage.json");
    if istanbul.is_file() {
        return load_path(&istanbul);
    }
    let go = root.join(".gridseak").join("coverage.out");
    if go.is_file() {
        return load_path(&go);
    }
    None
}

fn load_path(path: &Path) -> Option<RuntimeEvidence> {
    let text = std::fs::read_to_string(path).ok()?;
    if path.extension().and_then(|s| s.to_str()) == Some("out") || text.starts_with("mode:") {
        return ingest_go_cover(&text).ok();
    }
    ingest_istanbul(&text).ok()
}

/// Stamp observed Call edges as Runtime (p_true = 1.0) and persist executed nodes.
pub fn fuse_runtime_evidence(graph: &mut Graph, evidence: &RuntimeEvidence) {
    if evidence.observed.is_empty() && evidence.executed.is_empty() {
        return;
    }
    let fqn_to_id: std::collections::HashMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|n| (n.fqn.as_str(), n.id.as_str()))
        .collect();
    for obs in &evidence.observed {
        let Some(from) = fqn_to_id.get(obs.caller.as_str()) else {
            continue;
        };
        let Some(to) = fqn_to_id.get(obs.callee.as_str()) else {
            continue;
        };
        for e in graph.edges.iter_mut() {
            if e.from_id == *from
                && e.to_id == *to
                && matches!(e.kind, crate::domain::EdgeKind::Call)
            {
                e.provenance.source = ProvenanceSource::Runtime;
                e.provenance.confidence = Confidence::High;
            }
        }
    }
    if let Ok(json) = serde_json::to_string(&evidence.executed) {
        graph.metadata.insert("runtime_executed_json".into(), json);
    }
    graph.metadata.insert(
        "runtime_observed_edges".into(),
        evidence.observed.len().to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Edge, EdgeKind, Node, Provenance, Range};
    use graphengine_runtime_witness::EdgeObserved;

    #[test]
    fn observed_edge_becomes_runtime() {
        let mut g = Graph::new();
        let a = Node::function("a".into(), Range::test(1, 0, 1, 1));
        let b = Node::function("b".into(), Range::test(2, 0, 2, 1));
        g.add_node(a.clone());
        g.add_node(b.clone());
        g.add_edge(Edge {
            from_id: a.id.clone(),
            to_id: b.id.clone(),
            kind: EdgeKind::Call,
            provenance: Provenance::heuristic(),
        });
        let ev = RuntimeEvidence {
            executed: vec![],
            observed: vec![EdgeObserved {
                caller: a.fqn.clone(),
                callee: b.fqn.clone(),
                count: 1,
            }],
        };
        fuse_runtime_evidence(&mut g, &ev);
        assert_eq!(g.edges[0].provenance.source, ProvenanceSource::Runtime);
    }
}
