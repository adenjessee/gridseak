//! Implements the `GraphRepository` trait using SQLite for persistent storage.
//!
//! This module provides a concrete implementation of the `GraphRepository` port,
//! allowing the application layer to store and retrieve `Graph` objects
//! (nodes and edges) in a SQLite database. It handles serialization of complex
//! types like `Range` and `Provenance` to JSON, and ensures transactional
//! integrity for graph operations.
//!
//! # Database Abstraction
//! This implementation also implements `ParsingStorageBackend` for database-agnostic
//! lifecycle management, enabling future migration to other backends if needed.

use crate::application::ports::GraphRepository;
#[cfg(test)]
use crate::domain::EdgeKind;
use crate::domain::{Edge, Graph, Node, NodeKind, Provenance, Range};
use crate::infrastructure::storage::schema::{
    PARSE_META_SCHEMA_VERSION, PARSE_META_SCHEMA_VERSION_KEY, SCHEMA,
};
use crate::infrastructure::storage::storage_backend::{ParsingStorageBackend, ParsingStorageStats};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, instrument};

/// SQLite implementation of the `GraphRepository` trait.
///
/// Manages the connection to a SQLite database and provides methods
/// for storing, retrieving, listing, and deleting graphs.
#[derive(Debug, Clone)]
pub struct SqliteRepository {
    /// The SQLite connection, wrapped in an Arc<Mutex> for thread-safe access.
    conn: Arc<Mutex<Connection>>,
}

impl SqliteRepository {
    /// Creates a new `SqliteRepository` instance, opening a connection
    /// to the specified database file and initializing the schema if needed.
    pub fn new(path: &str) -> anyhow::Result<Self> {
        info!("Opening SQLite database at: {}", path);
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?; // Initialize schema

        // Migrate existing databases: add trait_metadata column if it doesn't exist
        Self::migrate_schema(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Migrate schema for existing databases.
    ///
    /// Idempotent: every step checks for the current shape before
    /// acting, so re-running against an already-migrated DB is a
    /// no-op. This is the single path that upgrades a pre-TR-A.0 DB
    /// without forcing users to re-parse their repository — the
    /// `CAVEAT_STALE_PARSE_DB_V1` signal (emitted by `ge-analyze`) is
    /// what tells them the data in those new tables is empty and
    /// needs a re-parse to populate.
    fn migrate_schema(conn: &Connection) -> anyhow::Result<()> {
        // Check if trait_metadata column exists
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM pragma_table_info('nodes') WHERE name='trait_metadata'",
        )?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;

        if count == 0 {
            info!("Migrating database: adding trait_metadata column to nodes table");
            conn.execute("ALTER TABLE nodes ADD COLUMN trait_metadata TEXT", [])?;
            info!("Migration complete: trait_metadata column added");
        }

        // Check if properties column exists
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('nodes') WHERE name='properties'")?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;

        if count == 0 {
            info!("Migrating database: adding properties column to nodes table");
            conn.execute(
                "ALTER TABLE nodes ADD COLUMN properties TEXT NOT NULL DEFAULT '{}'",
                [],
            )?;
            info!("Migration complete: properties column added");
        }

        // TR-A.0: parse_meta + apex_class_symbols tables. `SCHEMA`
        // includes them with `CREATE TABLE IF NOT EXISTS`, but older
        // DBs opened prior to this upgrade will have run an earlier
        // `SCHEMA` string that didn't. Re-run the creation statements
        // here so a pre-TR-A.0 DB gets the tables attached before any
        // reader expects them.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS parse_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS apex_class_symbols (api_name TEXT COLLATE NOCASE PRIMARY KEY, symbols_json TEXT NOT NULL)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_apex_class_symbols_api_name ON apex_class_symbols(api_name COLLATE NOCASE)",
            [],
        )?;
        // T8 (universal-fidelity sprint). Same "create-if-missing on
        // open" discipline as `apex_class_symbols`: DBs opened by an
        // older engine will not have this table, and
        // `upsert_file_extraction_coverage_sync` below expects it to
        // exist.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_extraction_coverage (file_path TEXT PRIMARY KEY, language TEXT NOT NULL, payload_json TEXT NOT NULL)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_extraction_coverage_language ON file_extraction_coverage(language)",
            [],
        )?;

