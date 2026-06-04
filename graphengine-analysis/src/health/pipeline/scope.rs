//! Delta scope and S2-γ trust ladder (L0–L3).

use std::collections::HashSet;

use graphengine_parsing::infrastructure::storage::parse_meta_store::IncrementalScanStats;

use super::segments::AnalysisSegment;
use super::{SEGMENTED_RATIO_THRESHOLD, SEGMENTED_SMALL_FILE_THRESHOLD};

#[derive(Debug, Clone)]
pub struct AnalysisDelta {
    pub changed_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

impl AnalysisDelta {
    pub fn from_stats(stats: &IncrementalScanStats) -> Self {
        Self {
            changed_paths: stats.changed_paths.clone(),
            removed_paths: stats.removed_paths.clone(),
        }
    }

    pub fn file_count(&self) -> usize {
        self.changed_paths.len() + self.removed_paths.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    L0,
    L1,
    L2,
    L3,
}

impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
        }
    }
}

pub struct TrustDecision {
    pub level: TrustLevel,
    pub structure_changed: bool,
    pub segments_to_reuse: HashSet<AnalysisSegment>,
    pub segments_to_run: HashSet<AnalysisSegment>,
}

/// Workspace edits exist but parse DB still reports zero delta — plan L1
/// until `gridseak scan rescan` catches up (avoids ratio false-L3 when cached=0).
pub fn trust_decision_for_workspace_rescan() -> TrustDecision {
    l1_decision()
}

pub fn classify_trust(
    stats: &IncrementalScanStats,
    total_files: usize,
    current_structure_fp: &str,
    prior_structure_fp: Option<&str>,
    force_full_analysis: bool,
) -> TrustDecision {
    if stats.is_zero_delta() {
        return TrustDecision {
            level: TrustLevel::L0,
            structure_changed: false,
            segments_to_reuse: all_reusable_segments(),
            segments_to_run: HashSet::new(),
        };
    }

    if force_full_analysis || is_large_delta(stats, total_files) {
        return full_run_decision(TrustLevel::L3);
    }

    let structure_changed = prior_structure_fp
        .map(|prior| prior != current_structure_fp)
        .unwrap_or(true);

    if !structure_changed {
        l1_decision()
    } else {
        l2_decision()
    }
}

fn is_large_delta(stats: &IncrementalScanStats, total_files: usize) -> bool {
    let delta = stats.delta_file_count();
    delta > SEGMENTED_SMALL_FILE_THRESHOLD
        || (total_files > 0 && delta as f64 / total_files as f64 > SEGMENTED_RATIO_THRESHOLD)
}

fn all_reusable_segments() -> HashSet<AnalysisSegment> {
    use AnalysisSegment::*;
    [
        GraphPrep,
        Cycles,
        FanMetrics,
        ModuleCoupling,
        DeadCode,
        BlastRadius,
        Complexity,
        AuxiliaryMetrics,
        FindingsAssembly,
        HealthScore,
    ]
    .into_iter()
    .collect()
}

fn full_run_decision(level: TrustLevel) -> TrustDecision {
    TrustDecision {
        level,
        structure_changed: true,
        segments_to_reuse: HashSet::new(),
        segments_to_run: all_reusable_segments(),
    }
}

fn l1_decision() -> TrustDecision {
    use AnalysisSegment::*;
    let mut reuse = HashSet::new();
    for seg in [
        Cycles,
        FanMetrics,
        ModuleCoupling,
        DeadCode,
        BlastRadius,
        FindingsAssembly,
    ] {
        reuse.insert(seg);
    }
    let mut run = HashSet::new();
    for seg in [GraphPrep, Complexity, HealthScore] {
        run.insert(seg);
    }
    TrustDecision {
        level: TrustLevel::L1,
        structure_changed: false,
        segments_to_reuse: reuse,
        segments_to_run: run,
    }
}

fn l2_decision() -> TrustDecision {
    use AnalysisSegment::*;
    let mut reuse = HashSet::new();
    reuse.insert(AuxiliaryMetrics);
    let mut run = HashSet::new();
    for seg in [
        GraphPrep,
        Cycles,
        FanMetrics,
        ModuleCoupling,
        DeadCode,
        BlastRadius,
        Complexity,
        FindingsAssembly,
        HealthScore,
    ] {
        run.insert(seg);
    }
    TrustDecision {
        level: TrustLevel::L2,
        structure_changed: true,
        segments_to_reuse: reuse,
        segments_to_run: run,
    }
}

