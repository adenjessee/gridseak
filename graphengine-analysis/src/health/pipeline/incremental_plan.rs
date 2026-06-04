//! Dry-run incremental trust plan (Tier A status) without running analyze.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use graphengine_parsing::infrastructure::storage::parse_meta_store::{
    compute_structure_fingerprint, read_incremental_scan_stats, read_structure_fingerprint,
    IncrementalScanStats,
};

use super::merge::read_health_score_cache;
use super::scope::{
    classify_trust, segment_names, trust_decision_for_workspace_rescan, TrustLevel,
};
use super::{SEGMENTED_RATIO_THRESHOLD, SEGMENTED_SMALL_FILE_THRESHOLD};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncrementalPlan {
    pub predicted_trust_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_fp_match: Option<bool>,
    pub changed_paths_parse: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_paths_workspace: Vec<String>,
    pub segments_to_reuse: Vec<String>,
    pub segments_to_rerun: Vec<String>,
    pub cache_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
}

pub fn predict_incremental_plan(
    conn: &Connection,
    workspace_dirty_paths: Option<&[String]>,
) -> Result<IncrementalPlan> {
    let mut stats = read_incremental_scan_stats(conn)?.unwrap_or(IncrementalScanStats {
        cached: 0,
        reparsed: 0,
        removed: 0,
        plan_disabled: false,
        changed_paths: Vec::new(),
        removed_paths: Vec::new(),
    });

    let workspace_dirty: Vec<String> = workspace_dirty_paths
        .map(|p| p.to_vec())
        .unwrap_or_default();

    let rescan_pending = !workspace_dirty.is_empty() && stats.is_zero_delta();
    if rescan_pending {
        stats.reparsed = workspace_dirty.len();
        stats.changed_paths = workspace_dirty.clone();
    }

    let total_files = (stats.cached + stats.reparsed).max(1);
    let prior_fp = read_structure_fingerprint(conn)?;
    let current_fp = compute_structure_fingerprint(conn)?;

    let structure_fp_match = if rescan_pending {
        None
    } else {
        prior_fp.as_ref().map(|prior| prior == &current_fp)
    };

    let trust = if rescan_pending {
        trust_decision_for_workspace_rescan()
    } else {
        classify_trust(&stats, total_files, &current_fp, prior_fp.as_deref(), false)
    };

    let cache_ready = segment_cache_ready(conn, &trust, &current_fp, prior_fp.as_deref());

    let stale_reason = stale_reason_for(&trust, &stats, rescan_pending, prior_fp.is_none());

    Ok(IncrementalPlan {
        predicted_trust_level: if prior_fp.is_none() && !stats.is_zero_delta() {
            "unknown".into()
        } else {
            trust.level.as_str().into()
        },
        structure_fp_match,
        changed_paths_parse: stats.changed_paths.clone(),
        changed_paths_workspace: workspace_dirty,
        segments_to_reuse: segment_names(&trust.segments_to_reuse),
        segments_to_rerun: segment_names(&trust.segments_to_run),
        cache_ready,
        stale_reason,
    })
}

fn segment_cache_ready(
    conn: &Connection,
    trust: &super::scope::TrustDecision,
    current_fp: &str,
    prior_fp: Option<&str>,
) -> bool {
    match trust.level {
        TrustLevel::L0 | TrustLevel::L1 => read_health_score_cache(conn, current_fp)
            .ok()
            .flatten()
            .is_some(),
        TrustLevel::L2 => prior_fp
            .and_then(|fp| read_health_score_cache(conn, fp).ok().flatten())
            .is_some(),
        TrustLevel::L3 => false,
    }
}

fn stale_reason_for(
    trust: &super::scope::TrustDecision,
    stats: &IncrementalScanStats,
    rescan_pending: bool,
    no_prior_fp: bool,
) -> Option<String> {
    if rescan_pending {
        return Some("rescan_required".into());
    }
    if no_prior_fp && !stats.is_zero_delta() {
        return Some("no_prior_scan".into());
    }
    if trust.level == TrustLevel::L3 {
        let delta = stats.delta_file_count();
        if delta > SEGMENTED_SMALL_FILE_THRESHOLD {
            return Some("large_delta".into());
        }
        if stats.cached + stats.reparsed > 0 {
            let ratio = delta as f64 / (stats.cached + stats.reparsed) as f64;
            if ratio > SEGMENTED_RATIO_THRESHOLD {
                return Some("large_delta".into());
            }
        }
    }
    if trust.structure_changed && trust.level == TrustLevel::L2 {
        return Some("structure_changed".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::pipeline::cache::{write_segment_cache, SegmentCacheRow};
    use crate::health::pipeline::merge::HealthScoreSegmentPayload;
    use crate::health::pipeline::AnalysisSegment;
    use graphengine_parsing::infrastructure::storage::parse_meta_store::{
        merge_incremental_scan_stats, write_structure_fingerprint,
    };

    fn build_seeded_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
               location TEXT NOT NULL, provenance TEXT NOT NULL, properties TEXT NOT NULL DEFAULT '{}',
               trait_metadata TEXT);
             CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL,
               provenance TEXT NOT NULL, PRIMARY KEY (from_id, to_id, kind));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes VALUES ('a', 'Function', 'mod::foo', '{}', '{}', '{}', NULL),
             ('b', 'Function', 'mod::bar', '{}', '{}', '{}', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges VALUES ('a', 'b', '{\"kind\":\"Call\"}', '{}')",
            [],
        )
        .unwrap();

        let fp = compute_structure_fingerprint(&conn).unwrap();
        write_structure_fingerprint(&conn, &fp).unwrap();
        let report = crate::health::run_analysis(path.to_str().unwrap()).expect("seed analysis");
        write_segment_cache(
            &conn,
            SegmentCacheRow {
                segment_id: AnalysisSegment::HealthScore.as_str().to_string(),
                graph_fingerprint: fp,
                payload_json: serde_json::to_string(&HealthScoreSegmentPayload { report }).unwrap(),
                updated_at: "now".into(),
            },
        )
        .unwrap();
        merge_incremental_scan_stats(
            &conn,
            &IncrementalScanStats {
                cached: 99,
                reparsed: 0,
                removed: 0,
                plan_disabled: false,
                changed_paths: Vec::new(),
                removed_paths: Vec::new(),
            },
        )
        .unwrap();
        (dir, conn)
    }

    #[test]
    fn predicts_l1_when_topology_stable_small_delta() {
        let (_dir, conn) = build_seeded_db();
        merge_incremental_scan_stats(
            &conn,
            &IncrementalScanStats {
                cached: 9,
                reparsed: 1,
                removed: 0,
                plan_disabled: false,
                changed_paths: vec!["src/a.rs".into()],
                removed_paths: Vec::new(),
            },
        )
        .unwrap();

        let plan = predict_incremental_plan(&conn, None).unwrap();
        assert_eq!(plan.predicted_trust_level, "L1");
        assert_eq!(plan.structure_fp_match, Some(true));
        assert!(plan.cache_ready);
        assert!(plan.segments_to_reuse.contains(&"Cycles".to_string()));
    }

    #[test]
    fn workspace_dirty_before_rescan_marks_rescan_required() {
        let (_dir, conn) = build_seeded_db();

        let plan = predict_incremental_plan(&conn, Some(&["src/lib.rs".into()])).unwrap();
        assert_eq!(plan.predicted_trust_level, "L1");
        assert_eq!(plan.structure_fp_match, None);
        assert_eq!(plan.stale_reason.as_deref(), Some("rescan_required"));
    }
}