        // S1 (incremental scanning). Same create-if-missing discipline.
        // The cache stores per-file extraction payloads keyed by
        // blake3 content hash; on rescan, hash hits skip re-extraction.
        // See `docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md` §4.2.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_cache (file_path TEXT PRIMARY KEY, content_hash TEXT NOT NULL, language TEXT NOT NULL, payload_json TEXT NOT NULL, cached_at TEXT NOT NULL)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_cache_hash ON file_cache(content_hash)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS analysis_segment_cache (
                segment_id TEXT NOT NULL,
                graph_fingerprint TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (segment_id, graph_fingerprint)
            )",
            [],
        )?;

        // Cache invalidation on schema bump.
        // is strictly less than the current `PARSE_META_SCHEMA_VERSION`
        // means the producing engine wrote with a different payload
        // shape — every cached `PerFileSlice` is suspect. We drop all
        // rows before stamping the new version so the orchestrator's
        // next scan rebuilds the cache from scratch.
        //
        // Absence of `parse_meta` (None) is treated as "pre-v2",
        // which is also < current — we still drop the table even
        // though it didn't exist at the time. The DELETE is a no-op
        // in that case because the table was just created above.
        if let Some(stored) = Self::read_schema_version_conn(conn)? {
            if stored < PARSE_META_SCHEMA_VERSION {
                conn.execute("DELETE FROM file_cache", [])?;
                info!(
                    "Schema bump {} -> {}: cleared file_cache rows",
                    stored, PARSE_META_SCHEMA_VERSION
                );
            }
        } else {
            conn.execute("DELETE FROM file_cache", [])?;
        }

        // Stamp the current schema version whenever we open the DB.
        // On brand-new DBs this records the producing-engine version.
        // On pre-TR-A.0 DBs this is a no-op on the cell value
        // (writer path) — the stale-DB caveat is emitted by
        // `ge-analyze` based on the version it reads *before* any
        // writer stamps a newer value. Writers always stamp the
        // current version; readers compare.
        //
        // NB: `SqliteRepository::new` is the canonical writer entry
        // point. `ge-analyze` opens the DB read-only and therefore
        // never runs this block, which is the behaviour we want:
        // reading must not silently "upgrade" a stale DB and mask
        // the caveat.
        conn.execute(
            "INSERT OR REPLACE INTO parse_meta (key, value) VALUES (?1, ?2)",
            params![
                PARSE_META_SCHEMA_VERSION_KEY,
                PARSE_META_SCHEMA_VERSION.to_string()
            ],
        )?;

        Ok(())
    }

    /// Read the persisted Apex class-symbols schema version.
    ///
    /// Returns `None` for DBs produced before the `parse_meta` table
    /// existed at all, which `ge-analyze` treats as equivalent to
    /// version `0` for the purposes of emitting
    /// `CAVEAT_STALE_PARSE_DB_V1`.
    pub fn read_schema_version(&self) -> anyhow::Result<Option<u32>> {
        let conn = self.conn.lock().unwrap();
        Self::read_schema_version_conn(&conn)
    }

    /// Same as [`read_schema_version`] but against a borrowed
    /// connection. Factored out so callers that already hold the
    /// connection (e.g. `ge-analyze`, which opens read-only) don't
    /// have to route through an `SqliteRepository` instance.
    pub fn read_schema_version_conn(conn: &Connection) -> anyhow::Result<Option<u32>> {
        // Absence of the table is equivalent to "produced by a
        // pre-TR-A.0 engine" — fall through to `Ok(None)` rather
        // than returning an error.
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='parse_meta'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if table_exists == 0 {
            return Ok(None);
        }

        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM parse_meta WHERE key = ?1",
                params![PARSE_META_SCHEMA_VERSION_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|v| v.parse::<u32>().ok()))
    }

    /// Upsert a batch of Apex class-symbols rows in a single
    /// transaction. Keyed by dotted api name (`Outer.Inner`) under
    /// `COLLATE NOCASE` to match Apex's case-insensitive identifier
    /// semantics.
    ///
    /// The `symbols_json` column is the bincode-free JSON
    /// serialisation of `ApexClassSymbols` so a future parse DB
    /// diff'd with `jq` is human-readable without re-implementing
    /// the domain type; see `class_symbols.rs` for the contract.
    pub fn upsert_apex_class_symbols_sync(
        &self,
        symbols: &[(String, String)],
    ) -> anyhow::Result<()> {
        if symbols.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO apex_class_symbols (api_name, symbols_json) VALUES (?1, ?2)",
            )?;
            for (api_name, symbols_json) in symbols {
                stmt.execute(params![api_name, symbols_json])?;
            }
        }
        tx.commit()?;
        debug!("Upserted {} apex_class_symbols rows", symbols.len());
        Ok(())
    }

    /// Upsert a batch of per-file extraction-coverage rows in a
    /// single transaction.
    ///
    /// `records` is the parser-produced vector on
    /// `SyntaxResults.extraction_coverage`; each element is
    /// re-serialised to JSON using the tagged
    /// [`FileExtractionCoverage`] wire format so the schema is
    /// forward-compatible with new `CoverageGap` variants.
    ///
    /// Introduced in T8 (universal-fidelity sprint). Mirrors the
    /// `apex_class_symbols` upsert in shape so the two persistence
    /// paths can be reasoned about by analogy.
    pub fn upsert_file_extraction_coverage_sync(
        &self,
        records: &[crate::application::ports::FileExtractionCoverage],
    ) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO file_extraction_coverage (file_path, language, payload_json) VALUES (?1, ?2, ?3)",
            )?;
            for rec in records {
                let path_str = rec.file_path.to_string_lossy().to_string();
                let payload = serde_json::to_string(rec)
                    .map_err(|e| anyhow::anyhow!("serialise FileExtractionCoverage: {e}"))?;
                stmt.execute(params![path_str, rec.language, payload])?;
            }
        }
        tx.commit()?;
        debug!("Upserted {} file_extraction_coverage rows", records.len());
        Ok(())
    }

    /// Read every persisted `file_extraction_coverage` row and
    /// deserialise back into domain types. Used by `ge-analyze` at
    /// attach-time to restore the coverage vector onto
    /// [`crate::application::ports::SyntaxResults`]-equivalent
    /// state for T8 downgrades. Returns an empty vector for DBs
    /// produced by a pre-T8 engine (the table may or may not
    /// exist; both cases collapse to "no rows").
    pub fn read_file_extraction_coverage(
        &self,
    ) -> anyhow::Result<Vec<crate::application::ports::FileExtractionCoverage>> {
        let conn = self.conn.lock().unwrap();
        Self::read_file_extraction_coverage_conn(&conn)
    }

    /// Connection-borrowing twin of [`read_file_extraction_coverage`]
    /// so callers that opened the DB read-only (`ge-analyze`) can
    /// read without constructing an `SqliteRepository`.
    pub fn read_file_extraction_coverage_conn(
        conn: &Connection,
    ) -> anyhow::Result<Vec<crate::application::ports::FileExtractionCoverage>> {
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='file_extraction_coverage'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if table_exists == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare("SELECT payload_json FROM file_extraction_coverage")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for payload in rows {
            let record: crate::application::ports::FileExtractionCoverage =
                serde_json::from_str(&payload)
                    .map_err(|e| anyhow::anyhow!("deserialise FileExtractionCoverage row: {e}"))?;
            out.push(record);
        }
        Ok(out)
    }

    // Sync helpers for the S1 incremental scan cache. Mirror the
    // `apex_class_symbols` / `file_extraction_coverage` pair: the
    // trait dispatch path in `impl GraphRepository for SqliteRepository`
    // is async by trait contract, but the actual SQLite write is
    // synchronous and lives here so callers that already hold the
    // repository can avoid the runtime hop.

    /// Read every cached `file_cache` row, keyed by `file_path`.
    pub fn read_file_cache_sync(
        &self,
    ) -> anyhow::Result<std::collections::BTreeMap<String, super::FileCacheRow>> {
        let conn = self.conn.lock().unwrap();
        super::FileCacheRepository::load_all(&conn)
    }

    /// Upsert a batch of `file_cache` rows in a single transaction.
    pub fn upsert_file_cache_sync(&self, rows: &[super::FileCacheRow]) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        super::FileCacheRepository::upsert_batch(&mut conn, rows)?;
        debug!("Upserted {} file_cache rows", rows.len());
        Ok(())
    }

    /// Drop `file_cache` rows whose `file_path` is not in
    /// `current_paths`, restricted to rows whose `language` column
    /// matches the supplied language. Returns the number of rows
    /// removed. See `GraphRepository::prune_file_cache_missing` for
    /// the language-scoping rationale (multi-language persistent DB).
    pub fn prune_file_cache_missing_sync(
        &self,
        language: &str,
        current_paths: &std::collections::HashSet<String>,
    ) -> anyhow::Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        super::FileCacheRepository::prune_missing(&mut conn, language, current_paths)
    }

    /// Delete all `nodes`, `edges` (via `ON DELETE CASCADE`), and
    /// `file_extraction_coverage` rows that belong to any path in
    /// `file_paths`. Single transaction. Returns the number of
    /// `nodes` rows deleted — `edges` removal is implicit via the
    /// foreign-key cascade on `nodes.id`, so callers wanting an edge
    /// count would need a separate aggregate query.
    ///
    /// File-attribution lookup uses `json_extract(location,
    /// '$.file') = ?1`. There is no index on the extracted path
    /// today (the schema indexes `fqn` and `kind`), so deletion is
    /// O(nodes_total · changed_files). For a 10k-node DB and ~20
    /// changed files per incremental scan this is sub-millisecond,
    /// and adding a generated-column index would gate behind a
    /// schema bump; deferred until the cost is observable.
    fn prune_apex_class_symbol_for_path(
        tx: &rusqlite::Transaction<'_>,
        path: &str,
    ) -> anyhow::Result<()> {
        let lower = path.to_ascii_lowercase();
        if !(lower.ends_with(".cls") || lower.ends_with(".trigger")) {
            return Ok(());
        }
        let api_name = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if api_name.is_empty() {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM apex_class_symbols WHERE api_name = ?1 COLLATE NOCASE",
            rusqlite::params![api_name],
        )?;
        Ok(())
    }

    pub fn prune_files_from_graph_sync(&self, file_paths: &[String]) -> anyhow::Result<usize> {
        if file_paths.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut deleted_nodes: usize = 0;
        {
            let mut node_stmt =
                tx.prepare("DELETE FROM nodes WHERE json_extract(location, '$.file') = ?1")?;
            let mut coverage_stmt =
                tx.prepare("DELETE FROM file_extraction_coverage WHERE file_path = ?1")?;
            for path in file_paths {
                deleted_nodes += node_stmt.execute(rusqlite::params![path])?;
                coverage_stmt.execute(rusqlite::params![path])?;
                Self::prune_apex_class_symbol_for_path(&tx, path)?;
            }
        }
        tx.commit()?;
        Ok(deleted_nodes)
    }

    /// S2: stamp `parse_meta.incremental_scan_stats` for the analyzer fast path.
    pub fn write_incremental_scan_stats_sync(
        &self,
        stats: &super::parse_meta_store::IncrementalScanStats,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        super::parse_meta_store::merge_incremental_scan_stats(&conn, stats)
    }

    /// Count rows in `apex_class_symbols`. Used by integration tests
    /// to confirm population and by the rev-7 canary assertion
    /// (~2,500 rows on NPSP).
    pub fn count_apex_class_symbols(&self) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM apex_class_symbols", [], |row| {
            row.get(0)
        })?;
        Ok(count)
    }

    /// Creates a new `SqliteRepository` instance with an in-memory SQLite database.
    /// Useful for testing or temporary storage.
    pub fn new_in_memory() -> anyhow::Result<Self> {
        info!("Opening in-memory SQLite database");
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?; // Initialize schema
        Self::migrate_schema(&conn)?; // Migrate (no-op for new databases)
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Helper to upsert nodes in a transaction.
    fn upsert_nodes(tx: &Connection, nodes: &[Node]) -> anyhow::Result<()> {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO nodes (id, kind, fqn, location, provenance, properties, trait_metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for node in nodes {
            let location_json = serde_json::to_string(&node.location)?;
            let trait_metadata_json = node
                .trait_metadata
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            debug!(
                fqn = %node.fqn,
                location = %location_json,
                trait_metadata = ?node.trait_metadata,
                "storing graph node"
            );
            stmt.execute(params![
                node.id,
                format!("{:?}", node.kind),
                node.fqn,
                location_json,
                serde_json::to_string(&node.provenance)?,
                serde_json::to_string(&node.properties)?,
                trait_metadata_json
            ])?;
        }
        Ok(())
    }

    /// Helper to upsert edges in a transaction.
    fn upsert_edges(tx: &Connection, edges: &[Edge]) -> anyhow::Result<()> {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO edges (from_id, to_id, kind, provenance) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for edge in edges {
            stmt.execute(params![
                edge.from_id,
                edge.to_id,
                serde_json::to_string(&edge.kind)?,
                serde_json::to_string(&edge.provenance)?
            ])?;
        }
        Ok(())
    }

    /// Store key-value metadata alongside the graph (resolution telemetry, etc.).
    fn upsert_metadata(tx: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
        tx.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Compute and store resolution telemetry from the graph's edge data.
    /// Update the `metadata` table with telemetry counts derived from
    /// the **persisted** edges table, NOT the in-memory `edges` slice
    /// of the current upsert.
    ///
    /// History: the original implementation counted Import edges in
    /// the passed slice. That works for single-language scans where
    /// every upsert is the full graph, but polyglot scans call
    /// `upsert` once per language pass with only that pass's edges,
    /// so `import_edges` ended up reflecting whichever pass ran last
    /// rather than the cumulative count. The edges *table* was always
    /// correct (because `INSERT OR REPLACE INTO edges` keys on
    /// `(from_id, to_id, kind)`), so polyglot repos showed e.g.
    /// `119` Import edges in the table but `metadata.import_edges = 0`
    /// from the last pass that contributed only Call edges. The
    /// analyzer trusts `metadata.import_edges` over the in-graph
    /// count (`effective_import_count = stored.unwrap_or(in_memory)`,
    /// see `graphengine_analysis::health::run_analysis_with_config`),
    /// so the metadata winning meant `ResolutionTier::None` and
    /// `Low-confidence · …` for every cross-file metric — the
    /// user-visible "no Import edges in graph" warning surfaced in
    /// the Stage 11 pilot reports.
    ///
    /// The fix: count from the persisted `edges` table inside the
    /// same transaction so every pass writes the right cumulative
    /// total. We still take `_edges` as input because the signature
    /// is on the call-chain; the slice is just no longer the source
    /// of truth for the counts.
    fn store_resolution_telemetry(tx: &Connection, _edges: &[Edge]) -> anyhow::Result<()> {
        // Edge kinds are persisted as serde-tagged JSON
        // (`{"kind":"Import"}`), not bare strings. We compare on the
        // wire-form so a future format drift surfaces as a count of
        // zero and trips the analyzer's `lacks_cross_file_edges`
        // warning rather than silently passing.
        let import_count: i64 = tx.query_row(
            r#"SELECT COUNT(*) FROM edges WHERE kind = '{"kind":"Import"}'"#,
            [],
            |row| row.get(0),
        )?;
        let total_edges: i64 = tx.query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))?;

        Self::upsert_metadata(tx, "import_edges", &import_count.to_string())?;
        Self::upsert_metadata(tx, "total_edges", &total_edges.to_string())?;
        Ok(())
    }

    /// Helper to convert a SQLite row to a `Node`.
    fn row_to_node(row: &Row) -> rusqlite::Result<Node> {
        use crate::domain::TraitMetadata;

        let id: String = row.get(0)?;
        let kind_str: String = row.get(1)?;
        let fqn: String = row.get(2)?;
        let location_json: String = row.get(3)?;
        let provenance_json: String = row.get(4)?;
        let properties_json: Option<String> = row.get(5).ok();
        let trait_metadata_json: Option<String> = row.get(6).ok();

        let kind = match kind_str.as_str() {
            "Function" => NodeKind::Function,
            "Struct" => NodeKind::Struct,
            "Module" => NodeKind::Module,
            "Variable" => NodeKind::Variable,
            "Interface" => NodeKind::Interface,
            "Enum" => NodeKind::Enum,
            "Type" => NodeKind::Type,
            "Import" => NodeKind::Import,
            "Project" => NodeKind::Project,
            "Crate" => NodeKind::Crate,
            "File" => NodeKind::File,
            "Folder" => NodeKind::Folder,
            _ => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unknown NodeKind: {}", kind_str),
                    )),
                ))
            }
        };
        let location: Range = serde_json::from_str(&location_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let provenance: Provenance = serde_json::from_str(&provenance_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let properties: std::collections::HashMap<String, serde_json::Value> = properties_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();

        // Parse trait_metadata if present
        let trait_metadata =
            trait_metadata_json.and_then(|json| serde_json::from_str::<TraitMetadata>(&json).ok());

        Ok(Node {
            id,
            kind,
            fqn,
            location,
            provenance,
            properties,
            trait_metadata,
        })
    }

    /// Helper to convert a SQLite row to an `Edge`.
    fn row_to_edge(row: &Row) -> rusqlite::Result<Edge> {
        let from_id: String = row.get(0)?;
        let to_id: String = row.get(1)?;
        let kind_str: String = row.get(2)?;
        let provenance_json: String = row.get(3)?;

        use crate::domain::edge::PersistedEdgeKind;
        let kind = match PersistedEdgeKind::from_wire(&kind_str) {
            PersistedEdgeKind::Known(k) => k,
            PersistedEdgeKind::Unknown(raw) => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unknown EdgeKind in parse.db: {raw}"),
                    )),
                ));
            }
        };
        let provenance: Provenance = serde_json::from_str(&provenance_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(Edge {
            from_id,
            to_id,
            kind,
            provenance,
        })
    }
}

