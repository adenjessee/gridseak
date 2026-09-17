//! Grep-only control policy (what a typical agent does without GridSeak).

use super::task::{Task, TaskKind};
use super::treatment::ArmDecision;
use std::path::Path;
use std::process::Command;

fn rg_files(root: &Path, pattern: &str, fixed: bool) -> Vec<String> {
    let mut cmd = Command::new("rg");
    cmd.arg("-l").args(["--glob", "!**/.git/**"]);
    if fixed {
        cmd.arg("-F");
    }
    cmd.arg(pattern).current_dir(root);
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.replace('\\', "/"))
        .collect()
}

/// Definition-only grep: the naive "unused?" agent looks for `func X` / a
/// method named X, not call sites. That is how it LGTMs live symbols.
fn definition_files(root: &Path, name: &str, language: &str) -> Vec<String> {
    if name.is_empty() {
        return Vec::new();
    }
    let pat = if language == "go" {
        format!(r"func\s+(\([^)]*\)\s+)?{name}\b")
    } else {
        format!(r"(?m)^\s+{name}\s*[<(]")
    };
    rg_files(root, &pat, false)
        .into_iter()
        .filter(|f| {
            !f.contains("_test.")
                && !f.contains("__tests__")
                && !f.contains("_examples")
                && !f.contains("deno/")
        })
        .collect()
}

fn first_hit<'a>(files: &'a [String], prefer_needles: &[&str]) -> Option<&'a str> {
    for n in prefer_needles {
        if let Some(f) = files.iter().find(|f| f.contains(n)) {
            return Some(f.as_str());
        }
    }
    files.first().map(String::as_str)
}

pub fn run_control(task: &Task, corpus_root: &Path) -> ArmDecision {
    let name = task
        .symbol_name
        .clone()
        .or_else(|| {
            task.symbol
                .as_ref()
                .and_then(|s| s.rsplit("::").next().map(str::to_string))
        })
        .unwrap_or_default();
    let files = if name.is_empty() {
        Vec::new()
    } else {
        rg_files(corpus_root, &name, true)
    };
    let tool_calls = 1u32;
    match task.kind {
        TaskKind::SafeDelete => {
            let defs = definition_files(corpus_root, &name, &task.language);
            // Naive agent: one definition ⇒ "only the declaration, safe to delete".
            if defs.len() <= 1 {
                ArmDecision {
                    arm: "control".into(),
                    action: "approve_delete".into(),
                    cited: defs,
                    claim_verdict: Some("verified".into()),
                    unknown: false,
                    tool_calls,
                    notes: "grep: ≤1 non-test definition, treat as unused".into(),
                }
            } else {
                ArmDecision {
                    arm: "control".into(),
                    action: "refuse_delete".into(),
                    cited: defs,
                    claim_verdict: Some("refuted".into()),
                    unknown: false,
                    tool_calls,
                    notes: "grep: name defined in multiple prod files".into(),
                }
            }
        }
        TaskKind::WrongTwin => {
            let pick = first_hit(&files, &["deno/", "chain.go", "chi.go"]).unwrap_or("");
            ArmDecision {
                arm: "control".into(),
                action: format!("edit:{pick}"),
                cited: files,
                claim_verdict: None,
                unknown: false,
                tool_calls,
                notes: "grep: first/preferred path match".into(),
            }
        }
        TaskKind::Blast | TaskKind::DiffImpact => ArmDecision {
            arm: "control".into(),
            action: "list_files".into(),
            cited: files,
            claim_verdict: None,
            unknown: false,
            tool_calls,
            notes: "grep: files containing the name, not callees".into(),
        },
        TaskKind::FalseClaim => {
            let both_present = task.gold_claim.as_deref().is_some_and(|c| {
                c.contains("calls") && files.len() >= 1
                    || rg_files(corpus_root, &name, true).len() > 1
            });
            ArmDecision {
                arm: "control".into(),
                action: "accept".into(),
                cited: files,
                claim_verdict: Some(if both_present {
                    "verified".into()
                } else {
                    "verified".into()
                }),
                unknown: false,
                tool_calls,
                notes: "grep agent accepts a fluent structural claim when names exist".into(),
            }
        }
        TaskKind::Cycle => ArmDecision {
            arm: "control".into(),
            action: "no_cycle".into(),
            cited: files,
            claim_verdict: Some("refuted".into()),
            unknown: false,
            tool_calls,
            notes: "grep cannot see cycles; default no".into(),
        },
        TaskKind::Honesty => ArmDecision {
            arm: "control".into(),
            action: "state_fact".into(),
            cited: files,
            claim_verdict: Some("verified".into()),
            unknown: false,
            tool_calls,
            notes: "model states a single concrete callee as fact".into(),
        },
    }
}
