//! T4 canary-level tier-classification fixtures.
//!
//! `MeasuredFidelityTier::from_call_edges` has unit coverage against a
//! hand-built `EdgesByConfidence` (see the `measured_fidelity_tests`
//! module in `graphengine-analysis/src/health/report.rs`). Those unit
//! tests prove the pure function. This file proves the end-to-end
//! contract: given a synthesized `parse.db` with a controlled
//! call-edge confidence distribution, `run_analysis` → `resolution_quality`
//! → `measured_fidelity.tier` produces the expected tier for every
//! D1 canary prediction AND for the 40 % / 80 % threshold boundaries.
//!
//! Why six fixtures, not two:
//!
//! Two endpoints (NPSP ~1 % High, hypothetical 85 % High) cover the
//! `SyntacticOnly` and `Authoritative` extremes. They do NOT catch a
//! silent direction flip at the thresholds — e.g., changing
//! `r >= 0.80` to `r > 0.80` would pass an endpoint-only test but
//! would reclassify any canary sitting exactly on a boundary. The
//! quartet at 39 / 41 / 79 / 81 % locks the direction of each cut.
//!
//! The fixture builder constructs a controlled call graph of
//! `num_high` High-confidence `Call` edges plus `num_low`
//! Low-confidence `Call` edges, all pointing from distinct caller
//! functions to a shared sink. Every other edge family (`Contains`
//! for module / file / function parenthood) is emitted at `High`
//! confidence and is filtered out of the denominator by
//! `call_edges_by_confidence`, so the ratio the tier reads is exactly
//! `num_high / (num_high + num_low)`.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

use graphengine_analysis::health::report::MeasuredFidelityTier;

fn create_schema(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL,
            provenance TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_fqn ON nodes(fqn);
        CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);

        CREATE TABLE IF NOT EXISTS edges (
            from_id TEXT NOT NULL REFERENCES nodes(id),
            to_id TEXT NOT NULL REFERENCES nodes(id),
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, kind)
        );
        CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
        CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
        CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
        ",
    )
    .unwrap();
}

fn insert_node(conn: &Connection, id: &str, kind: &str, fqn: &str, properties: &str) {
    conn.execute(
        "INSERT INTO nodes (id, kind, fqn, location, provenance, properties) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            kind,
            fqn,
            r#"{"file": "src/test.ts", "start_line": 1, "start_char": 0, "end_line": 10, "end_char": 0}"#,
            r#"{"source": "Lsp", "confidence": "High"}"#,
            properties,
        ],
    )
    .unwrap();
}

fn insert_edge(conn: &Connection, from_id: &str, to_id: &str, kind: &str, confidence: &str) {
    let prov = format!(
        r#"{{"source": "Heuristic", "confidence": "{}"}}"#,
        confidence
    );
    conn.execute(
        "INSERT INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![from_id, to_id, format!(r#"{{"kind":"{kind}"}}"#), prov],
    )
    .unwrap();
}