// =============================================================================
// ParsingStorageBackend Implementation
// =============================================================================
// This provides database-agnostic lifecycle management

impl ParsingStorageBackend for SqliteRepository {
    fn initialize(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        Self::migrate_schema(&conn)?;
        Ok(())
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        Self::migrate_schema(&conn)
    }

    fn optimize(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("VACUUM", [])?;
        conn.execute("ANALYZE", [])?;
        Ok(())
    }

    fn clear(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM edges", [])?;
        tx.execute("DELETE FROM nodes", [])?;
        tx.commit()?;
        Ok(())
    }

    fn health_check(&self) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result: i32 = conn.query_row("SELECT 1", [], |row| row.get(0))?;
        Ok(result == 1)
    }

    fn stats(&self) -> anyhow::Result<ParsingStorageStats> {
        let conn = self.conn.lock().unwrap();
        let node_count: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?;
        let edge_count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))?;

        // Get database size if possible
        let size_bytes = conn
            .query_row(
                "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(|s| s as u64);

        Ok(ParsingStorageStats {
            node_count: node_count as u64,
            edge_count: edge_count as u64,
            size_bytes,
        })
    }
}

// =============================================================================
// GraphRepository Implementation
// =============================================================================
// This provides the high-level graph CRUD operations

#[async_trait]
impl GraphRepository for SqliteRepository {
    #[instrument(skip(self, graph))]
    async fn upsert(&self, graph: &Graph) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        Self::upsert_nodes(&tx, &graph.nodes)?;
        Self::upsert_edges(&tx, &graph.edges)?;
        Self::store_resolution_telemetry(&tx, &graph.edges)?;
        for (key, value) in &graph.metadata {
            Self::upsert_metadata(&tx, key, value)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// TR-A.0: trait-level persistence entry point for the Apex
    /// class-symbols payload. Delegates to the inherent
    /// [`SqliteRepository::upsert_apex_class_symbols`]. Kept as a
    /// thin trampoline so any future call path that goes through
    /// `&dyn GraphRepository` (the orchestrator does) reaches the
    /// same transactional write as direct callers.
    async fn upsert_apex_class_symbols(&self, symbols: &[(String, String)]) -> anyhow::Result<()> {
        SqliteRepository::upsert_apex_class_symbols_sync(self, symbols)
    }

    /// T8 (universal-fidelity sprint): trait-level persistence
    /// entry point for per-file extraction coverage. Delegates to
    /// the inherent
    /// [`SqliteRepository::upsert_file_extraction_coverage_sync`]
    /// for the same reason as the `apex_class_symbols` trampoline
    /// directly above: orchestrator writes flow through
    /// `&dyn GraphRepository`, so both sync and trait paths must
    /// land in the same transactional write.
    async fn upsert_file_extraction_coverage(
        &self,
        records: &[crate::application::ports::FileExtractionCoverage],
    ) -> anyhow::Result<()> {
        SqliteRepository::upsert_file_extraction_coverage_sync(self, records)
    }

    /// S1 (incremental scanning): trait-level entry points for the
    /// per-file cache. Each delegates to the inherent `*_sync`
    /// helper for the same reason the other persistence trampolines
    /// do: orchestrator code flows through `&dyn GraphRepository`,
    /// so trait + sync paths must land in identical writes.
    async fn read_file_cache(
        &self,
    ) -> anyhow::Result<std::collections::BTreeMap<String, super::FileCacheRow>> {
        SqliteRepository::read_file_cache_sync(self)
    }

    async fn upsert_file_cache(&self, rows: &[super::FileCacheRow]) -> anyhow::Result<()> {
        SqliteRepository::upsert_file_cache_sync(self, rows)
    }

    async fn prune_file_cache_missing(
        &self,
        language: &str,
        current_paths: &std::collections::HashSet<String>,
    ) -> anyhow::Result<usize> {
        SqliteRepository::prune_file_cache_missing_sync(self, language, current_paths)
    }

    async fn prune_files_from_graph(&self, file_paths: &[String]) -> anyhow::Result<usize> {
        SqliteRepository::prune_files_from_graph_sync(self, file_paths)
    }

    async fn write_incremental_scan_stats(
        &self,
        stats: &super::parse_meta_store::IncrementalScanStats,
    ) -> anyhow::Result<()> {
        SqliteRepository::write_incremental_scan_stats_sync(self, stats)
    }

    #[instrument(skip(self))]
    async fn get(&self, id: &str) -> anyhow::Result<Option<Graph>> {
        let conn = self.conn.lock().unwrap();
        let mut nodes_stmt =
            conn.prepare("SELECT id, kind, fqn, location, provenance, properties, trait_metadata FROM nodes WHERE id = ?1")?;
        let mut edges_from_stmt =
            conn.prepare("SELECT from_id, to_id, kind, provenance FROM edges WHERE from_id = ?1")?;
        let mut edges_to_stmt =
            conn.prepare("SELECT from_id, to_id, kind, provenance FROM edges WHERE to_id = ?1")?;

        let node_opt = nodes_stmt
            .query_row(params![id], Self::row_to_node)
            .optional()?;

        if let Some(node) = node_opt {
            let mut graph = Graph::new();
            graph.add_node(node);

            // Collect all connected node IDs
            let mut connected_node_ids = std::collections::HashSet::new();

            for edge in edges_from_stmt.query_map(params![id], Self::row_to_edge)? {
                let edge = edge?;
                graph.add_edge(edge.clone());
                connected_node_ids.insert(edge.to_id);
            }
            for edge in edges_to_stmt.query_map(params![id], Self::row_to_edge)? {
                let edge = edge?;
                graph.add_edge(edge.clone());
                connected_node_ids.insert(edge.from_id);
            }

            // Retrieve all connected nodes
            for connected_id in connected_node_ids {
                if let Ok(connected_node) = conn.query_row(
                    "SELECT id, kind, fqn, location, provenance, properties, trait_metadata FROM nodes WHERE id = ?1",
                    params![connected_id],
                    Self::row_to_node,
                ) {
                    graph.add_node(connected_node);
                }
            }

            Ok(Some(graph))
        } else {
            Ok(None)
        }
    }

    #[instrument(skip(self))]
    async fn list(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT DISTINCT id FROM nodes")?;
        let node_ids = stmt
            .query_map(params![], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(node_ids)
    }

    #[instrument(skip(self))]
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let rows_affected = conn.execute("DELETE FROM nodes WHERE id = ?1", params![id])?;
        if rows_affected == 0 {
            debug!("No node found with ID: {}", id);
        } else {
            info!("Deleted node with ID: {}", id);
        }
        Ok(())
    }

    #[instrument(skip(self))]
    async fn clear(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        // Delete all edges first (due to foreign key constraints)
        let edges_deleted = tx.execute("DELETE FROM edges", [])?;
        info!("Cleared {} edges from database", edges_deleted);

        // Delete all nodes
        let nodes_deleted = tx.execute("DELETE FROM nodes", [])?;
        info!("Cleared {} nodes from database", nodes_deleted);

        tx.commit()?;
        info!("Database cleared successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Confidence, ProvenanceSource};
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_sqlite_repository_new_and_schema() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_str().unwrap();
        let repo = SqliteRepository::new(path).unwrap();

        let conn = repo.conn.lock().unwrap();
        // Check if tables exist
        conn.query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='nodes'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
        conn.query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='edges'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_sqlite_repository_new_in_memory() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let conn = repo.conn.lock().unwrap();
        conn.query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='nodes'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_upsert_and_get_node() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let node = Node::function(
            "test::func".to_string(),
            Range::with_file(1, 0, 1, 10, "test.rs".to_string()),
        );
        let mut graph = Graph::new();
        graph.add_node(node.clone());

        repo.upsert(&graph).await.unwrap();

        let retrieved_graph = repo.get(&node.id).await.unwrap().unwrap();
        assert_eq!(retrieved_graph.node_count(), 1);
        assert_eq!(retrieved_graph.get_node(&node.id).unwrap().fqn, node.fqn);
    }

    #[tokio::test]
    async fn test_upsert_and_get_edge() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 1, 10, "test.rs".to_string()),
        );
        let node2 = Node::function(
            "test::func2".to_string(),
            Range::with_file(2, 0, 2, 10, "test.rs".to_string()),
        );
        let edge = Edge::call(node1.id.clone(), node2.id.clone(), Provenance::lsp());

