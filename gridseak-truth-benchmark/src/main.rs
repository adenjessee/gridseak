//! `make bench` entry: sample G1 scan artifacts and write ECE JSON.

use graphengine_parsing::domain::p_true_from_provenance_json;
use graphengine_parsing::domain::ProvenanceSource;
use gridseak_truth_benchmark::{ece, pr_f1, prior_call_p_true, ECE_RELEASE_GATE};
use rusqlite::Connection;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct SliceReport {
    status: String,
    ece_gate: f64,
    languages: Vec<LangSlice>,
    notes: Vec<String>,
}

#[derive(Serialize)]
struct LangSlice {
    language: String,
    scan_id: Option<String>,
    artifact: String,
    sampled: usize,
    ece: Option<f64>,
    precision: Option<f64>,
    recall: Option<f64>,
    f1: Option<f64>,
    contamination: String,
}

fn sample_artifact(path: &Path, language: &str) -> Option<LangSlice> {
    if !path.exists() {
        return None;
    }
    let conn = Connection::open(path).ok()?;
    let scan_id = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'scan_id' LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let mut stmt = conn
        .prepare(
            "SELECT provenance FROM edges WHERE json_extract(kind, '$.kind') = 'Call' LIMIT 400",
        )
        .ok()?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();
    let mut tp = 0.0;
    let mut fp = 0.0;
    let mut fn_ = 0.0;
    for raw in &rows {
        let p = p_true_from_provenance_json(language, raw, "Call").unwrap_or(0.45);
        let oracle_true = raw.contains("\"Compiler\"") || raw.contains("\"Lsp\"");
        pairs.push((p, oracle_true));
        if oracle_true && p >= 0.6 {
            tp += 1.0;
        } else if !oracle_true && p >= 0.6 {
            fp += 1.0;
        } else if oracle_true && p < 0.6 {
            fn_ += 1.0;
        }
    }
    let metrics = pr_f1(tp, fp, fn_);
    Some(LangSlice {
        language: language.into(),
        scan_id,
        artifact: path.display().to_string(),
        sampled: pairs.len(),
        ece: Some(ece(&pairs, 10)),
        precision: Some(metrics.precision),
        recall: Some(metrics.recall),
        f1: Some(metrics.f1),
        contamination:
            "oracle is the same SCIP/LSP definition already on the edge; foreign LS subset not run"
                .into(),
    })
}

fn main() {
    println!("gridseak-truth-benchmark");
    println!("ece_gate={}", ECE_RELEASE_GATE);
    for lang in ["typescript", "go", "python", "rust"] {
        for src in [
            ProvenanceSource::Compiler,
            ProvenanceSource::Lsp,
            ProvenanceSource::Heuristic,
        ] {
            println!(
                "p_true {lang} {:?} Call = {:.2}",
                src,
                prior_call_p_true(lang, src)
            );
        }
    }

    let ts = std::env::var("GRIDSEAK_G3_TS_ARTIFACT").ok();
    let go = std::env::var("GRIDSEAK_G3_GO_ARTIFACT").ok();
    let mut languages = Vec::new();
    let mut notes = vec![
        "G3 slice: ≥100 Call sites per G1 language when artifacts are present.".into(),
        "Oracle contamination disclosed: SCIP definition already on the edge.".into(),
    ];
    if let Some(p) = ts.as_deref() {
        if let Some(s) = sample_artifact(Path::new(p), "typescript") {
            languages.push(s);
        } else {
            notes.push(format!("TS artifact unreadable: {p}"));
        }
    } else {
        notes.push("GRIDSEAK_G3_TS_ARTIFACT unset".into());
    }
    if let Some(p) = go.as_deref() {
        if let Some(s) = sample_artifact(Path::new(p), "go") {
            languages.push(s);
        } else {
            notes.push(format!("Go artifact unreadable: {p}"));
        }
    } else {
        notes.push("GRIDSEAK_G3_GO_ARTIFACT unset".into());
    }

    let mut worst = 0.0f64;
    let mut ready = true;
    for s in &languages {
        if s.sampled < 100 {
            ready = false;
            notes.push(format!(
                "{} sampled {} < 100 — slice incomplete",
                s.language, s.sampled
            ));
        }
        if let Some(e) = s.ece {
            worst = worst.max(e);
        }
    }
    let status = if languages.is_empty() {
        "no_g1_artifact"
    } else if !ready {
        "incomplete_slice"
    } else if worst > ECE_RELEASE_GATE {
        "ece_fail"
    } else {
        "ok"
    };
    let report = SliceReport {
        status: status.into(),
        ece_gate: ECE_RELEASE_GATE,
        languages,
        notes,
    };
    let json = serde_json::to_string_pretty(&report).expect("json");
    let out = std::path::Path::new("target/g3-slice.json");
    let _ = std::fs::create_dir_all("target");
    std::fs::write(out, &json).expect("write g3-slice.json");
    println!("{json}");
    if status == "ece_fail" {
        std::process::exit(1);
    }
}
