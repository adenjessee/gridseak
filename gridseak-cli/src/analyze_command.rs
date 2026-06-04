//! `gridseak analyze --background` — debounced background segment precompute.

use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use gridseak_local_store::{LocalStorePaths, ProjectStore};

#[derive(Args, Debug, Clone)]
pub struct AnalyzeArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Debounce interval in seconds before re-running analysis after changes.
    #[arg(long, default_value_t = 2)]
    pub debounce: u64,

    /// Run one analysis pass and exit (no watch loop).
    #[arg(long, default_value_t = false)]
    pub once: bool,
}

pub fn run_analyze_background(
    store: &ProjectStore,
    _paths: &LocalStorePaths,
    args: AnalyzeArgs,
) -> Result<()> {
    let project = store.resolve_project_lenient(&args.project)?;
    let scan = project
        .latest_scan
        .as_ref()
        .context("project has no scans — run `gridseak scan .` first")?;
    let graph_path = scan
        .graph_artifact_path
        .as_ref()
        .context("latest scan has no graph artifact")?;
    let root = project
        .roots
        .first()
        .map(|r| r.path.as_str())
        .unwrap_or(".");

    eprintln!(
        "[gridseak] analyze: background worker on {} (debounce {}s)",
        project.display_name, args.debounce
    );

    let mut last_mtime: Option<std::time::SystemTime> = None;
    loop {
        let mtime = latest_tree_mtime(Path::new(root))?;
        let changed = match (&last_mtime, mtime) {
            (Some(prev), Some(cur)) => cur > *prev,
            (None, Some(_)) => true,
            _ => false,
        };
        if changed {
            thread::sleep(Duration::from_secs(args.debounce));
            run_one_analysis(graph_path)?;
            last_mtime = mtime;
            eprintln!("[gridseak] analyze: segment cache refreshed");
        } else {
            thread::sleep(Duration::from_secs(1));
        }
        if args.once {
            break;
        }
    }
    Ok(())
}

fn run_one_analysis(graph_path: &str) -> Result<()> {
    graphengine_analysis::health::pipeline::run_analysis_pipeline(
        graph_path, None, None, None, None, true,
    )?;
    Ok(())
}

fn latest_tree_mtime(root: &Path) -> Result<Option<std::time::SystemTime>> {
    let mut latest = None;
    if !root.exists() {
        return Ok(None);
    }
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                latest = Some(latest.map_or(modified, |l: std::time::SystemTime| l.max(modified)));
            }
        }
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridseak_local_store::LocalStorePaths;

    #[test]
    fn background_analyze_requires_prior_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = LocalStorePaths::from_dirs(dir.path().join("data"), dir.path().join("cache"))
            .expect("store paths");
        let store = paths.open_store().expect("open store");
        let err = run_analyze_background(
            &store,
            &paths,
            AnalyzeArgs {
                project: ".".into(),
                debounce: 1,
                once: true,
            },
        )
        .expect_err("must fail without scans");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no scans"),
            "expected no-scans error, got: {msg}"
        );
    }
}
