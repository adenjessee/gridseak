use gridseak_truth_benchmark::agent_ab::scorer::{render_share_comment, write_clip_script};
use gridseak_truth_benchmark::agent_ab::{load_tasks, run_harness};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let catalog = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/agent-ab/tasks.json");
    let tasks = load_tasks(&catalog)?;
    let model =
        std::env::var("GRIDSEAK_AGENT_AB_MODEL").unwrap_or_else(|_| "deterministic-v1".into());
    let report = run_harness(&tasks, &model)?;
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::create_dir_all("target")?;
    std::fs::write("target/agent-ab.json", &json)?;
    std::fs::write(
        "target/agent-ab-pr-comment.md",
        render_share_comment(&report),
    )?;
    write_clip_script(
        &report,
        std::path::Path::new("target/agent-ab-clip-script.md"),
    )?;
    println!("{json}");
    if !report.shine.treatment_beats_false_delete || !report.shine.treatment_beats_wrong_twin {
        eprintln!(
            "SHINE FAIL: treatment did not beat control on false-delete + wrong-twin; do not film"
        );
        std::process::exit(1);
    }
    Ok(())
}
