//! Smoke-test the diagnostic pipeline against an actual report file.
//!
//! Run with:
//!     GRIDSEAK_SMOKE_REPORT=/tmp/gridseak-smoke/report.json \
//!         cargo test -p graphengine-diagnostic --test real_report_smoke -- --nocapture
//!
//! Skipped automatically when the env var is unset, so this file is safe to
//! keep in normal CI (it just prints and passes).

use graphengine_analysis::health::report::HealthReport;
use graphengine_diagnostic::priority;

#[test]
fn end_to_end_on_real_report() {
    let Ok(path) = std::env::var("GRIDSEAK_SMOKE_REPORT") else {
        eprintln!("GRIDSEAK_SMOKE_REPORT unset — skipping real-report smoke");
        return;
    };
    let data = std::fs::read_to_string(&path).expect("read report");
    let report: HealthReport = serde_json::from_str(&data).expect("deserialize report");
    let priorities = priority::compute_priorities(&report, priority::DEFAULT_TOP_N);
    println!("priorities returned: {}", priorities.len());
    for p in priorities.iter().take(5) {
        println!(
            "#{:>2}  score={:.2}  type={:?}  target={}",
            p.rank, p.priority_score, p.finding_type, p.target
        );
        println!("      narrative: {}", p.risk_narrative);
        println!("      action:    {}", p.suggested_action);
    }
}
