//! GridSeak diagnostic translation layer.
//!
//! Converts raw [`HealthReport`] data into decision-grade product output:
//!
//! - [`priority`]  — Fix-First composite score per `docs/02-strategy/DIAGNOSTIC_PRODUCT_SPEC.md` §1.
//! - [`narratives`] — Risk-narrative + suggested-action templates per §2.
//! - [`trend`]     — Baseline delta for two reports of the same project per §3.
//!
//! All functions are pure and deterministic. No LLMs, no network, no I/O.

pub mod narratives;
pub mod priority;
pub mod trend;

pub use narratives::{impact_category, risk_narrative, suggested_action};
pub use priority::{compute_priorities, enrich_findings, PriorityItem};
pub use trend::{diff, FindingDelta, MetricDelta, ModuleDelta, TrendDirection, TrendReport};
