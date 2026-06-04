//! Distance from the Main Sequence (D) metric per module.
//!
//! D = |A + I - 1| where A = Abstractness and I = Instability.
//! A module on the "main sequence" (D = 0) has a balanced trade-off between
//! abstractness and stability. Modules far from the main sequence are either:
//! - In the "Zone of Pain" (high stability, low abstractness) — hard to change
//! - In the "Zone of Uselessness" (high abstractness, low stability) — not depended on

use std::collections::{BTreeMap, HashMap};

use super::abstractness::ModuleAbstractness;
use super::instability::ModuleInstability;

#[derive(Debug, Clone)]
pub struct ModuleDistance {
    pub distance: f64,
    pub abstractness: f64,
    pub instability: f64,
    pub zone: Zone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    MainSequence,
    ZoneOfPain,
    ZoneOfUselessness,
    Balanced,
}

impl std::fmt::Display for Zone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Zone::MainSequence => write!(f, "main_sequence"),
            Zone::ZoneOfPain => write!(f, "zone_of_pain"),
            Zone::ZoneOfUselessness => write!(f, "zone_of_uselessness"),
            Zone::Balanced => write!(f, "balanced"),
        }
    }
}

/// Per-module distance-from-main-sequence result.
///
/// Returned as a `BTreeMap` — not `HashMap` — because downstream
/// consumers (HealthReport avg aggregation, finding generation,
/// MetricsReport descriptions) iterate `values()` / `iter()` on
/// the returned map and any HashMap iteration order would make
/// the aggregated float (avg_distance) drift by a last-ulp and
/// shuffle finding order across runs. See R35 in
/// FOLLOWUP_RISKS.md for the full determinism contract.
pub fn compute_distance(
    abstractness_map: &HashMap<String, ModuleAbstractness>,
    instability_map: &HashMap<String, ModuleInstability>,
) -> BTreeMap<String, ModuleDistance> {
    let mut results = BTreeMap::new();

    // Iterate the input HashMap via a canonical lexical walk. Float
    // results per module are independent of iteration order (each
    // (a, i) pair is computed locally), but sorting the key set here
    // also keeps any future in-place accumulation deterministic.
    let mut keys: Vec<&String> = abstractness_map.keys().collect();
    keys.sort();

    for module_key in keys {
        let abst = match abstractness_map.get(module_key) {
            Some(a) => a,
            None => continue,
        };
        let inst = match instability_map.get(module_key) {
            Some(i) => i,
            None => continue,
        };

        let a = abst.abstractness;
        let i = inst.instability;
        let d = (a + i - 1.0).abs();

        let zone = if d < 0.15 {
            Zone::MainSequence
        } else if a < 0.3 && i < 0.3 {
            Zone::ZoneOfPain
        } else if a > 0.7 && i > 0.7 {
            Zone::ZoneOfUselessness
        } else {
            Zone::Balanced
        };

        results.insert(
            module_key.clone(),
            ModuleDistance {
                distance: d,
                abstractness: a,
                instability: i,
                zone,
            },
        );
    }

    results
}
