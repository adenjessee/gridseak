//! Drivers for the history-shaped commands.
//!
//! `scans list`, `scan latest`, `compare`, and `trends` share the
//! same data sources (the local store's `scan_runs` and
//! `metric_snapshots` tables) and the same renderer choice (any of
//! table / markdown / json / for-llm). Keeping them in one module
//! keeps the four argument structs side-by-side so it stays obvious
//! that they form a coherent surface.
//!
//! Per-command driver functions live here; rendering lives in
//! `crate::render::{history,compare,trends}` so layout decisions are
//! one module away. The driver pattern is intentionally identical
//! across all four commands: resolve project → load store rows →
//! build view → dispatch to renderer.

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use graphengine_analysis::health::report::HealthReport;
use gridseak_local_store::{ProjectStore, ScanRunDto};

use crate::render::{
    compare::{CompareFormat, ScanCompareView},
    history::{HistoryFormat, ScanHistoryRow, ScanHistoryView},
    render_hero,
    trends::{TrendPoint, TrendsFormat, TrendsView},
    view::ScanReportView,
    width, HeroFormat,
};
use crate::scan_command::ScanOutputFormat;

// ---------------------------------------------------------------------------
// Shared format selector
// ---------------------------------------------------------------------------

/// Resolve a `--format` + `--for-llm` + global `--json` triplet into
/// a concrete output choice. Mirrors the precedence ladder in
/// `ScanArgs::hero_format`: `--for-llm` > `--json` > `--format`.
fn resolve_format(format: ScanOutputFormat, for_llm: bool, global_json: bool) -> ResolvedFormat {
    if for_llm {
        return ResolvedFormat::ForLlm;
    }
    if global_json {
        return ResolvedFormat::Json;
    }
    match format {
        ScanOutputFormat::Table => ResolvedFormat::Table,
        ScanOutputFormat::Markdown => ResolvedFormat::Markdown,
        ScanOutputFormat::Json => ResolvedFormat::Json,
    }
}

#[derive(Clone, Copy, Debug)]
enum ResolvedFormat {
    Table,
    Markdown,
    Json,
    ForLlm,
}

impl ResolvedFormat {
    fn history(self) -> HistoryFormat {
        match self {
            Self::Table => HistoryFormat::Table {
                layout: width::detect(),
            },
            Self::Markdown => HistoryFormat::Markdown,
            Self::Json => HistoryFormat::Json,
            Self::ForLlm => HistoryFormat::ForLlm,
        }
    }

    fn compare(self) -> CompareFormat {
        match self {
            Self::Table => CompareFormat::Table {
                layout: width::detect(),
            },
            Self::Markdown => CompareFormat::Markdown,
            Self::Json => CompareFormat::Json,
            Self::ForLlm => CompareFormat::ForLlm,
        }
    }

    fn trends(self) -> TrendsFormat {
        match self {
            Self::Table => TrendsFormat::Table {
                layout: width::detect(),
            },
            Self::Markdown => TrendsFormat::Markdown,
            Self::Json => TrendsFormat::Json,
            Self::ForLlm => TrendsFormat::ForLlm,
        }
    }

    fn hero(self, budget: Option<usize>) -> HeroFormat {
        match self {
            Self::Table => HeroFormat::Table {
                layout: width::detect(),
            },
            Self::Markdown => HeroFormat::Markdown,
            Self::Json => HeroFormat::Json,
            Self::ForLlm => HeroFormat::ForLlm { budget },
        }
    }
}

// ---------------------------------------------------------------------------
// `gridseak scans list`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct ScansListArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Cap on returned scans (most recent first). 0 means no cap.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    /// Filter by git branch.
    #[arg(long)]
    pub branch: Option<String>,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

