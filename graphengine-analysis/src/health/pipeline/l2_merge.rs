//! L2 partial merge: rerun wiring segments, reuse auxiliary metrics from prior cache.

use anyhow::Result;
use rusqlite::Connection;

use crate::validation::overrides::ValidationOverrides;

use super::super::config::AnalysisConfig;
use super::super::report::{AnalysisProvenance, CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1};
use super::full;
use super::merge::{self, read_health_score_cache};
use super::scope::{AnalysisDelta, TrustDecision};
use super::segments::AnalysisSegment;

// These ten parameters are the standard analysis-invocation bundle this
// function forwards to `full::run_partial` (db handle, structure fingerprints,
// trust decision, delta, and the config/norms/git/overrides inputs). Folding
// them into a shared `AnalysisInvocation` params struct used by run_partial /
// l1 / l2 uniformly is the right cleanup, tracked as a post-launch refactor;
// not done now to keep the v0.1.0 landing diff scoped.
#[allow(clippy::too_many_arguments)]
pub fn try_l2_partial_merge(
    conn: &Connection,
    db_path: &str,
    current_structure_fp: &str,
    prior_structure_fp: &str,
    trust: &TrustDecision,
    delta: &AnalysisDelta,
    config: Option<AnalysisConfig>,
    norms_path: Option<&str>,
    git_dir: Option<&str>,
    overrides: Option<&ValidationOverrides>,
) -> Result<Option<super::super::report::HealthReport>> {
    let Some(prior_report) = read_health_score_cache(conn, prior_structure_fp)? else {
        return Ok(None);
    };

    let mut segments_to_run = trust.segments_to_run.clone();
    segments_to_run.insert(AnalysisSegment::FindingsAssembly);
    segments_to_run.insert(AnalysisSegment::HealthScore);

    let partial = full::run_partial(
        &segments_to_run,
        db_path,
        config,
        norms_path,
        git_dir,
        overrides,
        Some(conn),
    )?;

    let mut report = merge::merge_l2_report(&prior_report, partial, &trust.segments_to_reuse);

    if !report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1)
    {
        report
            .integrity_status
            .schema_caveats
            .push(CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1.to_string());
    }

    let reused: Vec<String> = trust
        .segments_to_reuse
        .iter()
        .map(|s| s.as_str().to_string())
        .collect();
    let mut rerun: Vec<String> = segments_to_run
        .iter()
        .map(|s| s.as_str().to_string())
        .collect();
    rerun.sort();
    rerun.dedup();

    report.analysis_provenance = Some(AnalysisProvenance {
        analysis_mode: "segmented_sync".into(),
        trust_level: "L2".into(),
        structure_fingerprint: current_structure_fp.to_string(),
        structure_changed: true,
        segments_reused: {
            let mut v = reused;
            v.sort();
            v
        },
        segments_rerun: rerun,
        changed_paths: delta.changed_paths.clone(),
        delta_fingerprint: String::new(),
        query_trust_note: None,
    });

    Ok(Some(report))
}