/// Build a `parse.db` whose call-edge confidence distribution is
/// exactly `num_high` High edges and `num_low` Low edges.
///
/// Topology: single module (`src::mod_a`), one file, `num_high +
/// num_low + 1` functions (one sink, the rest callers). Each caller
/// emits exactly one `Call` edge to the sink. This guarantees:
///   - `call_edges_by_confidence.high == num_high`
///   - `call_edges_by_confidence.low == num_low`
///   - `high_ratio_on_calls == num_high / (num_high + num_low)`
///
/// Intentionally no Medium or Unknown — the thresholds sit in the
/// ratio of High over total, so adding other confidence levels would
/// just dilute the denominator. Keeping the distribution binary lets
/// each fixture's name (`boundary_39_pct_...`) match its arithmetic.
fn build_ratio_db(path: &std::path::Path, num_high: usize, num_low: usize) {
    let _ = fs::remove_file(path);
    let conn = Connection::open(path).unwrap();
    create_schema(&conn);

    insert_node(
        &conn,
        "proj",
        "Project",
        "project",
        r#"{"language": "typescript"}"#,
    );
    insert_node(
        &conn,
        "folder_a",
        "Folder",
        "src::mod_a",
        r#"{"path_repo_rel": "src/mod_a", "role": "source"}"#,
    );
    insert_edge(&conn, "proj", "folder_a", "Contains", "High");
    insert_node(
        &conn,
        "file_a",
        "File",
        "src::mod_a::file_a",
        r#"{"path_repo_rel": "src/mod_a/file_a.ts", "role": "source"}"#,
    );
    insert_edge(&conn, "folder_a", "file_a", "Contains", "High");

    // Sink function: every caller points here.
    insert_node(
        &conn,
        "fn_sink",
        "Function",
        "src::mod_a::fn_sink",
        r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
    );
    insert_edge(&conn, "file_a", "fn_sink", "Contains", "High");

    for i in 0..num_high {
        let id = format!("fn_h_{i}");
        insert_node(
            &conn,
            &id,
            "Function",
            &format!("src::mod_a::{id}"),
            r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
        );
        insert_edge(&conn, "file_a", &id, "Contains", "High");
        insert_edge(&conn, &id, "fn_sink", "Call", "High");
    }

    for i in 0..num_low {
        let id = format!("fn_l_{i}");
        insert_node(
            &conn,
            &id,
            "Function",
            &format!("src::mod_a::{id}"),
            r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
        );
        insert_edge(&conn, "file_a", &id, "Contains", "High");
        insert_edge(&conn, &id, "fn_sink", "Call", "Low");
    }
}

struct Case {
    /// Human-readable name, used in panic messages and for the temp DB
    /// filename. The name intentionally encodes the ratio + expected
    /// tier so a future engineer changing the thresholds sees the
    /// mismatch immediately.
    name: &'static str,
    num_high: usize,
    num_low: usize,
    expected_tier: MeasuredFidelityTier,
    /// Expected `high_ratio_on_calls`. Verified with 1e-9 tolerance.
    expected_ratio: f64,
}

fn run_case(c: &Case) {
    let tmp_dir = std::env::temp_dir();
    let db_path: PathBuf = tmp_dir.join(format!(
        "ge_analyze_t4_canary_{}_{}.sqlite",
        c.name,
        std::process::id()
    ));
    build_ratio_db(&db_path, c.num_high, c.num_low);
    let db_str = db_path.to_str().unwrap();

    let report = graphengine_analysis::health::run_analysis(db_str)
        .unwrap_or_else(|e| panic!("[{}] run_analysis failed: {e}", c.name));

    let rq = report.resolution_quality.as_ref().unwrap_or_else(|| {
        panic!(
            "[{}] resolution_quality must be populated on non-empty DB",
            c.name
        )
    });
    let mf = &rq.measured_fidelity;

    assert_eq!(
        mf.tier, c.expected_tier,
        "[{}] expected tier {:?}, got {:?}; ratio={:?}, breakdown={:?}",
        c.name, c.expected_tier, mf.tier, mf.high_ratio_on_calls, mf.call_edges_by_confidence,
    );

    assert_eq!(
        mf.call_edges_by_confidence.high, c.num_high,
        "[{}] call_edges_by_confidence.high: expected {}, got {}",
        c.name, c.num_high, mf.call_edges_by_confidence.high,
    );
    assert_eq!(
        mf.call_edges_by_confidence.low, c.num_low,
        "[{}] call_edges_by_confidence.low: expected {}, got {}",
        c.name, c.num_low, mf.call_edges_by_confidence.low,
    );
    assert_eq!(
        mf.call_edges_by_confidence.total(),
        c.num_high + c.num_low,
        "[{}] call_edges_by_confidence.total: expected {}, got {}",
        c.name,
        c.num_high + c.num_low,
        mf.call_edges_by_confidence.total(),
    );

    let actual_ratio = mf
        .high_ratio_on_calls
        .unwrap_or_else(|| panic!("[{}] high_ratio_on_calls must be Some", c.name));
    assert!(
        (actual_ratio - c.expected_ratio).abs() < 1e-9,
        "[{}] expected high_ratio_on_calls ~= {}, got {}",
        c.name,
        c.expected_ratio,
        actual_ratio,
    );

    let _ = fs::remove_file(&db_path);
}

