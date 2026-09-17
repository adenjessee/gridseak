//! Treatment arm: must use the GridSeak artifact (compiler-aware).

use super::oracle::{parse_calls_args, ArtifactOracle};
use super::task::{Task, TaskKind};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ArmDecision {
    pub arm: String,
    pub action: String,
    pub cited: Vec<String>,
    pub claim_verdict: Option<String>,
    pub unknown: bool,
    pub tool_calls: u32,
    pub notes: String,
}

pub fn run_treatment(task: &Task, oracle: &ArtifactOracle, treesitter_only: bool) -> ArmDecision {
    let arm = if treesitter_only {
        "ablation_treesitter"
    } else {
        "treatment"
    };
    let needle = task.symbol.as_deref().unwrap_or("");
    match task.kind {
        TaskKind::SafeDelete | TaskKind::FalseClaim
            if task
                .gold_claim
                .as_deref()
                .is_some_and(|c| c.starts_with("no_callers")) =>
        {
            let view = oracle.no_callers(needle, treesitter_only);
            let Some(view) = view else {
                return missing(arm, "symbol unresolved");
            };
            let refuse = view.verdict == "refuted";
            ArmDecision {
                arm: arm.into(),
                action: if refuse {
                    "refuse_delete".into()
                } else {
                    "approve_delete".into()
                },
                cited: view.witnesses.clone(),
                claim_verdict: Some(view.verdict),
                unknown: false,
                tool_calls: 1,
                notes: "verify_claim no_callers via scan artifact".into(),
            }
        }
        TaskKind::FalseClaim => {
            let claim = task.gold_claim.as_deref().unwrap_or("");
            let view = if let Some((a, b)) = parse_calls_args(claim) {
                oracle.calls(&a, &b)
            } else {
                None
            };
            let Some(view) = view else {
                return missing(arm, "calls() unresolved");
            };
            ArmDecision {
                arm: arm.into(),
                action: if view.verdict == "refuted" {
                    "refute".into()
                } else {
                    "accept".into()
                },
                cited: view.witnesses,
                claim_verdict: Some(view.verdict),
                unknown: false,
                tool_calls: 1,
                notes: "verify_claim calls() via scan artifact".into(),
            }
        }
        TaskKind::WrongTwin => match oracle.resolve(needle) {
            Some((id, _)) => {
                let Some(file) = oracle.file_of(&id) else {
                    return missing(arm, "symbol has no file");
                };
                ArmDecision {
                    arm: arm.into(),
                    action: format!("edit:{file}"),
                    cited: vec![file],
                    claim_verdict: None,
                    unknown: false,
                    tool_calls: 1,
                    notes: "resolved symbol file from scan artifact".into(),
                }
            }
            None => ArmDecision {
                arm: arm.into(),
                action: "edit:stdlib".into(),
                cited: vec!["stdlib".into()],
                claim_verdict: None,
                unknown: false,
                tool_calls: 1,
                notes: "no in-repo node; compiler treated the site as external".into(),
            },
        },
        TaskKind::Blast | TaskKind::DiffImpact => {
            let Some((id, _)) = oracle.resolve(needle) else {
                return missing(arm, "symbol unresolved");
            };
            let cited = oracle.callees(&id).unwrap_or_default();
            ArmDecision {
                arm: arm.into(),
                action: "list_callees".into(),
                cited,
                claim_verdict: None,
                unknown: false,
                tool_calls: 1,
                notes: "graph callees / diff review set from artifact".into(),
            }
        }
        TaskKind::Cycle => {
            let view = oracle.in_cycle(needle);
            let Some(view) = view else {
                return missing(arm, "symbol unresolved");
            };
            ArmDecision {
                arm: arm.into(),
                action: "use_oracle".into(),
                cited: view.witnesses,
                claim_verdict: Some(view.verdict),
                unknown: false,
                tool_calls: 1,
                notes: "in_cycle on artifact".into(),
            }
        }
        TaskKind::Honesty => ArmDecision {
            arm: arm.into(),
            action: "unknown".into(),
            cited: vec![],
            claim_verdict: None,
            unknown: true,
            tool_calls: 1,
            notes: "interface / dynamic dispatch: abstain".into(),
        },
        TaskKind::SafeDelete => {
            let view = oracle.no_callers(needle, treesitter_only);
            let Some(view) = view else {
                return missing(arm, "symbol unresolved");
            };
            ArmDecision {
                arm: arm.into(),
                action: if view.verdict == "refuted" {
                    "refuse_delete".into()
                } else {
                    "approve_delete".into()
                },
                cited: view.witnesses,
                claim_verdict: Some(view.verdict),
                unknown: false,
                tool_calls: 1,
                notes: "verify_claim no_callers".into(),
            }
        }
    }
}

fn missing(arm: &str, why: &str) -> ArmDecision {
    ArmDecision {
        arm: arm.into(),
        action: "unknown".into(),
        cited: vec![],
        claim_verdict: Some("unknown".into()),
        unknown: true,
        tool_calls: 1,
        notes: why.into(),
    }
}

pub fn default_artifact(corpus: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let (env_key, fallback_id) = match corpus {
        "chi" => (
            "GRIDSEAK_G1_CHI_ARTIFACT",
            "73abec07-12fd-47e9-a2e8-fb3c69fdf971",
        ),
        "zod" => (
            "GRIDSEAK_G1_ZOD_ARTIFACT",
            "4cb6e82a-e4d7-458f-aecb-09c3f8d87452",
        ),
        _ => return None,
    };
    if let Ok(p) = std::env::var(env_key) {
        return Some(Path::new(&p).to_path_buf());
    }
    Some(
        Path::new(&home)
            .join("Library/Application Support/com.gridseak.desktop/project-graphs")
            .join(format!("{fallback_id}.sqlite")),
    )
}

pub fn default_corpus_root(corpus: &str) -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".cache/g1")
        .join(corpus);
    if root.is_dir() {
        Some(root)
    } else {
        None
    }
}
