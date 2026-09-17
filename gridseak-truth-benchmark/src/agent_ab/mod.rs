//! Deterministic agent-vs-control harness (plan: Agent A/B proof).

pub mod control;
pub mod oracle;
pub mod scorer;
pub mod task;
pub mod treatment;

pub use scorer::{run_harness, HarnessReport};
pub use task::{load_tasks, Task};
