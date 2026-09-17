use super::control::run_control;
use super::oracle::ArtifactOracle;
use super::task::{Task, TaskKind};
use super::treatment::{default_artifact, default_corpus_root, run_treatment, ArmDecision};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ScoredRow {
    pub task_id: String,
    pub kind: TaskKind,
    pub language: String,
    pub symbol: Option<String>,
    pub gold_action: Option<String>,
    pub control: ArmDecision,
    pub treatment: ArmDecision,
    pub ablation_treesitter: ArmDecision,
    pub control_correct: bool,
    pub treatment_correct: bool,
    pub ablation_correct: bool,
    pub scan_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShineRates {
    pub false_delete_control: f64,
    pub false_delete_treatment: f64,
    pub wrong_twin_control: f64,
    pub wrong_twin_treatment: f64,
    pub treatment_beats_false_delete: bool,
    pub treatment_beats_wrong_twin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessReport {
    pub model: String,
    pub arms: usize,
    pub tasks: usize,
    pub rows: Vec<ScoredRow>,
    pub shine: ShineRates,
    pub control_accuracy: f64,
    pub treatment_accuracy: f64,
    pub ablation_accuracy: f64,
    pub share_task_id: Option<String>,
    pub headline: String,
    pub notes: Vec<String>,
}

fn correct(task: &Task, d: &ArmDecision, oracle: &ArtifactOracle) -> bool {
    match task.kind {
        TaskKind::SafeDelete => d.action == "refuse_delete",
        TaskKind::WrongTwin => {
            let gold = task.correct_file.as_deref().unwrap_or("");
            if gold == "stdlib" {
                return d.action.contains("stdlib")
                    || task.wrong_twin_files.iter().all(|w| !d.action.contains(w));
            }
            d.action.contains(gold) && task.wrong_twin_files.iter().all(|w| !d.action.contains(w))
        }
        TaskKind::Blast | TaskKind::DiffImpact => task
            .gold_blast_contains
            .iter()
            .all(|need| d.cited.iter().any(|c| c.contains(need))),
        TaskKind::FalseClaim => {
            let want = task.gold_verdict.as_deref().unwrap_or("refuted");
            d.claim_verdict.as_deref() == Some(want) || d.action == "refute"
        }
        TaskKind::Cycle => {
            let needle = task.symbol.as_deref().unwrap_or("");
            let gold = oracle
                .in_cycle(needle)
                .map(|v| v.verdict)
                .unwrap_or_else(|| "unknown".into());
            d.claim_verdict.as_deref() == Some(gold.as_str()) && d.action == "use_oracle"
        }
        TaskKind::Honesty => d.unknown || d.action == "unknown",
    }
}

fn rate(rows: &[ScoredRow], kind: TaskKind, pred: fn(&ScoredRow) -> bool) -> f64 {
    let set: Vec<&ScoredRow> = rows.iter().filter(|r| r.kind == kind).collect();
    if set.is_empty() {
        return 0.0;
    }
    set.iter().filter(|r| pred(r)).count() as f64 / set.len() as f64
}

pub fn run_harness(tasks: &[Task], model: &str) -> anyhow::Result<HarnessReport> {
    let mut rows = Vec::new();
    let mut notes = Vec::new();
    for task in tasks {
        let Some(art_path) = default_artifact(&task.corpus) else {
            notes.push(format!("{}: no artifact mapping", task.id));
            continue;
        };
        if !art_path.exists() {
            notes.push(format!(
                "{}: missing artifact {}",
                task.id,
                art_path.display()
            ));
            continue;
        }
        let Some(root) = default_corpus_root(&task.corpus) else {
            notes.push(format!("{}: missing corpus checkout", task.id));
            continue;
        };
        let oracle = ArtifactOracle::open(&art_path)?;
        let scan_id = oracle.scan_id();
        let control = run_control(task, &root);
        let treatment = run_treatment(task, &oracle, false);
        let ablation = run_treatment(task, &oracle, true);
        let control_correct = correct(task, &control, &oracle);
        let treatment_correct = correct(task, &treatment, &oracle);
        let ablation_correct = correct(task, &ablation, &oracle);
        rows.push(ScoredRow {
            task_id: task.id.clone(),
            kind: task.kind,
            language: task.language.clone(),
            symbol: task.symbol.clone(),
            gold_action: task.gold_action.clone(),
            control,
            treatment,
            ablation_treesitter: ablation,
            control_correct,
            treatment_correct,
            ablation_correct,
            scan_id,
        });
    }
    anyhow::ensure!(
        !rows.is_empty(),
        "no scored rows (artifacts/corpora missing)"
    );

    let n = rows.len() as f64;
    let shine = ShineRates {
        false_delete_control: rate(&rows, TaskKind::SafeDelete, |r| {
            r.control.action == "approve_delete"
        }),
        false_delete_treatment: rate(&rows, TaskKind::SafeDelete, |r| {
            r.treatment.action == "approve_delete"
        }),
        wrong_twin_control: rate(&rows, TaskKind::WrongTwin, |r| !r.control_correct),
        wrong_twin_treatment: rate(&rows, TaskKind::WrongTwin, |r| !r.treatment_correct),
        treatment_beats_false_delete: false,
        treatment_beats_wrong_twin: false,
    };
    let mut shine = shine;
    shine.treatment_beats_false_delete = shine.false_delete_treatment < shine.false_delete_control;
    shine.treatment_beats_wrong_twin = shine.wrong_twin_treatment < shine.wrong_twin_control;

    let share = rows.iter().find(|r| {
        r.kind == TaskKind::SafeDelete
            && r.control.action == "approve_delete"
            && r.treatment.action == "refuse_delete"
    });

    let headline = if let Some(r) = share {
        format!(
            "The other run said it was unused. GridSeak's compiler said callers on `{}`. I would have shipped a production break.",
            r.symbol.as_deref().unwrap_or(&r.task_id)
        )
    } else {
        "No scored control false-delete; do not film a win.".into()
    };

    Ok(HarnessReport {
        model: model.into(),
        arms: 2,
        tasks: rows.len(),
        control_accuracy: rows.iter().filter(|r| r.control_correct).count() as f64 / n,
        treatment_accuracy: rows.iter().filter(|r| r.treatment_correct).count() as f64 / n,
        ablation_accuracy: rows.iter().filter(|r| r.ablation_correct).count() as f64 / n,
        share_task_id: share.map(|r| r.task_id.clone()),
        shine,
        headline,
        notes,
        rows,
    })
}

pub fn render_share_comment(report: &HarnessReport) -> String {
    let mut out = String::from("<!-- gridseak-pr-comment -->\n## GridSeak structural review\n\n");
    out.push_str(&format!("**Headline:** {}\n\n", report.headline));
    out.push_str("### Agent A/B (deterministic grep control vs GridSeak treatment)\n\n");
    out.push_str(&format!(
        "- Control accuracy: {:.2}\n- Treatment accuracy: {:.2}\n- Tree-sitter-only ablation: {:.2}\n",
        report.control_accuracy, report.treatment_accuracy, report.ablation_accuracy
    ));
    out.push_str(&format!(
        "- False-delete rate control/treatment: {:.2} / {:.2}\n- Wrong-twin error control/treatment: {:.2} / {:.2}\n\n",
        report.shine.false_delete_control,
        report.shine.false_delete_treatment,
        report.shine.wrong_twin_control,
        report.shine.wrong_twin_treatment
    ));
    if let Some(id) = &report.share_task_id {
        if let Some(r) = report.rows.iter().find(|x| x.task_id == *id) {
            out.push_str(&format!(
                "### Share task `{id}` (`{}`)\nThe control approved a delete. Treatment ran `no_callers` and **refused** (verdict `{}`).\n\nCompiler-tier callers (first 5):\n",
                r.symbol.as_deref().unwrap_or("?"),
                r.treatment.claim_verdict.as_deref().unwrap_or("?")
            ));
            for w in r.treatment.cited.iter().take(5) {
                out.push_str(&format!("- `{w}`\n"));
            }
        }
    }
    out.push_str(
        "\nTiers: treatment quotes the scan artifact (Compiler Call edges). Control is grep.\n",
    );
    out
}

pub fn write_clip_script(report: &HarnessReport, dest: &Path) -> anyhow::Result<()> {
    let body = format!(
        "# 90s clip storyboard (from scored A/B, not a marketing fiction)\n\n\
         0–15s: Same prompt on {}.\n\
         15–40s: Control greps the name and says unused / edits the twin.\n\
         40–70s: Treatment `verify_claim` / callers; names the compiler-tier witnesses.\n\
         70–90s: Super: \"{}\"\n\n\
         Do not film if treatment_beats_false_delete is false.\n\
         Beats false-delete: {}\n\
         Beats wrong-twin: {}\n",
        report.share_task_id.as_deref().unwrap_or("(none)"),
        report.headline,
        report.shine.treatment_beats_false_delete,
        report.shine.treatment_beats_wrong_twin
    );
    std::fs::write(dest, body)?;
    Ok(())
}
