//! S2-β segmented analysis pipeline — mode selection, segment cache, merge.

pub mod cache;
pub mod full;
pub mod incremental_plan;
pub mod l1_merge;
pub mod l2_merge;
pub mod merge;
pub mod scope;
pub mod segments;
pub mod session;

pub use incremental_plan::{predict_incremental_plan, IncrementalPlan};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use graphengine_parsing::infrastructure::storage::parse_meta_store::{
    compute_delta_fingerprint, compute_structure_fingerprint, read_incremental_scan_stats,
    read_structure_fingerprint, write_structure_fingerprint, IncrementalScanStats,
};

use crate::validation::overrides::ValidationOverrides;

use super::config::AnalysisConfig;
use super::incremental_fast_path::{write_analysis_cache, ANALYSIS_STATUS_KEY};
use super::report::{
    AnalysisProvenance, HealthReport, CAVEAT_INCREMENTAL_ANALYSIS_STRUCTURE_CHANGED_V1,
};

use cache::{write_segment_cache, SegmentCacheRow};
use scope::{classify_trust, invalidate_segments, segment_names, AnalysisDelta, TrustLevel};
use segments::{all_segment_ids, AnalysisMode, AnalysisSegment};

pub const SEGMENTED_SMALL_FILE_THRESHOLD: usize = 5;
pub const SEGMENTED_RATIO_THRESHOLD: f64 = 0.02;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisStatusRecord {
    pub mode: String,
    pub complete: bool,
    #[serde(default)]
    pub segments_done: Vec<String>,
    #[serde(default)]
    pub segments_pending: Vec<String>,
    pub started_at: String,
    pub updated_at: String,
}

pub struct PipelineOutcome {
    pub report: HealthReport,
    pub mode: AnalysisMode,
    pub delta_files: usize,
}

#[derive(Default)]
pub struct PipelineOptions {
    pub force_full_analysis: bool,
}

pub fn select_mode(stats: &IncrementalScanStats, total_files: usize) -> AnalysisMode {
    if stats.is_zero_delta() {
        AnalysisMode::ZeroReuse
    } else if stats.delta_file_count() <= SEGMENTED_SMALL_FILE_THRESHOLD
        || (total_files > 0
            && stats.delta_file_count() as f64 / total_files as f64 <= SEGMENTED_RATIO_THRESHOLD)
    {
        AnalysisMode::SegmentedSync
    } else {
        AnalysisMode::Full
    }
}

pub fn run_analysis_pipeline(
    db_path: &str,
    config: Option<AnalysisConfig>,
    norms_path: Option<&str>,
    git_dir: Option<&str>,
    overrides: Option<&ValidationOverrides>,
    incremental_enabled: bool,
) -> Result<PipelineOutcome> {
    run_analysis_pipeline_with_options(
        db_path,
        config,
        norms_path,
        git_dir,
        overrides,
        incremental_enabled,
        PipelineOptions::default(),
    )
}

