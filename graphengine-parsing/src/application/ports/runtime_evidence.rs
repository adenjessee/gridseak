//! Port for runtime coverage / trace witnesses.

use graphengine_runtime_witness::{EdgeObserved, NodeExecuted};

pub trait RuntimeEvidencePort {
    fn executed_nodes(&self) -> &[NodeExecuted];
    fn observed_edges(&self) -> &[EdgeObserved];
}

pub struct StaticRuntimeEvidence {
    pub executed: Vec<NodeExecuted>,
    pub observed: Vec<EdgeObserved>,
}

impl RuntimeEvidencePort for StaticRuntimeEvidence {
    fn executed_nodes(&self) -> &[NodeExecuted] {
        &self.executed
    }
    fn observed_edges(&self) -> &[EdgeObserved] {
        &self.observed
    }
}
