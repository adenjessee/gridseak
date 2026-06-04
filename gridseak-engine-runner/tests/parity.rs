//! Parity test: running the same fixture through the runner twice in a
//! row must produce reports that are byte-identical after masking the
//! few fields known to vary across runs (timestamps, durations, the
//! per-scan db path).
//!
//! Why this exists: the entire reason the runner crate was extracted
//! is to make it impossible for the CLI and the desktop to emit
//! different metric numbers for the same input. The strongest possible
//! local proof of that is determinism of the runner itself — if two
//! runs of the runner are byte-identical, then any two consumers of
//! the runner *must* be byte-identical too (they share the function).
//!
//! What we mask:
//! - `generated_at`        — wall-clock timestamp.
//! - `analysis_duration_ms` — wall-clock-derived.
//! - `db_path`             — contains the scan-id, which is per-run.
//!
//! What we do **not** mask:
//! - Anything else. Findings, metrics, classifications, summary, all
//!   of it should hash identically. If a future change introduces a
//!   non-deterministic field elsewhere, this test should fail loudly
//!   and force us to mask it on purpose with a comment explaining why.

mod common;

use common::{ensure_engine_binaries, pipeline_config};
use gridseak_engine_runner::run_pipeline;
use serde_json::Value;
use uuid::Uuid;

#[tokio::test]
async fn two_runs_of_the_runner_produce_identical_reports() {
    ensure_engine_binaries();

    let scratch = tempfile::tempdir().expect("create scratch dir");

    // Run #1
    let scan_a = Uuid::new_v4();
    let cfg_a = pipeline_config(
        scratch.path(),
        scan_a,
        vec!["rust".into(), "typescript".into()],
    );
    let out_a = run_pipeline(cfg_a)
        .await
        .expect("run #1 of runner pipeline succeeds");

    // Run #2 — fresh scan_id, same inputs.
    let scan_b = Uuid::new_v4();
    let cfg_b = pipeline_config(
        scratch.path(),
        scan_b,
        vec!["rust".into(), "typescript".into()],
    );
    let out_b = run_pipeline(cfg_b)
        .await
        .expect("run #2 of runner pipeline succeeds");

    // Both runs claim to have parsed the same languages in the same
    // order. (Order matters: the analyzer's behavior depends on which
    // language seeded the SQLite DB with `--clear`.)
    assert_eq!(out_a.languages_parsed, out_b.languages_parsed);
    assert_eq!(out_a.languages_skipped, out_b.languages_skipped);

    let mut a = serde_json::to_value(&out_a.report).expect("serialize report a");
    let mut b = serde_json::to_value(&out_b.report).expect("serialize report b");

    mask_volatile(&mut a);
    mask_volatile(&mut b);

    if a != b {
        // Surface the first divergence so failures are debuggable
        // without manually diffing two ~1000-line JSON blobs.
        let diff = first_divergence(&a, &b, "$");
        panic!(
            "runner reports diverged after masking volatile fields: {diff}\n\
             scan_a = {scan_a}\nscan_b = {scan_b}"
        );
    }

    // Sanity: both reports have *some* findings or *some* metric data.
    // A trivially empty report would pass the equality check above
    // even if the parser silently skipped the fixture.
    let findings = a.get("findings").and_then(Value::as_array);
    let metrics = a.get("metrics");
    assert!(
        findings.map(|f| !f.is_empty()).unwrap_or(false) || metrics.is_some(),
        "report has neither findings nor metrics — fixture probably wasn't parsed"
    );
}

/// Replace fields known to vary across runs with a fixed sentinel so
/// the byte-equality check above is meaningful.
fn mask_volatile(v: &mut Value) {
    if let Value::Object(obj) = v {
        if obj.contains_key("generated_at") {
            obj.insert("generated_at".into(), Value::String("<masked>".into()));
        }
        if obj.contains_key("analysis_duration_ms") {
            obj.insert("analysis_duration_ms".into(), Value::from(0u64));
        }
        if obj.contains_key("db_path") {
            obj.insert("db_path".into(), Value::String("<masked>".into()));
        }
        for child in obj.values_mut() {
            mask_volatile(child);
        }
    } else if let Value::Array(arr) = v {
        for child in arr.iter_mut() {
            mask_volatile(child);
        }
    }
}

/// Walk both values in lockstep and return a `$.foo.bar[0]`-style path
/// to the first scalar that differs, plus the two values at that path.
/// Used only on the failure path; the happy path never builds a string.
fn first_divergence(a: &Value, b: &Value, path: &str) -> String {
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let keys: std::collections::BTreeSet<_> = am.keys().chain(bm.keys()).collect();
            for k in keys {
                let next = format!("{path}.{k}");
                match (am.get(k), bm.get(k)) {
                    (Some(av), Some(bv)) if av == bv => continue,
                    (Some(av), Some(bv)) => return first_divergence(av, bv, &next),
                    (Some(_), None) => return format!("{next}: present in a, missing in b"),
                    (None, Some(_)) => return format!("{next}: missing in a, present in b"),
                    (None, None) => unreachable!(),
                }
            }
            format!("{path}: objects compared equal during walk but != at root")
        }
        (Value::Array(av), Value::Array(bv)) => {
            if av.len() != bv.len() {
                return format!("{path}: array length {} vs {}", av.len(), bv.len());
            }
            for (i, (ae, be)) in av.iter().zip(bv.iter()).enumerate() {
                if ae != be {
                    return first_divergence(ae, be, &format!("{path}[{i}]"));
                }
            }
            format!("{path}: arrays compared equal during walk but != at root")
        }
        (a, b) => format!(
            "{path}: scalar diverged\n  a = {}\n  b = {}",
            serde_json::to_string(a).unwrap_or_default(),
            serde_json::to_string(b).unwrap_or_default()
        ),
    }
}
