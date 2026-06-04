//! TR-A.0 end-to-end: the Apex extractor populates `ApexClassSymbols`
//! on the existing parse pass, the orchestrator persists them to the
//! `apex_class_symbols` SQLite table, and the `parse_meta` table
//! records the current schema version.
//!
//! This test is the spec the rev-7 NPSP canary depends on:
//!
//! 1. Every `.cls` file in the committed Apex corpus contributes at
//!    least its top-level class to `SyntaxResults.class_symbols`.
//! 2. Inner-class declarations also emit rows (load-bearing for TR-A.6
//!    inner-class dispatch). The `Outer.Inner` dotted shape is the
//!    same shape the resolver consumes through `DottedPathProvider`.
//! 3. The `.trigger` carve-out holds: trigger files are explicitly
//!    skipped for TR-A.0 (implicit-context variables need a sibling
//!    `TriggerSymbols` type deferred to Phase B).
//! 4. Parse-DB schema is stamped with the current
//!    `PARSE_META_SCHEMA_VERSION` value so `ge-analyze` will not fire
//!    `CAVEAT_STALE_PARSE_DB_V1` against a freshly-produced DB.
//!
//! Failing any of these assertions means the Phase A resolver work
//! (PRs 2–5) cannot reach the symbols it depends on. All four
//! assertions are blockers, not advisory — do not relax them without
//! owning the downstream consequences.

use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::infrastructure::storage::schema::{
    PARSE_META_SCHEMA_VERSION, PARSE_META_SCHEMA_VERSION_KEY,
};
use graphengine_parsing::infrastructure::storage::sqlite_repository::SqliteRepository;
use graphengine_parsing::syntax::language::apex::APEX_CLASS_SYMBOLS_SCHEMA_VERSION;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn corpus_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("apex")
        .join("force-app")
        .join("main")
        .join("default")
        .join("classes");
    std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", root.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cls"))
        .collect()
}

#[tokio::test]
async fn class_symbols_payload_reaches_syntax_results_for_every_cls_file() {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let results = extractor
        .extract(&corpus_files())
        .await
        .expect("extract corpus");

    assert!(
        !results.class_symbols.is_empty(),
        "apex corpus produced zero class_symbols rows — the extractor hook is unwired"
    );

    // Every top-level class name (file stem) must appear as an
    // `api_name` key. This is what PRs 2–5 rely on: a resolver arm
    // that asks the registry for "Foo" expects a symbols row for
    // "Foo" any time the corpus contains `Foo.cls`.
    let top_level_keys: std::collections::BTreeSet<String> = results
        .class_symbols
        .iter()
        .map(|(api, _)| api.split('.').next().unwrap_or("").to_string())
        .collect();

    for file in corpus_files() {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if stem.is_empty() {
            continue;
        }
        assert!(
            top_level_keys.contains(stem),
            "no class_symbols entry for `{}` (expected from {:?})",
            stem,
            file
        );
    }
}

#[tokio::test]
async fn class_symbols_payload_captures_inner_classes_with_dotted_api_names() {
    // This fixture ships at least one `Outer.Inner` shape
    // (`apex_inner_class_propagation_e2e` is the end-to-end
    // coverage); we assert the dotted key exists so TR-A.6's
    // containment walker has something to resolve.
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let results = extractor
        .extract(&corpus_files())
        .await
        .expect("extract corpus");

    let has_dotted_key = results
        .class_symbols
        .iter()
        .any(|(api, _)| api.contains('.'));

    assert!(
        has_dotted_key,
        "apex corpus exposed no Outer.Inner dotted symbols \u{2014} TR-A.6 resolver arms cannot hit"
    );
}

