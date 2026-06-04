//! Full analysis — orchestrates extracted segment runners in dependency order (S2-γ).

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use super::super::config::AnalysisConfig;
use super::super::report::HealthReport;
use crate::validation::overrides::ValidationOverrides;

use super::segments::{all_segment_ids, AnalysisSegment};
use super::session::AnalysisRunContext;

pub fn run_full(
    db_path: &str,
    config: Option<AnalysisConfig>,
    norms_path: Option<&str>,
    git_dir: Option<&str>,
    overrides: Option<&ValidationOverrides>,
    progress: Option<&Connection>,
) -> Result<HealthReport> {
    run_partial(
        &HashSet::from_iter(all_segment_ids()),
        db_path,
        config,
        norms_path,
        git_dir,
        overrides,
        progress,
    )
}

pub fn run_partial(
    segments_to_run: &HashSet<AnalysisSegment>,
    db_path: &str,
    config: Option<AnalysisConfig>,
    norms_path: Option<&str>,
    git_dir: Option<&str>,
    overrides: Option<&ValidationOverrides>,
    progress: Option<&Connection>,
) -> Result<HealthReport> {
    let mut ctx =
        AnalysisRunContext::new(db_path.to_string(), config, norms_path, git_dir, overrides);

    let needs_report = segments_to_run.contains(&AnalysisSegment::HealthScore);

    // GraphPrep always runs when any downstream segment is requested.
    if segments_to_run.contains(&AnalysisSegment::GraphPrep) || !segments_to_run.is_empty() {
        if let Some(report) = AnalysisSegment::GraphPrep.run(&mut ctx)? {
            return Ok(report);
        }
        if let Some(conn) = progress {
            super::record_segment_progress(conn, AnalysisSegment::GraphPrep)?;
        }
    }

    let ordered = all_segment_ids();
    for segment in ordered {
        if segment == AnalysisSegment::GraphPrep {
            continue;
        }
        if segment == AnalysisSegment::HealthScore {
            if needs_report {
                AnalysisSegment::HealthScore.run(&mut ctx)?;
                if let Some(conn) = progress {
                    super::record_segment_progress(conn, AnalysisSegment::HealthScore)?;
                }
            }
            continue;
        }
        if segments_to_run.contains(&segment) {
            segment.run(&mut ctx)?;
            if let Some(conn) = progress {
                super::record_segment_progress(conn, segment)?;
            }
        }
    }

    ctx.report
        .ok_or_else(|| anyhow::anyhow!("HealthScore segment did not produce a report"))
}