        let mut graph = Graph::new();
        graph.add_node(node1.clone());
        graph.add_node(node2.clone());
        graph.add_edge(edge.clone());

        repo.upsert(&graph).await.unwrap();

        let retrieved_graph = repo.get(&node1.id).await.unwrap().unwrap();
        assert_eq!(retrieved_graph.node_count(), 2);
        assert_eq!(retrieved_graph.edge_count(), 1);
        assert_eq!(
            retrieved_graph.get_edges_from(&node1.id)[0].kind,
            EdgeKind::Call
        );
    }

    #[tokio::test]
    async fn test_upsert_duplicate_node() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let node = Node::function(
            "test::func".to_string(),
            Range::with_file(1, 0, 1, 10, "test.rs".to_string()),
        );
        let mut graph = Graph::new();
        graph.add_node(node.clone());

        repo.upsert(&graph).await.unwrap();
        // Modify node and upsert again
        let mut modified_node = node.clone();
        modified_node.provenance = Provenance::new(ProvenanceSource::Heuristic, Confidence::Low);
        let mut modified_graph = Graph::new();
        modified_graph.add_node(modified_node.clone());
        repo.upsert(&modified_graph).await.unwrap();

        let retrieved_graph = repo.get(&node.id).await.unwrap().unwrap();
        assert_eq!(retrieved_graph.node_count(), 1);
        assert_eq!(
            retrieved_graph.get_node(&node.id).unwrap().provenance,
            modified_node.provenance
        );
    }

    #[tokio::test]
    async fn test_delete_node() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 1, 10, "test.rs".to_string()),
        );
        let node2 = Node::function(
            "test::func2".to_string(),
            Range::with_file(2, 0, 2, 10, "test.rs".to_string()),
        );
        let edge = Edge::call(node1.id.clone(), node2.id.clone(), Provenance::lsp());

        let mut graph = Graph::new();
        graph.add_node(node1.clone());
        graph.add_node(node2.clone());
        graph.add_edge(edge.clone());

        repo.upsert(&graph).await.unwrap();
        assert!(repo.get(&node1.id).await.unwrap().is_some());

        repo.delete(&node1.id).await.unwrap();
        assert!(repo.get(&node1.id).await.unwrap().is_none());
        // Deleting node1 should also cascade delete the edge
        assert!(repo.get(&node2.id).await.unwrap().is_some()); // node2 still exists
        let retrieved_graph_node2 = repo.get(&node2.id).await.unwrap().unwrap();
        assert!(retrieved_graph_node2.get_edges_to(&node2.id).is_empty()); // but no edges to it from node1
    }

    #[tokio::test]
    async fn prune_files_from_graph_removes_nodes_and_cascades_to_edges() {
        // S1-ε pre-extraction cleanup: when a file's content
        // changes, its old nodes (which had different body-hash IDs)
        // must be removed before re-extraction so the next UPSERT
        // doesn't leave stale rows behind. `ON DELETE CASCADE` on
        // `edges` removes incident edges automatically.
        let repo = SqliteRepository::new_in_memory().unwrap();
        let n_a = Node::function(
            "mod::a".to_string(),
            Range::with_file(1, 0, 1, 10, "a.rs".to_string()),
        );
        let n_b = Node::function(
            "mod::b".to_string(),
            Range::with_file(2, 0, 2, 10, "b.rs".to_string()),
        );
        let n_c = Node::function(
            "mod::c".to_string(),
            Range::with_file(3, 0, 3, 10, "c.rs".to_string()),
        );
        let e_ab = Edge::call(n_a.id.clone(), n_b.id.clone(), Provenance::lsp());
        let e_bc = Edge::call(n_b.id.clone(), n_c.id.clone(), Provenance::lsp());

        let mut graph = Graph::new();
        for n in [&n_a, &n_b, &n_c] {
            graph.add_node(n.clone());
        }
        for e in [&e_ab, &e_bc] {
            graph.add_edge(e.clone());
        }
        repo.upsert(&graph).await.unwrap();

        let deleted = repo
            .prune_files_from_graph(&["b.rs".to_string()])
            .await
            .unwrap();
        assert_eq!(deleted, 1, "should delete the single node in b.rs");

        assert!(
            repo.get(&n_a.id).await.unwrap().is_some(),
            "a.rs node survives"
        );
        assert!(
            repo.get(&n_b.id).await.unwrap().is_none(),
            "b.rs node deleted"
        );
        assert!(
            repo.get(&n_c.id).await.unwrap().is_some(),
            "c.rs node survives"
        );

        // The a→b edge and b→c edge must both be cascade-removed
        // because their shared endpoint `n_b` is gone.
        let g_a = repo.get(&n_a.id).await.unwrap().unwrap();
        assert!(g_a.get_edges_from(&n_a.id).is_empty(), "a→b edge cascaded");
        let g_c = repo.get(&n_c.id).await.unwrap().unwrap();
        assert!(g_c.get_edges_to(&n_c.id).is_empty(), "b→c edge cascaded");
    }

    #[tokio::test]
    async fn prune_files_from_graph_empty_input_is_noop() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let n = Node::function(
            "mod::a".to_string(),
            Range::with_file(1, 0, 1, 10, "a.rs".to_string()),
        );
        let mut graph = Graph::new();
        graph.add_node(n.clone());
        repo.upsert(&graph).await.unwrap();

        let deleted = repo.prune_files_from_graph(&[]).await.unwrap();
        assert_eq!(deleted, 0);
        assert!(repo.get(&n.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn prune_files_from_graph_also_drops_file_extraction_coverage() {
        use crate::application::ports::{CoverageConfidence, FileExtractionCoverage};

        let repo = SqliteRepository::new_in_memory().unwrap();
        let cov = FileExtractionCoverage {
            file_path: std::path::PathBuf::from("doomed.rs"),
            language: "rust".to_string(),
            walked_node_count: 0,
            unwalked_node_count: 0,
            coverage_gaps: vec![],
            confidence: CoverageConfidence::High,
        };
        repo.upsert_file_extraction_coverage(std::slice::from_ref(&cov))
            .await
            .unwrap();

        // Sanity: row exists before prune.
        let count_before: i64 = repo
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM file_extraction_coverage WHERE file_path = ?1",
                rusqlite::params!["doomed.rs"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count_before, 1);

        repo.prune_files_from_graph(&["doomed.rs".to_string()])
            .await
            .unwrap();

        let count_after: i64 = repo
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM file_extraction_coverage WHERE file_path = ?1",
                rusqlite::params!["doomed.rs"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count_after, 0);
    }

    #[tokio::test]
    async fn test_list_graphs() {
        let repo = SqliteRepository::new_in_memory().unwrap();
        let node1 = Node::function(
            "test::func1".to_string(),
            Range::with_file(1, 0, 1, 10, "test.rs".to_string()),
        );
        let node2 = Node::function(
            "test::func2".to_string(),
            Range::with_file(2, 0, 2, 10, "test.rs".to_string()),
        );

        let mut graph1 = Graph::new();
        graph1.add_node(node1.clone());
        repo.upsert(&graph1).await.unwrap();

        let mut graph2 = Graph::new();
        graph2.add_node(node2.clone());
        repo.upsert(&graph2).await.unwrap();

        let listed_ids = repo.list().await.unwrap();
        assert_eq!(listed_ids.len(), 2);
        assert!(listed_ids.contains(&node1.id));
        assert!(listed_ids.contains(&node2.id));
    }
}