pub fn run_analysis_pipeline_with_options(
    db_path: &str,
    config: Option<AnalysisConfig>,
    norms_path: Option<&str>,
    git_dir: Option<&str>,
    overrides: Option<&ValidationOverrides>,
    incremental_enabled: bool,
    options: PipelineOptions,
) -> Result<PipelineOutcome> {
    let conn = Connection::open(db_path)?;
    let stats = read_incremental_scan_stats(&conn)?.unwrap_or(IncrementalScanStats {
        cached: 0,
        reparsed: 0,
        removed: 0,
        plan_disabled: !incremental_enabled,
        changed_paths: Vec::new(),
        removed_paths: Vec::new(),
    });

    let total_files = stats.cached + stats.reparsed;
    let mode = if incremental_enabled && !options.force_full_analysis {
        select_mode(&stats, total_files.max(1))
    } else {
        AnalysisMode::Full
    };

    if incremental_enabled && mode == AnalysisMode::ZeroReuse {
        if let Some(outcome) = super::incremental_fast_path::try_fast_path(&conn, db_path, true)? {
            write_status(&conn, AnalysisMode::ZeroReuse, true, all_segment_ids())?;
            emit_analysis_mode(AnalysisMode::ZeroReuse, 0, TrustLevel::L0, &[], &[]);
            return Ok(PipelineOutcome {
                report: outcome.report,
                mode: AnalysisMode::ZeroReuse,
                delta_files: 0,
            });
        }
    }

    write_status(&conn, mode, false, all_segment_ids())?;

    let delta = AnalysisDelta::from_stats(&stats);
    let structure_fp = compute_structure_fingerprint(&conn)?;
    let delta_fp = compute_delta_fingerprint(&stats);
    let prior_fp = read_structure_fingerprint(&conn)?;
    let trust = classify_trust(
        &stats,
        total_files.max(1),
        &structure_fp,
        prior_fp.as_deref(),
        options.force_full_analysis || mode == AnalysisMode::Full,
    );

    emit_analysis_mode(
        mode,
        delta.file_count(),
        trust.level,
        &segment_names(&trust.segments_to_reuse),
        &segment_names(&trust.segments_to_run),
    );

    if incremental_enabled && mode == AnalysisMode::SegmentedSync && trust.level == TrustLevel::L1 {
        if let Some(report) =
            l1_merge::try_l1_fast_merge(&conn, db_path, &structure_fp, &delta, config.clone())?
        {
            let mut report = report;
            stamp_query_trust_metadata(&mut report, trust.level, &delta, &delta_fp);
            write_analysis_cache(&conn, &report, &stats)?;
            write_segment_caches(&conn, &report, &structure_fp)?;
            write_structure_fingerprint(&conn, &structure_fp)?;
            write_status(&conn, mode, true, Vec::new())?;
            return Ok(PipelineOutcome {
                report,
                mode,
                delta_files: delta.file_count(),
            });
        }
    }

    let skip_git =
        mode == AnalysisMode::SegmentedSync && delta.file_count() <= SEGMENTED_SMALL_FILE_THRESHOLD;
    let effective_git_dir = if skip_git { None } else { git_dir };

    if incremental_enabled && mode == AnalysisMode::SegmentedSync && trust.level == TrustLevel::L2 {
        if let Some(prior_fp) = prior_fp.as_deref() {
            if let Some(report) = l2_merge::try_l2_partial_merge(
                &conn,
                db_path,
                &structure_fp,
                prior_fp,
                &trust,
                &delta,
                config.clone(),
                norms_path,
                effective_git_dir,
                overrides,
            )? {
                let mut report = report;
                stamp_query_trust_metadata(&mut report, trust.level, &delta, &delta_fp);
                write_analysis_cache(&conn, &report, &stats)?;
                write_segment_caches(&conn, &report, &structure_fp)?;
                write_structure_fingerprint(&conn, &structure_fp)?;
                write_status(&conn, mode, true, Vec::new())?;
                return Ok(PipelineOutcome {
                    report,
                    mode,
                    delta_files: delta.file_count(),
                });
            }
        }
    }

    let _invalidated = invalidate_segments(&trust);

    let mut report = full::run_full(
        db_path,
        config,
        norms_path,
        effective_git_dir,
        overrides,
        Some(&conn),
    )?;

    if trust.level == TrustLevel::L2
        && !report
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

    report.analysis_provenance = Some(AnalysisProvenance {
        analysis_mode: format!("{mode:?}").to_ascii_lowercase(),
        trust_level: trust.level.as_str().to_string(),
        structure_fingerprint: structure_fp.clone(),
        structure_changed: trust.structure_changed,
        segments_reused: segment_names(&trust.segments_to_reuse),
        segments_rerun: segment_names(&trust.segments_to_run),
        changed_paths: delta.changed_paths.clone(),
        delta_fingerprint: delta_fp.clone(),
        query_trust_note: None,
    });
    stamp_query_trust_metadata(&mut report, trust.level, &delta, &delta_fp);

    if incremental_enabled {
        write_analysis_cache(&conn, &report, &stats)?;
        write_segment_caches(&conn, &report, &structure_fp)?;
        write_structure_fingerprint(&conn, &structure_fp)?;
    }

    write_status(&conn, mode, true, Vec::new())?;
    Ok(PipelineOutcome {
        report,
        mode,
        delta_files: delta.file_count(),
    })
}

fn write_segment_caches(
    conn: &Connection,
    report: &HealthReport,
    structure_fp: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    for (segment, payload) in merge::extract_typed_segment_payloads(report) {
        if segment == AnalysisSegment::HealthScore {
            continue;
        }
        write_segment_cache(
            conn,
            SegmentCacheRow {
                segment_id: segment.as_str().to_string(),
                graph_fingerprint: structure_fp.to_string(),
                payload_json: serde_json::to_string(&payload)?,
                updated_at: now.clone(),
            },
        )?;
    }
    let health_payload = merge::HealthScoreSegmentPayload {
        report: report.clone(),
    };
    write_segment_cache(
        conn,
        SegmentCacheRow {
            segment_id: AnalysisSegment::HealthScore.as_str().to_string(),
            graph_fingerprint: structure_fp.to_string(),
            payload_json: serde_json::to_string(&health_payload)?,
            updated_at: now,
        },
    )?;
    Ok(())
}

