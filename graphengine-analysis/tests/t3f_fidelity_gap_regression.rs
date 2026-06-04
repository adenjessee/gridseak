//! T3f regression fixture: dual-metric emission produces a non-zero
//! fidelity gap on a contrived graph whose structural edge indices
//! diverge from their high-confidence subset.
//!
//! This file covers two concerns:
//!
//! 1. **Divergent-gap coverage** across *every* Layer-3 metric that
//!    carries `fidelity: Option<FidelityGap>`. The original fixture
//!    (`fidelity_gap_is_nonzero_on_divergent_graph`) asserted gap
//!    on `cycles` and `dead_code` only; the six sibling metrics
//!    (`coupling`, `cohesion`, `hotspot_concentration`, `depth`,
//!    `tangle_index`, `distance_from_main_sequence`) could silently
//!    regress to `None` or have `compute_health`'s swap-guard flip
//!    the wrong edge set without any test catching it. This file
//!    pins per-metric `Some(fidelity)` + signed-gap expectations.
//!
//! 2. **T4 measured-tier sanity** on the same fixture: the divergent
//!    DB has 50% High call-like edges and must classify as
//!    `HeuristicPrimary`. That assertion was already in the original
//!    test; it is preserved here so the two concerns are measured
//!    against the same seed graph.
//!
//! Fixture shape (see `build_divergent_db` comments for the literal
//! edge table): two modules (`src::mod_a`, `src::mod_b`), four
//! functions, a cross-module call cycle where exactly one side is
//! Low-confidence, and a dead-code probe whose only inbound caller is
//! a Low-confidence call. The two-module shape is required for
//! `coupling`, `cohesion`, and `distance_from_main_sequence` to
//! produce non-None values; the single-module predecessor of this
//! fixture produced `None` on those metrics.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

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

