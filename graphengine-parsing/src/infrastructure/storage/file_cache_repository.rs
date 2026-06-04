//! Persistence for the S1 incremental-scanning per-file cache.
//!
//! One row per discovered file. `file_path` is the lossy-string form
//! used by `file_extraction_coverage.file_path` and by
//! `Node.location.file`, so a join across cache and graph tables works
//! without any normalisation step.
//!
//! The orchestrator interacts with this module in three places:
//!
//! 1. **Cache load** — at scan start, [`load_all`] reads every cached
//!    row into memory so the incremental planner can look up by path
//!    without hitting SQLite for each file.
//! 2. **Cache write** — after a successful scan, [`upsert_batch`]
//!    overwrites the row for every file the orchestrator visited
//!    (changed *and* unchanged — the unchanged rows get their
//!    `cached_at` refreshed so a future user can tell when the cache
//!    was last validated).
//! 3. **Cache prune** — files that disappeared from disk between
//!    scans are removed by [`prune_missing`].
//!
//! See `docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md` §4.2–§4.7.

use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashSet};

/// One row of the `file_cache` SQLite table, in domain shape.
///
/// `payload_json` is opaque to this module — the orchestrator
/// serialises a `PerFileSlice` into it and deserialises on read.
/// Keeping the JSON opaque here means new fields on `PerFileSlice`
/// do not require schema changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCacheRow {
    pub file_path: String,
    pub content_hash: String,
    pub language: String,
    pub payload_json: String,
    pub cached_at: String,
}

/// Repository for the `file_cache` table. Stateless namespace; every
/// method takes the connection it operates on so the call site
/// controls transaction scope.
pub struct FileCacheRepository;

