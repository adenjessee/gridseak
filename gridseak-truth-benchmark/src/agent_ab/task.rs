use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    SafeDelete,
    WrongTwin,
    Blast,
    FalseClaim,
    DiffImpact,
    Cycle,
    Honesty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub language: String,
    pub kind: TaskKind,
    pub corpus: String,
    pub prompt: String,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub symbol_name: Option<String>,
    #[serde(default)]
    pub correct_file: Option<String>,
    #[serde(default)]
    pub wrong_twin_files: Vec<String>,
    #[serde(default)]
    pub gold_action: Option<String>,
    #[serde(default)]
    pub gold_claim: Option<String>,
    #[serde(default)]
    pub gold_verdict: Option<String>,
    #[serde(default)]
    pub gold_blast_contains: Vec<String>,
    #[serde(default)]
    pub changed_files: Vec<String>,
    pub oracle: String,
    pub oracle_notes: String,
}

pub fn load_tasks(path: &Path) -> anyhow::Result<Vec<Task>> {
    let raw = std::fs::read_to_string(path)?;
    let tasks: Vec<Task> = serde_json::from_str(&raw)?;
    anyhow::ensure!(tasks.len() == 20, "expected 20 tasks, got {}", tasks.len());
    let ts = tasks.iter().filter(|t| t.language == "typescript").count();
    let go = tasks.iter().filter(|t| t.language == "go").count();
    anyhow::ensure!(
        ts == 10 && go == 10,
        "expected 10 ts + 10 go, got {ts}+{go}"
    );
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_twenty() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/agent-ab/tasks.json");
        let tasks = load_tasks(&p).expect("catalog");
        assert_eq!(tasks.len(), 20);
        for t in &tasks {
            let dir = if t.language == "go" { "go" } else { "ts" };
            let md = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/agent-ab/tasks")
                .join(dir)
                .join(format!("{}.md", t.id));
            assert!(md.is_file(), "missing {}", md.display());
            assert!(!t.oracle_notes.is_empty(), "{} missing provenance", t.id);
        }
    }
}