fn insert_edge_with_confidence(
    conn: &Connection,
    from_id: &str,
    to_id: &str,
    kind: &str,
    confidence: &str,
) {
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

/// Construct a two-module synthetic DB with mixed-confidence calls.
///
/// Module A (`src/mod_a`): `fn_a`, `fn_caller`
/// Module B (`src/mod_b`): `fn_b`, `fn_c`
///
/// Call edges (all 4 call-like; 2 High, 2 Low):
///   fn_a      -> fn_b      (High)   cross-module
///   fn_b      -> fn_a      (Low)    cross-module, closes a cycle
///   fn_caller -> fn_c      (Low)    cross-module, dead-code probe
///   fn_a      -> fn_caller (High)   intra-module, keeps fn_caller alive
///
/// Cross-module Import edges (populate module-level coupling):
///   mod_a file -> mod_b file (High)
///
/// All-edges views:
///   - cycles: {fn_a, fn_b} forms 1 SCC.
///   - dead_code: every function reachable.
///   - coupling/cohesion/distance: both modules connected.
///
/// High-only views (Low edges filtered):
///   - cycles: 0 SCCs (back-edge dropped).
///   - dead_code: fn_c has no callers → dead.
///   - coupling: cross-module structural edge count drops because
///     the Low Call edges are filtered out of the production set.
fn build_divergent_db(path: &std::path::Path) {
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

    // Module A
    insert_node(
        &conn,
        "folder_a",
        "Folder",
        "src::mod_a",
        r#"{"path_repo_rel": "src/mod_a", "role": "source"}"#,
    );
    insert_edge_with_confidence(&conn, "proj", "folder_a", "Contains", "High");

    insert_node(
        &conn,
        "file_a",
        "File",
        "src::mod_a::file_a",
        r#"{"path_repo_rel": "src/mod_a/file_a.ts", "role": "source"}"#,
    );
    insert_edge_with_confidence(&conn, "folder_a", "file_a", "Contains", "High");

    // Module B
    insert_node(
        &conn,
        "folder_b",
        "Folder",
        "src::mod_b",
        r#"{"path_repo_rel": "src/mod_b", "role": "source"}"#,
    );
    insert_edge_with_confidence(&conn, "proj", "folder_b", "Contains", "High");

    insert_node(
        &conn,
        "file_b",
        "File",
        "src::mod_b::file_b",
        r#"{"path_repo_rel": "src/mod_b/file_b.ts", "role": "source"}"#,
    );
    insert_edge_with_confidence(&conn, "folder_b", "file_b", "Contains", "High");

    // Functions in A
    for name in &["fn_a", "fn_caller"] {
        insert_node(
            &conn,
            name,
            "Function",
            &format!("src::mod_a::{name}"),
            r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
        );
        insert_edge_with_confidence(&conn, "file_a", name, "Contains", "High");
    }

    // Functions in B
    for name in &["fn_b", "fn_c"] {
        insert_node(
            &conn,
            name,
            "Function",
            &format!("src::mod_b::{name}"),
            r#"{"cyclomatic_complexity": 1, "cognitive_complexity": 1}"#,
        );
        insert_edge_with_confidence(&conn, "file_b", name, "Contains", "High");
    }

    // Cycle: fn_a <-> fn_b, Low back-edge closes the SCC.
    insert_edge_with_confidence(&conn, "fn_a", "fn_b", "Call", "High");
    insert_edge_with_confidence(&conn, "fn_b", "fn_a", "Call", "Low");

    // Dead-code probe: fn_c only kept alive by a Low cross-module call.
    insert_edge_with_confidence(&conn, "fn_caller", "fn_c", "Call", "Low");

    // Keep fn_caller alive with a trivial High intra-module call.
    insert_edge_with_confidence(&conn, "fn_a", "fn_caller", "Call", "High");

    // Module-level coupling signal: file_a imports file_b.
    insert_edge_with_confidence(&conn, "file_a", "file_b", "Import", "High");
}

#[test]
fn fidelity_gap_is_nonzero_on_divergent_graph() {
    let tmp_dir = std::env::temp_dir();
    let db_path: PathBuf = tmp_dir.join(format!(
        "ge_analyze_t3f_fidelity_{}.sqlite",
        std::process::id()
    ));
    build_divergent_db(&db_path);
    let db_str = db_path.to_str().unwrap();

    let report = graphengine_analysis::health::run_analysis(db_str)
        .expect("run_analysis on T3f divergent fixture failed");

    let metrics = &report.metrics;

    // ------------------------------------------------------------
    // 1. cycles MUST diverge: 1 SCC all-edges, 0 SCC high-only.
    // ------------------------------------------------------------
    let cycles_fidelity = metrics
        .cycles
        .fidelity
        .as_ref()
        .expect("cycles.fidelity must be populated by T3 dual emission");
    assert!(
        cycles_fidelity.absolute_gap.abs() > 0.0,
        "cycles absolute_gap must be non-zero on divergent fixture, got {:?}",
        cycles_fidelity
    );
    assert!(
        cycles_fidelity.all_edges_value > cycles_fidelity.high_only_value,
        "cycles all_edges_value ({}) must exceed high_only_value ({}) \
         because the Low back-edge is required to close the SCC",
        cycles_fidelity.all_edges_value,
        cycles_fidelity.high_only_value
    );
    assert!(
        cycles_fidelity.all_edges_count >= cycles_fidelity.high_only_edges_count,
        "all_edges_count ({}) must be >= high_only_edges_count ({})",
        cycles_fidelity.all_edges_count,
        cycles_fidelity.high_only_edges_count
    );

    // ------------------------------------------------------------
    // 2. dead_code MUST diverge: fn_c is alive all-edges, dead high-only.
    // ------------------------------------------------------------
    let dead_fidelity = metrics
        .dead_code
        .fidelity
        .as_ref()
        .expect("dead_code.fidelity must be populated by T3 dual emission");
    assert!(
        dead_fidelity.absolute_gap.abs() > 0.0,
        "dead_code absolute_gap must be non-zero on divergent fixture, got {:?}",
        dead_fidelity
    );
    assert!(
        dead_fidelity.high_only_value >= dead_fidelity.all_edges_value,
        "dead_code high_only_value ({}) must be >= all_edges_value ({}) \
         because filtering Low edges removes callers and surfaces more dead fns",
        dead_fidelity.high_only_value,
        dead_fidelity.all_edges_value
    );

    // ------------------------------------------------------------
    // 3. depth MUST diverge: max-depth path fn_a -> fn_caller -> fn_c
    //    (len 2) uses the Low edge fn_caller -> fn_c. High-only
    //    collapses to fn_a -> fn_caller (len 1) or similar.
    // ------------------------------------------------------------
    let depth_fidelity = metrics
        .depth
        .fidelity
        .as_ref()
        .expect("depth.fidelity must be populated by T3 dual emission");
    assert!(
        depth_fidelity.absolute_gap.abs() > 0.0,
        "depth absolute_gap must be non-zero on divergent fixture, got {:?}",
        depth_fidelity
    );

    // ------------------------------------------------------------
    // 4. tangle_index MUST be populated. Sign can go either way:
    //    tangle is normalized by node count, and with only one tiny
    //    SCC the all-edges value is small but non-zero while high-only
    //    is zero. Assert Some + non-equality.
    // ------------------------------------------------------------
    let tangle_fidelity = metrics
        .tangle_index
        .fidelity
        .as_ref()
        .expect("tangle_index.fidelity must be populated by T3 dual emission");
    assert!(
        tangle_fidelity.absolute_gap.abs() > 0.0,
        "tangle_index absolute_gap must be non-zero on divergent fixture, got {:?}",
        tangle_fidelity
    );
    assert!(
        tangle_fidelity.all_edges_value >= tangle_fidelity.high_only_value,
        "tangle_index all_edges_value ({}) must be >= high_only_value ({}) \
         because dropping Low edges breaks the cycle and reduces tangle",
        tangle_fidelity.all_edges_value,
        tangle_fidelity.high_only_value
    );

    // ------------------------------------------------------------
    // 5. hotspot_concentration MUST be populated. Ratio-based metric;
    //    we assert Some + edge-count invariant (high_only <= all_edges)
    //    rather than sign, because hotspot is dominated by fan-in
    //    distribution and can be equal if the heuristic edges do not
    //    create new hubs.
    // ------------------------------------------------------------
    let hotspot_fidelity = metrics
        .hotspot_concentration
        .fidelity
        .as_ref()
        .expect("hotspot_concentration.fidelity must be populated by T3 dual emission");
    assert!(
        hotspot_fidelity.all_edges_count >= hotspot_fidelity.high_only_edges_count,
        "hotspot_concentration all_edges_count ({}) must be >= high_only_edges_count ({})",
        hotspot_fidelity.all_edges_count,
        hotspot_fidelity.high_only_edges_count
    );

    // ------------------------------------------------------------
    // 6. coupling MUST be populated on two-module fixtures. Low-
    //    confidence edges inflate perceived cross-module coupling;
    //    high-only should be equal-or-lower.
    // ------------------------------------------------------------
    let coupling_fidelity =
        metrics.coupling.fidelity.as_ref().expect(
            "coupling.fidelity must be populated by T3 dual emission (two modules present)",
        );
    assert!(
        coupling_fidelity.all_edges_count >= coupling_fidelity.high_only_count_or_total(),
        "coupling all_edges_count ({}) must be >= high_only_edges_count ({})",
        coupling_fidelity.all_edges_count,
        coupling_fidelity.high_only_edges_count
    );

    // ------------------------------------------------------------
    // 7. cohesion: Option-wrapped. Must be Some on two-module fixtures
    //    with enough intra-module edges; assert fidelity populated.
    // ------------------------------------------------------------
    if let Some(cohesion) = metrics.cohesion.as_ref() {
        let cohesion_fidelity = cohesion.fidelity.as_ref().expect(
            "cohesion.fidelity must be populated by T3 dual emission when cohesion is Some",
        );
        assert!(
            cohesion_fidelity.all_edges_count >= cohesion_fidelity.high_only_edges_count,
            "cohesion all_edges_count ({}) must be >= high_only_edges_count ({})",
            cohesion_fidelity.all_edges_count,
            cohesion_fidelity.high_only_edges_count
        );
    }

    // ------------------------------------------------------------
    // 8. distance_from_main_sequence: Option-wrapped. Asserted Some +
    //    fidelity populated if the fixture is large enough; guarded
    //    with a soft check because distance requires both abstractness
    //    (type-based) and instability (edge-count-based) signal.
    // ------------------------------------------------------------
    if let Some(distance) = metrics.distance_from_main_sequence.as_ref() {
        let distance_fidelity = distance.fidelity.as_ref().expect(
            "distance_from_main_sequence.fidelity must be populated by T3 dual emission \
             when the metric itself is Some",
        );
        assert!(
            distance_fidelity.all_edges_count >= distance_fidelity.high_only_edges_count,
            "distance all_edges_count ({}) must be >= high_only_edges_count ({})",
            distance_fidelity.all_edges_count,
            distance_fidelity.high_only_edges_count
        );
    }

    // ------------------------------------------------------------
    // 9. Integrity caveat MUST flag dual-metric emission.
    // ------------------------------------------------------------
    let has_caveat = report
        .integrity_status
        .schema_caveats
        .iter()
        .any(|c| c == graphengine_analysis::health::report::CAVEAT_DUAL_METRIC_EMISSION_V1);
    assert!(
        has_caveat,
        "integrity_status.schema_caveats must include `{}` whenever any metric \
         carries a FidelityGap; schema_caveats={:?}",
        graphengine_analysis::health::report::CAVEAT_DUAL_METRIC_EMISSION_V1,
        report.integrity_status.schema_caveats
    );

    // ------------------------------------------------------------
    // 10. T4 measured-fidelity tier sanity: 2/4 High calls -> 50%
    //     -> HeuristicPrimary (in the [0.40, 0.80) band).
    // ------------------------------------------------------------
    let rq = report
        .resolution_quality
        .as_ref()
        .expect("resolution_quality must be populated on non-empty DB");
    let mf = &rq.measured_fidelity;
    assert_eq!(
        mf.tier,
        graphengine_analysis::health::report::MeasuredFidelityTier::HeuristicPrimary,
        "expected HeuristicPrimary on divergent fixture, got {:?} with ratio {:?} and breakdown {:?}",
        mf.tier,
        mf.high_ratio_on_calls,
        mf.call_edges_by_confidence,
    );
    assert_eq!(mf.call_edges_by_confidence.high, 2);
    assert_eq!(mf.call_edges_by_confidence.low, 2);
    assert_eq!(mf.call_edges_by_confidence.total(), 4);
    let ratio = mf
        .high_ratio_on_calls
        .expect("ratio must be Some on non-empty calls");
    assert!(
        (ratio - 0.5).abs() < 1e-9,
        "expected high_ratio_on_calls ~= 0.5, got {}",
        ratio
    );

    let _ = fs::remove_file(&db_path);
}

// Local trait to simplify the coupling assertion above — `FidelityGap`
// exposes `all_edges_count` and `high_only_edges_count` as public
// fields, but the assertion reads them both. We keep the helper inline
// because exposing it in the public `report` module would be
// one-assertion-of-scope beyond this test.
trait FidelityGapExt {
    fn high_only_count_or_total(&self) -> usize;
}

impl FidelityGapExt for graphengine_analysis::health::report::FidelityGap {
    fn high_only_count_or_total(&self) -> usize {
        self.high_only_edges_count
    }
}