pub fn run_scans_list(store: &ProjectStore, args: ScansListArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let mut scans = store.list_scan_runs(&project.id)?;
    if let Some(branch) = args.branch.as_deref() {
        scans.retain(|s| s.git_branch.as_deref() == Some(branch));
    }
    if args.limit > 0 {
        scans.truncate(args.limit);
    }
    let rows: Vec<ScanHistoryRow> = scans
        .iter()
        .map(|s| ScanHistoryRow::from_scan(s, s.metrics.as_ref()))
        .collect();
    let view = ScanHistoryView {
        repo_name: project.display_name.clone(),
        scans: rows,
    };
    let format = resolve_format(args.format, args.for_llm, global_json).history();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    crate::render::history::render(&format, &view, &mut handle)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `gridseak scan latest`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct ScanLatestArgs {
    #[arg(default_value = ".")]
    pub project: String,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,

    #[arg(long)]
    pub budget: Option<usize>,
}

pub fn run_scan_latest(
    store: &ProjectStore,
    args: ScanLatestArgs,
    global_json: bool,
) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scan = project
        .latest_scan
        .clone()
        .context("project has no scans — run `gridseak scan .` first")?;
    let report_value = store.load_report(&scan.id)?;
    let report: HealthReport = serde_json::from_value(report_value)?;
    let view = ScanReportView::build(&report, &project, &scan);
    let format = resolve_format(args.format, args.for_llm, global_json).hero(args.budget);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    render_hero(&format, &view, &mut handle)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `gridseak compare`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct CompareArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Older scan id. Defaults to the second-most-recent scan.
    #[arg(long)]
    pub from: Option<String>,

    /// Newer scan id. Defaults to the most recent scan.
    #[arg(long)]
    pub to: Option<String>,

    /// Shorthand for comparing the two most recent scans. Equivalent
    /// to omitting both `--from` and `--to`; kept as a noun-shaped
    /// flag because the spec example used it.
    #[arg(long, default_value_t = false)]
    pub previous: bool,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

pub fn run_compare(store: &ProjectStore, args: CompareArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let scans = store.list_scan_runs(&project.id)?;
    if scans.len() < 2 && (args.from.is_none() || args.to.is_none()) {
        anyhow::bail!(
            "need at least two scans on `{}` to compare (have {}). \
             Run `gridseak scan .` again or pass --from/--to explicitly.",
            project.display_name,
            scans.len()
        );
    }
    let (from_scan, to_scan) =
        pick_compare_scans(&scans, args.from.as_deref(), args.to.as_deref())?;
    let from_row = ScanHistoryRow::from_scan(from_scan, from_scan.metrics.as_ref());
    let to_row = ScanHistoryRow::from_scan(to_scan, to_scan.metrics.as_ref());
    let view = ScanCompareView::build(project.display_name.clone(), from_row, to_row);
    let format = resolve_format(args.format, args.for_llm, global_json).compare();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    crate::render::compare::render(&format, &view, &mut handle)?;
    Ok(())
}

fn pick_compare_scans<'a>(
    scans: &'a [ScanRunDto],
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(&'a ScanRunDto, &'a ScanRunDto)> {
    // `scans` arrives newest-first from list_scan_runs.
    let find = |id: &str| -> Result<&ScanRunDto> {
        scans
            .iter()
            .find(|s| s.id == id)
            .with_context(|| format!("scan id `{id}` not found in this project's history"))
    };
    let to_scan = match to {
        Some(id) => find(id)?,
        None => scans.first().context("project has no scans")?,
    };
    let from_scan = match from {
        Some(id) => find(id)?,
        None => scans
            .iter()
            .find(|s| s.id != to_scan.id)
            .context("could not find a second scan to compare against")?,
    };
    Ok((from_scan, to_scan))
}

// ---------------------------------------------------------------------------
// `gridseak trends`
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Clone)]
pub struct TrendsArgs {
    #[arg(default_value = ".")]
    pub project: String,

    /// Metric series to plot.
    #[arg(long, value_enum, default_value_t = TrendsMetric::Score)]
    pub metric: TrendsMetric,

    /// Time window. Supports `30d`, `12w`, `6m`. Default `90d`.
    #[arg(long, default_value = "90d")]
    pub window: String,

    /// Max points to render after window filter.
    #[arg(long, default_value_t = 30)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = ScanOutputFormat::Table)]
    pub format: ScanOutputFormat,

    #[arg(long, default_value_t = false)]
    pub for_llm: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum TrendsMetric {
    Score,
    Critical,
    High,
    Cycles,
    Hotspots,
    DeadCode,
    Coupling,
}

impl TrendsMetric {
    fn label(&self) -> &'static str {
        match self {
            Self::Score => "score",
            Self::Critical => "critical findings",
            Self::High => "high findings",
            Self::Cycles => "cycles",
            Self::Hotspots => "hotspots",
            Self::DeadCode => "dead code",
            Self::Coupling => "avg coupling",
        }
    }