impl FileCacheRepository {
    /// Load every cached row, keyed by `file_path`. `BTreeMap` so the
    /// caller gets a stable iteration order for snapshot tests and
    /// for deterministic progress emission.
    ///
    /// Returns an empty map when the table is empty (e.g. fresh DB,
    /// or post-schema-bump cache invalidation). The table itself is
    /// guaranteed to exist by `SqliteRepository::migrate_schema`.
    pub fn load_all(conn: &Connection) -> anyhow::Result<BTreeMap<String, FileCacheRow>> {
        let mut stmt = conn.prepare(
            "SELECT file_path, content_hash, language, payload_json, cached_at FROM file_cache",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileCacheRow {
                file_path: row.get(0)?,
                content_hash: row.get(1)?,
                language: row.get(2)?,
                payload_json: row.get(3)?,
                cached_at: row.get(4)?,
            })
        })?;

        let mut map = BTreeMap::new();
        for row in rows {
            let row = row?;
            map.insert(row.file_path.clone(), row);
        }
        Ok(map)
    }

    /// Upsert a batch of rows in a single transaction. Existing rows
    /// for the same `file_path` are replaced wholesale — there's no
    /// partial-column update path because every field is recomputed
    /// per scan.
    pub fn upsert_batch(conn: &mut Connection, rows: &[FileCacheRow]) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO file_cache \
                 (file_path, content_hash, language, payload_json, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.file_path,
                    row.content_hash,
                    row.language,
                    row.payload_json,
                    row.cached_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete rows whose `language` column matches `language` AND
    /// whose `file_path` is not in `current_paths`. Returns the
    /// number of rows removed. Called at end-of-scan so the cache
    /// reflects the current discovery output (deleted files don't
    /// linger). Language-scoped so a polyglot scan does not have one
    /// language's pass clobber another language's cache rows when
    /// the parse DB is persistent across scans (S1-ε).
    ///
    /// Implementation note: SQLite's `IN (?)` with a list parameter
    /// needs one placeholder per item, which would force the caller
    /// to chunk for huge repos. We instead select the existing keys,
    /// compute the set difference in Rust, and DELETE one path at a
    /// time inside a transaction. For typical repo sizes (~10⁴
    /// files) this is well under 100 ms; the API stays simple.
    pub fn prune_missing(
        conn: &mut Connection,
        language: &str,
        current_paths: &HashSet<String>,
    ) -> anyhow::Result<usize> {
        let stored_paths: Vec<String> = {
            let mut stmt = conn.prepare("SELECT file_path FROM file_cache WHERE language = ?1")?;
            let rows = stmt
                .query_map(params![language], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };

        let to_delete: Vec<&String> = stored_paths
            .iter()
            .filter(|path| !current_paths.contains(*path))
            .collect();

        if to_delete.is_empty() {
            return Ok(0);
        }

        let count = to_delete.len();
        let tx = conn.transaction()?;
        {
            let mut stmt =
                tx.prepare("DELETE FROM file_cache WHERE language = ?1 AND file_path = ?2")?;
            for path in to_delete {
                stmt.execute(params![language, path])?;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// Total row count. Diagnostic helper for tests and the progress
    /// event stream.
    pub fn count(conn: &Connection) -> anyhow::Result<i64> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM file_cache", [], |row| row.get(0))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::storage::schema::SCHEMA;

    fn make_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(SCHEMA).expect("apply schema");
        conn
    }

    fn row(path: &str, hash: &str, payload: &str) -> FileCacheRow {
        FileCacheRow {
            file_path: path.to_string(),
            content_hash: hash.to_string(),
            language: "rust".to_string(),
            payload_json: payload.to_string(),
            cached_at: "2026-05-25T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn load_all_empty_table_returns_empty_map() {
        let conn = make_db();
        let map = FileCacheRepository::load_all(&conn).expect("load");
        assert!(map.is_empty());
    }

    #[test]
    fn upsert_batch_followed_by_load_round_trips_every_field() {
        let mut conn = make_db();
        let inputs = vec![
            row("/a.rs", "hash-a", r#"{"k":1}"#),
            row("/b.rs", "hash-b", r#"{"k":2}"#),
        ];
        FileCacheRepository::upsert_batch(&mut conn, &inputs).expect("upsert");

        let loaded = FileCacheRepository::load_all(&conn).expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["/a.rs"], inputs[0]);
        assert_eq!(loaded["/b.rs"], inputs[1]);
    }

    #[test]
    fn upsert_batch_replaces_existing_row_for_same_path() {
        let mut conn = make_db();
        let initial = vec![row("/a.rs", "hash-old", r#"{"old":true}"#)];
        FileCacheRepository::upsert_batch(&mut conn, &initial).expect("upsert initial");

        let updated = vec![row("/a.rs", "hash-new", r#"{"new":true}"#)];
        FileCacheRepository::upsert_batch(&mut conn, &updated).expect("upsert updated");

        let loaded = FileCacheRepository::load_all(&conn).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["/a.rs"].content_hash, "hash-new");
        assert_eq!(loaded["/a.rs"].payload_json, r#"{"new":true}"#);
    }

    #[test]
    fn upsert_batch_empty_input_is_no_op() {
        let mut conn = make_db();
        FileCacheRepository::upsert_batch(&mut conn, &[]).expect("no-op");
        assert_eq!(FileCacheRepository::count(&conn).unwrap(), 0);
    }

    #[test]
    fn prune_missing_drops_paths_absent_from_current_set() {
        let mut conn = make_db();
        let rows = vec![
            row("/keep.rs", "h1", "{}"),
            row("/drop1.rs", "h2", "{}"),
            row("/drop2.rs", "h3", "{}"),
        ];
        FileCacheRepository::upsert_batch(&mut conn, &rows).expect("upsert");

        let mut current = HashSet::new();
        current.insert("/keep.rs".to_string());
        let removed =
            FileCacheRepository::prune_missing(&mut conn, "rust", &current).expect("prune");

        assert_eq!(removed, 2);
        let loaded = FileCacheRepository::load_all(&conn).expect("load");
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("/keep.rs"));
    }

    #[test]
    fn prune_missing_no_changes_returns_zero() {
        let mut conn = make_db();
        let rows = vec![row("/a.rs", "h", "{}")];
        FileCacheRepository::upsert_batch(&mut conn, &rows).expect("upsert");

        let mut current = HashSet::new();
        current.insert("/a.rs".to_string());
        let removed =
            FileCacheRepository::prune_missing(&mut conn, "rust", &current).expect("prune");

        assert_eq!(removed, 0);
    }

    #[test]
    fn prune_missing_empty_table_returns_zero() {
        let mut conn = make_db();
        let current: HashSet<String> = HashSet::new();
        let removed =
            FileCacheRepository::prune_missing(&mut conn, "rust", &current).expect("prune");
        assert_eq!(removed, 0);
    }

    #[test]
    fn prune_missing_is_scoped_to_the_caller_language() {
        // S1-ε persistent-DB invariant: a rust pass calling
        // prune_missing("rust", …) must NEVER delete rows owned by
        // another language, even when those rows' file_paths aren't
        // in the rust pass's discovery output.
        let mut conn = make_db();

        let rust_row = FileCacheRow {
            file_path: "/a.rs".to_string(),
            content_hash: "h-rs".to_string(),
            language: "rust".to_string(),
            payload_json: "{}".to_string(),
            cached_at: "2026-05-25T00:00:00+00:00".to_string(),
        };
        let python_row = FileCacheRow {
            file_path: "/b.py".to_string(),
            content_hash: "h-py".to_string(),
            language: "python".to_string(),
            payload_json: "{}".to_string(),
            cached_at: "2026-05-25T00:00:00+00:00".to_string(),
        };
        FileCacheRepository::upsert_batch(&mut conn, &[rust_row.clone(), python_row.clone()])
            .expect("upsert");

        // Rust pass: discovery returns nothing. The prune must
        // remove the rust row but leave the python row intact.
        let removed =
            FileCacheRepository::prune_missing(&mut conn, "rust", &HashSet::new()).expect("prune");
        assert_eq!(removed, 1, "rust row should be removed");

        let loaded = FileCacheRepository::load_all(&conn).expect("load");
        assert_eq!(loaded.len(), 1);
        assert!(
            loaded.contains_key("/b.py"),
            "python row must survive a rust-scoped prune"
        );
    }

    #[test]
    fn count_returns_row_count() {
        let mut conn = make_db();
        assert_eq!(FileCacheRepository::count(&conn).unwrap(), 0);

        FileCacheRepository::upsert_batch(
            &mut conn,
            &[row("/a.rs", "h", "{}"), row("/b.rs", "h", "{}")],
        )
        .unwrap();
        assert_eq!(FileCacheRepository::count(&conn).unwrap(), 2);
    }

    #[test]
    fn load_all_returns_stable_iteration_order() {
        let mut conn = make_db();
        FileCacheRepository::upsert_batch(
            &mut conn,
            &[
                row("/z.rs", "h", "{}"),
                row("/a.rs", "h", "{}"),
                row("/m.rs", "h", "{}"),
            ],
        )
        .unwrap();

        let loaded = FileCacheRepository::load_all(&conn).expect("load");
        let keys: Vec<&String> = loaded.keys().collect();
        assert_eq!(keys, vec!["/a.rs", "/m.rs", "/z.rs"]);
    }
}
