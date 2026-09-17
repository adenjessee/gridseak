//! `gridseak judge <subject> <verdict>` — record a human/agent edge judgment.

use anyhow::Result;
use clap::Args;
use gridseak_local_store::ProjectStore;

#[derive(Args, Debug)]
pub struct JudgeArgs {
    /// Edge or node identifier (FQN, edge id, or file:line).
    pub subject: String,
    /// `confirm` | `reject` | `unsure`.
    pub verdict: String,
    #[arg(long)]
    pub note: Option<String>,
    #[arg(long)]
    pub scan_id: Option<String>,
    #[arg(long, default_value = "human")]
    pub actor: String,
    #[arg(default_value = ".")]
    pub project: String,
}

pub fn run_judge(store: &ProjectStore, args: JudgeArgs) -> Result<()> {
    let project_id = store
        .resolve_project_lenient(&args.project)
        .ok()
        .map(|p| p.id);
    let id = store.record_judgment(
        project_id.as_deref(),
        &args.subject,
        &args.verdict,
        args.note.as_deref(),
        args.scan_id.as_deref(),
        &args.actor,
    )?;
    println!("judgment {id} recorded ({})", args.verdict);
    Ok(())
}