fn stamp_query_trust_metadata(
    report: &mut HealthReport,
    trust_level: TrustLevel,
    delta: &AnalysisDelta,
    delta_fp: &str,
) {
    if let Some(provenance) = report.analysis_provenance.as_mut() {
        provenance.delta_fingerprint = delta_fp.to_string();
    } else {
        report.analysis_provenance = Some(AnalysisProvenance {
            analysis_mode: String::new(),
            trust_level: trust_level.as_str().to_string(),
            structure_fingerprint: String::new(),
            structure_changed: false,
            segments_reused: Vec::new(),
            segments_rerun: Vec::new(),
            changed_paths: delta.changed_paths.clone(),
            delta_fingerprint: delta_fp.to_string(),
            query_trust_note: None,
        });
    }

    if delta.changed_paths.is_empty() {
        return;
    }

    let note = match trust_level {
        TrustLevel::L1 => Some(
            "Symbols in changed_paths may have stale complexity/dead-code findings until --full-analysis."
                .to_string(),
        ),
        TrustLevel::L2 => Some(
            "Call graph topology changed; wiring-sensitive findings were rerun. Symbols in changed_paths may still be stale for body-level metrics until --full-analysis."
                .to_string(),
        ),
        _ => None,
    };

    if let Some(note) = note {
        if let Some(provenance) = report.analysis_provenance.as_mut() {
            provenance.query_trust_note = Some(note);
        }
    }
}

fn record_segment_progress(conn: &Connection, segment: AnalysisSegment) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let sid = segment.as_str().to_string();
    let mut record =
        graphengine_parsing::infrastructure::storage::parse_meta_store::read_parse_meta(
            conn,
            ANALYSIS_STATUS_KEY,
        )?
        .and_then(|raw| serde_json::from_str::<AnalysisStatusRecord>(&raw).ok())
        .unwrap_or_else(|| AnalysisStatusRecord {
            mode: String::new(),
            complete: false,
            segments_done: Vec::new(),
            segments_pending: all_segment_ids()
                .iter()
                .map(|s| s.as_str().to_string())
                .collect(),
            started_at: now.clone(),
            updated_at: now.clone(),
        });

    if !record.segments_done.contains(&sid) {
        record.segments_done.push(sid.clone());
    }
    record.segments_pending.retain(|s| s != &sid);
    record.updated_at = now;
    let json = serde_json::to_string(&record)?;
    graphengine_parsing::infrastructure::storage::parse_meta_store::upsert_parse_meta(
        conn,
        ANALYSIS_STATUS_KEY,
        &json,
    )?;
    Ok(())
}

fn write_status(
    conn: &Connection,
    mode: AnalysisMode,
    complete: bool,
    pending: Vec<AnalysisSegment>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let done: Vec<String> = if complete {
        all_segment_ids()
            .iter()
            .map(|s| s.as_str().to_string())
            .collect()
    } else {
        Vec::new()
    };
    let pending: Vec<String> = pending.iter().map(|s| s.as_str().to_string()).collect();
    let record = AnalysisStatusRecord {
        mode: format!("{mode:?}").to_ascii_lowercase(),
        complete,
        segments_done: done,
        segments_pending: pending,
        started_at: now.clone(),
        updated_at: now,
    };
    let json = serde_json::to_string(&record)?;
    graphengine_parsing::infrastructure::storage::parse_meta_store::upsert_parse_meta(
        conn,
        ANALYSIS_STATUS_KEY,
        &json,
    )?;
    Ok(())
}

pub fn emit_analysis_mode(
    mode: AnalysisMode,
    delta_files: usize,
    trust_level: TrustLevel,
    segments_reused: &[String],
    segments_rerun: &[String],
) {
    use graphengine_progress::EngineEvent;
    let (label, message) = match (mode, trust_level) {
        (AnalysisMode::ZeroReuse, TrustLevel::L0) => (
            "zero_reuse",
            "Analysis: cached (no file changes)".to_string(),
        ),
        (AnalysisMode::SegmentedSync, TrustLevel::L1) => (
            "segmented_sync",
            format!(
                "Analysis: fast merge L1 — call graph unchanged, reused [{}]",
                segments_reused.join(", ")
            ),
        ),
        (AnalysisMode::SegmentedSync, TrustLevel::L2) => (
            "segmented_sync",
            format!("Analysis: partial L2 — call graph changed ({delta_files} files)"),
        ),
        (AnalysisMode::Full, _) | (_, TrustLevel::L3) => ("full", "Analysis: full".to_string()),
        (AnalysisMode::Background, _) => (
            "background",
            format!("background segments (rerun: {})", segments_rerun.join(", ")),
        ),
        _ => ("segmented_sync", format!("segmented ({delta_files} files)")),
    };
    graphengine_progress::emit_line(&EngineEvent::AnalysisMode {
        mode: label.to_string(),
        delta_files,
        message,
    })
    .ok();
}