    fn unit(&self) -> &'static str {
        match self {
            Self::Score => "/100",
            _ => "",
        }
    }

    fn value(&self, scan: &ScanRunDto) -> Option<f64> {
        let m = scan.metrics.as_ref()?;
        Some(match self {
            Self::Score => m.health_score?,
            Self::Critical => m.critical_count as f64,
            Self::High => m.high_count as f64,
            Self::Cycles => m.cycle_count as f64,
            Self::Hotspots => m.hotspot_count as f64,
            Self::DeadCode => m.dead_code_count as f64,
            Self::Coupling => m.avg_coupling?,
        })
    }
}

pub fn run_trends(store: &ProjectStore, args: TrendsArgs, global_json: bool) -> Result<()> {
    let project = store.resolve_project(&args.project)?;
    let mut scans = store.list_scan_runs(&project.id)?;
    let cutoff = parse_window(&args.window)?;
    scans.retain(|s| {
        chrono::DateTime::parse_from_rfc3339(&s.started_at)
            .map(|dt| dt.with_timezone(&chrono::Utc) >= cutoff)
            .unwrap_or(true)
    });
    // Re-sort ascending so the sparkline reads left-to-right as time
    // moves forward, which is what every chart in the universe does.
    scans.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    if args.limit > 0 && scans.len() > args.limit {
        scans = scans.split_off(scans.len() - args.limit);
    }

    let series: Vec<TrendPoint> = scans
        .iter()
        .filter_map(|s| {
            args.metric.value(s).map(|v| TrendPoint {
                scan_id: s.id.clone(),
                date: shorten_date(&s.started_at),
                value: v,
            })
        })
        .collect();

    let view = TrendsView {
        repo_name: project.display_name.clone(),
        metric: args.metric.label().to_string(),
        unit: args.metric.unit().to_string(),
        series,
    };
    let format = resolve_format(args.format, args.for_llm, global_json).trends();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    crate::render::trends::render(&format, &view, &mut handle)?;
    Ok(())
}

/// Parse a window expression like `30d`, `12w`, `6m`, `1y`. Returns
/// the absolute cutoff timestamp; scans with `started_at >= cutoff`
/// pass the filter.
///
/// Accepts a bare integer (assumed to be days) for legacy CI scripts
/// that didn't read the help text.
fn parse_window(expr: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    let expr = expr.trim().to_ascii_lowercase();
    if expr.is_empty() {
        anyhow::bail!("empty --window expression");
    }
    let (n, unit) = if let Some(last) = expr.chars().last() {
        if last.is_ascii_digit() {
            (expr.parse::<i64>().context("invalid --window value")?, 'd')
        } else {
            let n: i64 = expr[..expr.len() - 1]
                .parse()
                .context("invalid --window number")?;
            (n, last)
        }
    } else {
        anyhow::bail!("invalid --window expression");
    };
    let dur = match unit {
        'd' => chrono::Duration::days(n),
        'w' => chrono::Duration::weeks(n),
        'm' => chrono::Duration::days(n * 30),
        'y' => chrono::Duration::days(n * 365),
        other => anyhow::bail!("unknown --window unit `{other}` (use d, w, m, or y)"),
    };
    Ok(chrono::Utc::now() - dur)
}

fn shorten_date(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.split('T').next().unwrap_or(rfc3339).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_window_supports_units() {
        let now = chrono::Utc::now();
        let d = parse_window("30d").unwrap();
        assert!((now - d).num_days() >= 29);
        let w = parse_window("2w").unwrap();
        assert!((now - w).num_days() >= 13);
        let m = parse_window("3m").unwrap();
        assert!((now - m).num_days() >= 89);
    }

    #[test]
    fn parse_window_rejects_garbage() {
        assert!(parse_window("garbage").is_err());
        assert!(parse_window("").is_err());
        assert!(parse_window("3z").is_err());
    }

    #[test]
    fn parse_window_treats_bare_int_as_days() {
        let d = parse_window("7").unwrap();
        let now = chrono::Utc::now();
        assert!((now - d).num_days() >= 6);
    }
}