pub fn segment_names(segments: &HashSet<AnalysisSegment>) -> Vec<String> {
    let mut names: Vec<String> = segments.iter().map(|s| s.as_str().to_string()).collect();
    names.sort();
    names
}

/// Segments that must rerun for the current trust decision.
pub fn invalidate_segments(trust: &TrustDecision) -> HashSet<AnalysisSegment> {
    trust.segments_to_run.clone()
}

/// Segments safe to load from cache for the current trust decision.
pub fn reusable_segments(trust: &TrustDecision) -> HashSet<AnalysisSegment> {
    trust.segments_to_reuse.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(changed: &[&str], reparsed: usize, cached: usize) -> IncrementalScanStats {
        IncrementalScanStats {
            cached,
            reparsed,
            removed: 0,
            plan_disabled: false,
            changed_paths: changed.iter().map(|s| s.to_string()).collect(),
            removed_paths: Vec::new(),
        }
    }

    #[test]
    fn classify_trust_matrix() {
        struct Case {
            name: &'static str,
            stats: IncrementalScanStats,
            total_files: usize,
            current_fp: &'static str,
            prior_fp: Option<&'static str>,
            force_full: bool,
            level: TrustLevel,
            reuse: HashSet<AnalysisSegment>,
            run: HashSet<AnalysisSegment>,
        }

        use AnalysisSegment::*;

        let l1_reuse: HashSet<_> = [
            Cycles,
            FanMetrics,
            ModuleCoupling,
            DeadCode,
            BlastRadius,
            FindingsAssembly,
        ]
        .into_iter()
        .collect();
        let l1_run: HashSet<_> = [GraphPrep, Complexity, HealthScore].into_iter().collect();

        let l2_reuse: HashSet<_> = [AuxiliaryMetrics].into_iter().collect();
        let l2_run: HashSet<_> = [
            GraphPrep,
            Cycles,
            FanMetrics,
            ModuleCoupling,
            DeadCode,
            BlastRadius,
            Complexity,
            FindingsAssembly,
            HealthScore,
        ]
        .into_iter()
        .collect();

        let all = all_reusable_segments();

        let cases = vec![
            Case {
                name: "L0 zero delta",
                stats: stats(&[], 0, 10),
                total_files: 10,
                current_fp: "fp-a",
                prior_fp: Some("fp-a"),
                force_full: false,
                level: TrustLevel::L0,
                reuse: all.clone(),
                run: HashSet::new(),
            },
            Case {
                name: "L1 topology stable small delta",
                stats: stats(&["src/a.rs"], 1, 99),
                total_files: 100,
                current_fp: "fp-a",
                prior_fp: Some("fp-a"),
                force_full: false,
                level: TrustLevel::L1,
                reuse: l1_reuse.clone(),
                run: l1_run.clone(),
            },
            Case {
                name: "L2 structure changed small delta",
                stats: stats(&["src/a.rs"], 1, 99),
                total_files: 100,
                current_fp: "fp-b",
                prior_fp: Some("fp-a"),
                force_full: false,
                level: TrustLevel::L2,
                reuse: l2_reuse.clone(),
                run: l2_run.clone(),
            },
            Case {
                name: "L3 force full",
                stats: stats(&["src/a.rs"], 1, 9),
                total_files: 10,
                current_fp: "fp-a",
                prior_fp: Some("fp-a"),
                force_full: true,
                level: TrustLevel::L3,
                reuse: HashSet::new(),
                run: all.clone(),
            },
            Case {
                name: "L3 large delta",
                stats: stats(&["a", "b", "c", "d", "e", "f"], 6, 94),
                total_files: 100,
                current_fp: "fp-a",
                prior_fp: Some("fp-a"),
                force_full: false,
                level: TrustLevel::L3,
                reuse: HashSet::new(),
                run: all,
            },
        ];

        for case in cases {
            let decision = classify_trust(
                &case.stats,
                case.total_files,
                case.current_fp,
                case.prior_fp,
                case.force_full,
            );
            assert_eq!(decision.level, case.level, "case {}", case.name);
            assert_eq!(
                decision.segments_to_reuse, case.reuse,
                "reuse mismatch for {}",
                case.name
            );
            assert_eq!(
                decision.segments_to_run, case.run,
                "run mismatch for {}",
                case.name
            );
        }
    }
}
