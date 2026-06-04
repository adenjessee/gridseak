//! Defines the SQLite database schema for storing nodes and edges.

/// Current persisted schema version. Persisted in the `parse_meta`
/// table under the [`PARSE_META_SCHEMA_VERSION_KEY`] key.
///
/// When a parse DB is opened for analysis, `ge-analyze` compares the
/// stored value to this constant. A lower stored value means the DB
/// was produced by an older engine and the analysis emits
/// `CAVEAT_STALE_PARSE_DB_V1` so downstream consumers know the data
/// may have shape mismatches.
///
/// Version history:
/// * `1` — pre-TR-A.0 (no `parse_meta` table at all; treated as `0`).
/// * `2` — TR-A.0 shipped: `parse_meta`, `apex_class_symbols`, and
///   later `file_extraction_coverage` tables.
/// * `3` — S1 incremental scanning: adds the `file_cache` table that
///   stores per-file extraction payloads keyed by blake3 content
///   hash. Bumping to 3 invalidates any v2 parse DB's cache rows
///   (they didn't exist, so this is a no-op for existing DBs).
///
/// Bumping this constant is a **breaking** change for persistence. Any
/// edit that changes the JSON shape of a payload column MUST bump this
/// and add a migration to `migrate_schema` in `sqlite_repository.rs`.
/// The S1 cache treats a version mismatch as "every row invalid" so the
/// orchestrator falls back to a full re-extract.
pub const PARSE_META_SCHEMA_VERSION: u32 = 4;

/// Key under which [`PARSE_META_SCHEMA_VERSION`] is stored in the
/// `parse_meta` table. Kept as a constant so both writer and reader
/// paths reference the same literal.
pub const PARSE_META_SCHEMA_VERSION_KEY: &str = "schema_version";

/// SQL schema for creating the necessary tables and indexes.
///
/// Table ownership:
/// * `nodes` / `edges` — graph storage.
/// * `metadata` — transient per-parse-run telemetry (`import_edges`,
///   `total_edges`). Overwritten on every upsert.
/// * `parse_meta` — durable parse-DB metadata that must survive
///   regeneration-free (the schema version the DB was produced under).
///   Kept separate from `metadata` on purpose so a resolver-telemetry
///   upsert cannot accidentally clobber the schema version.
/// * `apex_class_symbols` — Apex type-oracle payload per user-declared
///   class. Keyed by dotted api name (`Outer.Inner`). See
///   `graphengine-parsing/src/domain/apex/class_symbols.rs`
///   for the payload shape; see `symbols_repository.rs` for the
///   serialisation contract.
///
///   `api_name TEXT COLLATE NOCASE PRIMARY KEY` matches Apex's
///   case-insensitive identifier semantics so `foo` and `FOO` cannot
///   produce two rows.
pub const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    fqn TEXT NOT NULL,
    location TEXT NOT NULL,  -- JSON: {\"file\": \"...\", \"start_line\": 10, ...}
    provenance TEXT NOT NULL, -- JSON: {\"source\": \"Lsp\", \"confidence\": \"High\"}
    properties TEXT NOT NULL DEFAULT '{}', -- JSON object: stable client/template properties (classification, paths, etc.)
    trait_metadata TEXT       -- JSON: {\"trait_name\": \"...\", \"is_trait_default\": true, \"implementing_type\": \"...\"} or NULL
);
CREATE INDEX IF NOT EXISTS idx_nodes_fqn ON nodes(fqn);
CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);

CREATE TABLE IF NOT EXISTS edges (
    from_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    to_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL, -- JSON as above
    PRIMARY KEY (from_id, to_id, kind) -- Composite primary key to prevent duplicate edges
);
CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);

CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS parse_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS apex_class_symbols (
    api_name TEXT COLLATE NOCASE PRIMARY KEY,
    symbols_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_apex_class_symbols_api_name
    ON apex_class_symbols(api_name COLLATE NOCASE);

-- T8 (universal-fidelity sprint) per-file extraction coverage.
-- One row per parsed file whose language ran a coverage pass.
-- `file_path` is the string the parser recorded; the analysis
-- binary compares it byte-identically against `NodeAnnotation.
-- file_path` so both sides see the same identifier. `payload_json`
-- holds the serialised `FileExtractionCoverage` (tagged variants
-- for `coverage_gaps`) so the schema does not need to grow a new
-- column each time a `CoverageGap` variant is added.
CREATE TABLE IF NOT EXISTS file_extraction_coverage (
    file_path TEXT PRIMARY KEY,
    language TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_file_extraction_coverage_language
    ON file_extraction_coverage(language);

-- S1 (incremental scanning) per-file extraction cache. One row per
-- discovered file. `file_path` is the same lossy-string form used by
-- `file_extraction_coverage.file_path` and by `Node.location.file`
-- so cross-table joins stay byte-identical. `content_hash` is the
-- blake3 hex digest of the file's raw bytes (so comment/whitespace
-- edits invalidate). `payload_json` is the serialised `PerFileSlice`
-- — the subset of `SyntaxResults` attributable to this file. On
-- rescan the orchestrator hashes every discovered file, looks up the
-- matching row, and skips re-extraction for hits. See
-- `docs/02-strategy/S1_INCREMENTAL_SCANNING_DESIGN.md` §4.2.
CREATE TABLE IF NOT EXISTS file_cache (
    file_path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    language TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    cached_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_file_cache_hash ON file_cache(content_hash);

CREATE TABLE IF NOT EXISTS analysis_segment_cache (
    segment_id TEXT NOT NULL,
    graph_fingerprint TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (segment_id, graph_fingerprint)
);
";