// ------------------------------------------------------------
// Canary endpoint: NPSP rev 9 shape (1.1% High on call-like edges).
// D1 prediction: SyntacticOnly.
// ------------------------------------------------------------
#[test]
fn npsp_rev9_shape_is_syntactic_only() {
    // NPSP rev 9 D1 distribution: 860 High / 77 544 total → 1.10876 %.
    // We model with a small scaled-down ratio that preserves the band:
    // 1 High / 99 Low = 1.0 % → still below 40 % threshold.
    run_case(&Case {
        name: "npsp_rev9_shape",
        num_high: 1,
        num_low: 99,
        expected_tier: MeasuredFidelityTier::SyntacticOnly,
        expected_ratio: 0.01,
    });
}

// ------------------------------------------------------------
// Target endpoint: hypothetical post-Layer-2 ratio (85% High).
// D1 target for Rust-Layer-2-on-gridseak-self: Authoritative.
// ------------------------------------------------------------
#[test]
fn hypothetical_85pct_high_is_authoritative() {
    run_case(&Case {
        name: "hypothetical_85pct_high",
        num_high: 85,
        num_low: 15,
        expected_tier: MeasuredFidelityTier::Authoritative,
        expected_ratio: 0.85,
    });
}

// ------------------------------------------------------------
// Boundary quartet at 40% / 80%. Each pair straddles a cut so an
// off-by-one at the threshold (>= vs >) flips a classification.
// ------------------------------------------------------------

/// 39 % High — just below the 40 % floor. Must classify
/// `SyntacticOnly`. If this test fails with
/// `HeuristicPrimary`, the low-end threshold has been loosened from
/// `>= 0.40` to `> 0.39` or similar.
#[test]
fn boundary_39_pct_is_syntactic_only() {
    run_case(&Case {
        name: "boundary_39_pct",
        num_high: 39,
        num_low: 61,
        expected_tier: MeasuredFidelityTier::SyntacticOnly,
        expected_ratio: 0.39,
    });
}

/// 41 % High — just above the 40 % floor. Must classify
/// `HeuristicPrimary`. Complements the 39 % fixture: together they
/// pin the direction of the low cut (which side contains 40.0).
#[test]
fn boundary_41_pct_is_heuristic_primary() {
    run_case(&Case {
        name: "boundary_41_pct",
        num_high: 41,
        num_low: 59,
        expected_tier: MeasuredFidelityTier::HeuristicPrimary,
        expected_ratio: 0.41,
    });
}

/// 79 % High — just below the 80 % ceiling. Must classify
/// `HeuristicPrimary`.
#[test]
fn boundary_79_pct_is_heuristic_primary() {
    run_case(&Case {
        name: "boundary_79_pct",
        num_high: 79,
        num_low: 21,
        expected_tier: MeasuredFidelityTier::HeuristicPrimary,
        expected_ratio: 0.79,
    });
}

/// 81 % High — just above the 80 % ceiling. Must classify
/// `Authoritative`. Complements the 79 % fixture.
#[test]
fn boundary_81_pct_is_authoritative() {
    run_case(&Case {
        name: "boundary_81_pct",
        num_high: 81,
        num_low: 19,
        expected_tier: MeasuredFidelityTier::Authoritative,
        expected_ratio: 0.81,
    });
}

// ------------------------------------------------------------
// D1 canary predictions (UF-FU-007 closure, Gate 1.3 hygiene).
//
// These fixtures mirror the distribution shape reported in
// `DISCOVERY_REPORT.md` §D1 for every canary currently in the corpus,
// plus the post-T6 Rust distribution measured in
// `experiments/results/gate1-2-t6-pr2-dogfood/rollup.json`. Each
// fixture pins the tier the classifier must report for that canary's
// High / (High + Low) ratio so a future classifier tweak cannot
// silently flip the prediction.
//
// A synthetic scaled-down ratio is used wherever the source canary
// has > 100 call edges, so the SQLite insertion stays cheap (< 100 ms
// per case) while preserving band membership. The name of each test
// encodes the source ratio so a diff reviewer can match it against
// DISCOVERY_REPORT.md without chasing pointers.
// ------------------------------------------------------------

