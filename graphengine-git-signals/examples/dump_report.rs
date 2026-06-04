//! One-shot measurement binary for T7 §9 correlation exercises.
//!
//! Opens the repository rooted at `$1` (or `.` when omitted), runs
//! the extractor with the default CI window, and prints a
//! pretty-JSON `GitSignalReport`. The point is not to ship this as
//! product — it is a purpose-built tool for recording
//! post-implementation correlation data (top-N hotspots, file
//! counts, confidence distribution) into T7 design doc §9.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p graphengine-git-signals --example dump_report -- .
//! ```

use std::path::PathBuf;

use graphengine_git_signals::{GitSignalExtractor, HistoryWindow};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let root = PathBuf::from(arg);

    let extractor = match GitSignalExtractor::open(&root) {
        Ok(x) => x,
        Err(err) => {
            eprintln!("open failed for {:?}: {err}", root);
            std::process::exit(2);
        }
    };
    eprintln!("repo_shape = {:?}", extractor.repo_shape());

    let start = std::time::Instant::now();
    let window = HistoryWindow::default_ci();
    let report = match extractor.extract(&window) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("extract failed: {err}");
            std::process::exit(3);
        }
    };
    let elapsed = start.elapsed();

    eprintln!(
        "commits_walked = {} files_touched = {} wall = {:?}",
        report.commits_walked, report.files_touched, elapsed
    );

    // Top-10 hotspot-score files for quick eyeballing.
    let mut ranked: Vec<_> = report
        .per_file
        .iter()
        .map(|(p, s)| (p.clone(), s.clone()))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.hotspot_score
            .partial_cmp(&a.1.hotspot_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    eprintln!("top-10 by hotspot_score:");
    for (path, signals) in ranked.iter().take(10) {
        eprintln!(
            "  {:>7.2}  changes={:>4}  authors={:>2}  last={:>3?}  conf={:?}  {}",
            signals.hotspot_score,
            signals.change_frequency,
            signals.distinct_authors,
            signals.last_touched_days,
            signals.confidence,
            path.display()
        );
    }

    if std::env::var("DUMP_REPORT_SKIP_JSON").is_ok() {
        // Memory-measurement mode: avoid the full-report
        // pretty-print because it dominates peak RSS for large
        // repos and is not representative of production use via
        // `attach_git_signals`.
        eprintln!("skipped JSON serialization (DUMP_REPORT_SKIP_JSON set)");
        return;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize report")
    );
}