#[tokio::test]
async fn trigger_files_are_scoped_out_of_class_symbols() {
    // Build a synthetic `.trigger` file alongside the corpus and
    // confirm it contributes zero class_symbols rows. The carve-out
    // is a documented TR-A.0 boundary — triggers need a separate
    // `TriggerSymbols` type deferred to Phase B.
    let tmp = tempfile::tempdir().unwrap();
    let trigger_path = tmp.path().join("SomeTrigger.trigger");
    std::fs::write(
        &trigger_path,
        "trigger SomeTrigger on Account (before insert) { System.debug('hi'); }",
    )
    .unwrap();

    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("build apex extractor");
    let results = extractor
        .extract(std::slice::from_ref(&trigger_path))
        .await
        .expect("extract trigger file");

    // Apex `.trigger` files still parse under the Apex grammar and
    // may yield *file-module* symbols and managed-package synthesis,
    // but they MUST NOT yield any `class_symbols` rows in TR-A.0.
    assert!(
        results.class_symbols.is_empty(),
        "trigger files must not contribute class_symbols rows; got: {:?}",
        results
            .class_symbols
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn fresh_parse_db_stamps_current_schema_version_in_parse_meta() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();
    let _repo = SqliteRepository::new(&db_path).expect("create repo");

    let conn = Connection::open(&db_path).unwrap();
    let version: String = conn
        .query_row(
            "SELECT value FROM parse_meta WHERE key = ?1",
            rusqlite::params![PARSE_META_SCHEMA_VERSION_KEY],
            |row| row.get(0),
        )
        .expect("parse_meta.schema_version must be present");
    assert_eq!(
        version.parse::<u32>().unwrap(),
        PARSE_META_SCHEMA_VERSION,
        "writer path must stamp the current PARSE_META_SCHEMA_VERSION"
    );
    // The parse-DB version is a per-DB rollup of every persisted
    // payload's shape. The Apex class-symbols payload's own shape
    // version is independent — it tracks only the JSON layout of
    // `apex_class_symbols.symbols_json`. The relationship is:
    //
    //   PARSE_META_SCHEMA_VERSION >= APEX_CLASS_SYMBOLS_SCHEMA_VERSION
    //
    // ie. the parse-DB version is at least as new as the Apex payload
    // version. S1 (incremental scanning) bumped PARSE_META to 3 to
    // signal the new `file_cache` table; the Apex payload shape did
    // not change in that bump, so APEX_CLASS_SYMBOLS_SCHEMA_VERSION
    // stayed at 2. A future Apex payload edit must bump *both*
    // constants; this assertion only catches the rarer case of an
    // Apex payload bump that forgets to bump the parse-DB rollup.
    // Both versions are `const`, so the comparison evaluates at
    // compile time. Bind through a `const _: ()` so the check is
    // a static assertion rather than a runtime `assert!(true)` —
    // clippy correctly flags the runtime form as a no-op.
    const _: () = assert!(
        PARSE_META_SCHEMA_VERSION >= APEX_CLASS_SYMBOLS_SCHEMA_VERSION,
        "parse-DB schema version must be >= Apex class-symbols domain version"
    );
}

#[test]
fn apex_class_symbols_table_exists_and_supports_collate_nocase_lookups() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_string_lossy().to_string();
    let repo = SqliteRepository::new(&db_path).expect("create repo");

    // Minimal payload: one api name, round-trip it through the
    // repo's trait method via async_trait; we call the sync helper
    // directly to avoid needing a tokio runtime in this test.
    repo.upsert_apex_class_symbols_sync(&[
        ("Outer".to_string(), r#"{"fields":[]}"#.to_string()),
        (
            "Outer.Inner".to_string(),
            r#"{"fields":[{"name":"payload"}]}"#.to_string(),
        ),
    ])
    .expect("upsert symbols");

    assert_eq!(repo.count_apex_class_symbols().unwrap(), 2);

    // COLLATE NOCASE on the api_name column: a lookup for the
    // lower-case form MUST hit the upper-case row.
    let conn = Connection::open(&db_path).unwrap();
    let json: String = conn
        .query_row(
            "SELECT symbols_json FROM apex_class_symbols WHERE api_name = ?1",
            rusqlite::params!["outer"],
            |row| row.get(0),
        )
        .expect("case-insensitive lookup must succeed");
    assert!(json.contains("fields"));
}