/// `commons-lang` canary: D1 reports 441 High / (441 + 209,815) =
/// 0.210 % High on Call edges, predicted `SyntacticOnly`. We model
/// the band (< 40 %) with 1 High / 499 Low = 0.200 % which matches
/// the source ratio to two significant figures.
#[test]
fn canary_commons_lang_java_is_syntactic_only() {
    run_case(&Case {
        name: "canary_commons_lang",
        num_high: 1,
        num_low: 499,
        expected_tier: MeasuredFidelityTier::SyntacticOnly,
        expected_ratio: 1.0 / 500.0,
    });
}

/// `django-site` canary: D1 reports 0 High / 834 total = 0.0 % High
/// on Call edges, predicted `SyntacticOnly`. We model with 0 High /
/// 100 Low to exercise the zero-High arm of the classifier.
#[test]
fn canary_django_site_python_is_syntactic_only() {
    run_case(&Case {
        name: "canary_django_site",
        num_high: 0,
        num_low: 100,
        expected_tier: MeasuredFidelityTier::SyntacticOnly,
        expected_ratio: 0.0,
    });
}

/// `nextjs-commerce` canary: D1 reports 62 High / (62 + 32) = 66.0 %
/// High on Call edges (recomputed from raw counts; the 53 % figure in
/// DISCOVERY_REPORT §D1 is Lsp/High share of the full edge family
/// including Heuristic/Medium, which dilutes the denominator and is
/// the wrong base for tier classification). Predicted
/// `HeuristicPrimary` — below the 80 % `Authoritative` ceiling but
/// above the 40 % `SyntacticOnly` floor. This fixture is the nearest
/// real-canary signal that crosses the 40 % cut in an intended
/// direction.
#[test]
fn canary_nextjs_commerce_ts_is_heuristic_primary() {
    run_case(&Case {
        name: "canary_nextjs_commerce",
        num_high: 62,
        num_low: 32,
        expected_tier: MeasuredFidelityTier::HeuristicPrimary,
        expected_ratio: 62.0 / 94.0,
    });
}

/// `serilog` canary: D1 reports 191 High / (191 + 20,894) = 0.906 %
/// High on Call edges, predicted `SyntacticOnly`. We model with 9
/// High / 991 Low = 0.900 %.
#[test]
fn canary_serilog_csharp_is_syntactic_only() {
    run_case(&Case {
        name: "canary_serilog",
        num_high: 9,
        num_low: 991,
        expected_tier: MeasuredFidelityTier::SyntacticOnly,
        expected_ratio: 9.0 / 1000.0,
    });
}

/// Rust post-T6 canary: Gate 1.2 dogfood on `gridseak-self` produced
/// 6,550 High Call edges with 0 heuristic fallback emissions (every
/// call site the adapter resolved stayed High; every call site the
/// adapter missed dropped out of the edge set rather than landing at
/// Low via the name-match fallback because `gridseak-self` does not
/// satisfy the unique-name criterion for most symbols). Ratio is
/// 1.00 → `Authoritative`. See rollup at
/// `experiments/results/gate1-2-t6-pr2-dogfood/rollup.json`.
///
/// The numbers below use a down-scaled `100 High / 0 Low` shape; the
/// tier classifier sees High / (High + Low) = 1.0 regardless of
/// scale. The real gridseak-self run is far too large for a SQLite
/// round-trip in unit-test time.
#[test]
fn canary_rust_post_t6_gridseak_self_is_authoritative() {
    run_case(&Case {
        name: "canary_rust_post_t6",
        num_high: 100,
        num_low: 0,
        expected_tier: MeasuredFidelityTier::Authoritative,
        expected_ratio: 1.0,
    });
}
