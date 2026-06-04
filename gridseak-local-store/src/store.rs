use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use graphengine_analysis::health::report::HealthReport;
use graphengine_diagnostic::priority;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone)]
pub struct ProjectStore {
    db_path: PathBuf,
    reports_dir: PathBuf,
    graphs_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectDto {
    pub id: String,
    pub display_name: String,
    pub created_at: String,
    pub updated_at: String,
    pub storage_mode: String,
    pub sync_status: String,
    pub last_scan_run_id: Option<String>,
    pub roots: Vec<ProjectRootDto>,
    pub latest_scan: Option<ScanRunDto>,
    pub latest_metrics: Option<MetricSnapshotDto>,
    pub scan_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectRootDto {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub kind: String,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScanRunDto {
    pub id: String,
    pub project_id: String,
    pub root_id: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub app_version: String,
    pub engine_version: String,
    pub report_path: Option<String>,
    pub graph_artifact_path: Option<String>,
    pub ai_summary_json_path: Option<String>,
    pub ai_summary_md_path: Option<String>,
    pub sync_status: String,
    pub duration_ms: Option<i64>,
    pub primary_language: Option<String>,
    pub scan_languages: Vec<String>,
    pub git_branch: Option<String>,
    pub git_commit: Option<String>,
    pub git_dirty: Option<bool>,
    pub scan_trigger: String,
    pub requested_by: Option<String>,
    pub metrics: Option<MetricSnapshotDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MetricSnapshotDto {
    pub health_score: Option<f64>,
    pub finding_count: i64,
    pub critical_count: i64,
    pub high_count: i64,
    pub total_nodes: i64,
    pub total_edges: i64,
    pub total_modules: i64,
    pub total_functions: i64,
    pub total_files: Option<i64>,
    pub cycle_count: i64,
    pub hotspot_count: i64,
    pub dead_code_count: i64,
    pub avg_coupling: Option<f64>,
    pub propagation_cost: Option<f64>,
    pub percentile: Option<f64>,
    pub included_modules: Option<i64>,
    pub excluded_modules: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectDetailDto {
    pub project: ProjectDto,
    pub scans: Vec<ScanRunDto>,
}

/// Locally-stored row from the `feedback` table.
///
/// The shadow-mode `gridseak feedback "<text>"` command appends one of
/// these rows so the user has a durable record of what they wished the
/// tool could do. It never leaves the machine unless the user
/// explicitly exports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackDto {
    pub id: String,
    pub project_id: Option<String>,
    pub created_at: String,
    pub app_version: String,
    pub source: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitContext {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub dirty: Option<bool>,
}

impl GitContext {
    pub fn from_path(path: &Path) -> Result<Self> {
        let branch = git_output(path, ["rev-parse", "--abbrev-ref", "HEAD"]).ok();
        let commit = git_output(path, ["rev-parse", "HEAD"]).ok();
        let dirty = git_output(path, ["status", "--porcelain"])
            .ok()
            .map(|s| !s.trim().is_empty());
        Ok(Self {
            branch,
            commit,
            dirty,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BeginScanRecord {
    pub scan_id: Uuid,
    pub project_id: String,
    pub root_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub app_version: String,
    pub engine_version: String,
    pub primary_language: Option<String>,
    pub scan_languages: Vec<String>,
    pub git: GitContext,
    pub scan_trigger: String,
    pub requested_by: Option<String>,
}

impl ProjectStore {
    pub fn open(db_path: PathBuf, reports_dir: PathBuf, graphs_dir: PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::create_dir_all(&reports_dir)
            .with_context(|| format!("create {}", reports_dir.display()))?;
        std::fs::create_dir_all(&graphs_dir)
            .with_context(|| format!("create {}", graphs_dir.display()))?;

        let store = Self {
            db_path,
            reports_dir,
            graphs_dir,
        };
        store.init()?;
        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn conn(&self) -> Result<Connection> {
        Connection::open(&self.db_path)
            .with_context(|| format!("open project db {}", self.db_path.display()))
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS projects (
              id TEXT PRIMARY KEY,
              display_name TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              storage_mode TEXT NOT NULL DEFAULT 'local_only',
              sync_status TEXT NOT NULL DEFAULT 'local_only',
              last_scan_run_id TEXT
            );

            CREATE TABLE IF NOT EXISTS project_roots (
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
              path TEXT NOT NULL UNIQUE,
              kind TEXT NOT NULL DEFAULT 'primary',
              added_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS scan_runs (
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
              root_id TEXT REFERENCES project_roots(id) ON DELETE SET NULL,
              started_at TEXT NOT NULL,
              completed_at TEXT,
              status TEXT NOT NULL,
              app_version TEXT NOT NULL,
              engine_version TEXT NOT NULL,
              report_path TEXT,
              graph_artifact_path TEXT,
              ai_summary_json_path TEXT,
              ai_summary_md_path TEXT,
              sync_status TEXT NOT NULL DEFAULT 'local_only',
              duration_ms INTEGER,
              primary_language TEXT,
              scan_languages TEXT,
              git_branch TEXT,
              git_commit TEXT,
              git_dirty INTEGER,
              scan_trigger TEXT NOT NULL DEFAULT 'desktop',
              requested_by TEXT
            );

            CREATE TABLE IF NOT EXISTS metric_snapshots (
              scan_run_id TEXT PRIMARY KEY REFERENCES scan_runs(id) ON DELETE CASCADE,
              health_score REAL,
              finding_count INTEGER NOT NULL DEFAULT 0,
              critical_count INTEGER NOT NULL DEFAULT 0,
              high_count INTEGER NOT NULL DEFAULT 0,
              total_nodes INTEGER NOT NULL DEFAULT 0,
              total_edges INTEGER NOT NULL DEFAULT 0,
              total_modules INTEGER NOT NULL DEFAULT 0,
              total_functions INTEGER NOT NULL DEFAULT 0,
              total_files INTEGER,
              cycle_count INTEGER NOT NULL DEFAULT 0,
              hotspot_count INTEGER NOT NULL DEFAULT 0,
              dead_code_count INTEGER NOT NULL DEFAULT 0,
              avg_coupling REAL,
              propagation_cost REAL,
              percentile REAL,
              included_modules INTEGER,
              excluded_modules INTEGER
            );

            CREATE TABLE IF NOT EXISTS feedback (
              id TEXT PRIMARY KEY,
              project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
              created_at TEXT NOT NULL,
              app_version TEXT NOT NULL,
              source TEXT NOT NULL DEFAULT 'cli',
              text TEXT NOT NULL
            );
            "#,
        )?;

        for (name, ddl) in [
            ("ai_summary_json_path", "TEXT"),
            ("ai_summary_md_path", "TEXT"),
            ("git_branch", "TEXT"),
            ("scan_languages", "TEXT"),
            ("git_commit", "TEXT"),
            ("git_dirty", "INTEGER"),
            ("scan_trigger", "TEXT NOT NULL DEFAULT 'desktop'"),
            ("requested_by", "TEXT"),
        ] {
            ensure_column(&conn, "scan_runs", name, ddl)?;
        }
        for (name, ddl) in [
            ("total_files", "INTEGER"),
            ("hotspot_count", "INTEGER NOT NULL DEFAULT 0"),
            ("avg_coupling", "REAL"),
            ("propagation_cost", "REAL"),
            ("percentile", "REAL"),
            ("included_modules", "INTEGER"),
            ("excluded_modules", "INTEGER"),
        ] {
            ensure_column(&conn, "metric_snapshots", name, ddl)?;
        }
        Ok(())
    }

    /// Append a free-form feedback row. Local-only by design — the row
    /// never leaves the user's machine and is only meant to be drained
    /// by `gridseak feedback --export` (future) when the user opts in.
    /// Returns the generated row id so callers can echo it for trust.
    pub fn record_feedback(
        &self,
        project_id: Option<&str>,
        text: &str,
        source: &str,
        app_version: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO feedback (id, project_id, created_at, app_version, source, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, project_id, now, app_version, source, text],
        )?;
        Ok(id)
    }

    /// Return every feedback row newest-first. Used by the future
    /// `gridseak feedback --export` flow; today exposed mostly so the
    /// CLI smoke test can prove the row landed.
    pub fn list_feedback(&self) -> Result<Vec<FeedbackDto>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, created_at, app_version, source, text
             FROM feedback
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FeedbackDto {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    created_at: row.get(2)?,
                    app_version: row.get(3)?,
                    source: row.get(4)?,
                    text: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn create_project_for_folder(&self, folder: &Path) -> Result<ProjectDto> {
        let canonical = folder
            .canonicalize()
            .unwrap_or_else(|_| folder.to_path_buf())
            .display()
            .to_string();
        let conn = self.conn()?;

        if let Some(existing_id) = conn
            .query_row(
                "SELECT project_id FROM project_roots WHERE path = ?1",
                params![canonical],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return self.get_project(&existing_id);
        }

        let now = Utc::now().to_rfc3339();
        let project_id = Uuid::new_v4().to_string();
        let root_id = Uuid::new_v4().to_string();
        let display_name = folder
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled project")
            .to_string();

        conn.execute(
            "INSERT INTO projects (id, display_name, created_at, updated_at, storage_mode, sync_status)
             VALUES (?1, ?2, ?3, ?4, 'local_only', 'local_only')",
            params![project_id, display_name, now, now],
        )?;
        conn.execute(
            "INSERT INTO project_roots (id, project_id, path, kind, added_at)
             VALUES (?1, ?2, ?3, 'primary', ?4)",
            params![root_id, project_id, canonical, now],
        )?;

        self.get_project(&project_id)
    }

    /// Lenient project resolution for MCP and other "I just want
    /// the project I'm working on right now" callers.
    ///
    /// Behaves like [`Self::resolve_project`] for any reference the
    /// user typed deliberately (a name, an absolute path, a UUID).
    /// For implicit references — empty string, `null` shaped as an
    /// empty string by serde, `.`, `./` — it tries the current
    /// working directory first and then falls back to the most
    /// recently completed scan in the store. This is what we want
    /// from the MCP layer: an agent that hands us `project: "."`
    /// from a workspace that hasn't been registered should NOT get
    /// a `project not found` error if there's any local scan we
    /// can reasonably attribute the call to.
    ///
    /// Errors carry actionable guidance (the lenient layer is what
    /// agents and humans see most often), so the surface should
    /// suggest a concrete next step — typically `gridseak scan .`
    /// — rather than just "not found."
    pub fn resolve_project_lenient(&self, reference: &str) -> Result<ProjectDto> {
        let trimmed = reference.trim();
        let is_implicit = trimmed.is_empty() || trimmed == "." || trimmed == "./";

        if !is_implicit {
            return self.resolve_project(reference);
        }

        // Explicit "current location" intent. Try cwd first.
        if let Ok(project) = self.resolve_project(trimmed) {
            return Ok(project);
        }

        // Cwd had no project. Fall back to the most recently scanned
        // project in the store so the agent can answer the user's
        // question instead of bouncing them with `project not found`.
        if let Some(project) = self.latest_scanned_project()? {
            return Ok(project);
        }

        anyhow::bail!(
            "no GridSeak project at the current directory and no scans found in the local store. \
             Run `gridseak scan .` from the project you want to analyse, then retry."
        )
    }

    /// Project whose most recently completed scan is newest across
    /// the whole store. Used as the lenient fallback when the
    /// caller's cwd has no project record. `Ok(None)` if the store
    /// has no scans at all (vs. `Err` for I/O / SQL errors).
    pub fn latest_scanned_project(&self) -> Result<Option<ProjectDto>> {
        let conn = self.conn()?;
        let id: Option<String> = conn
            .query_row(
                "SELECT p.id
                 FROM projects p
                 INNER JOIN scan_runs s ON s.project_id = p.id
                 WHERE s.completed_at IS NOT NULL
                 ORDER BY s.completed_at DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => Ok(Some(self.get_project(&id)?)),
            None => Ok(None),
        }
    }

    pub fn resolve_project(&self, reference: &str) -> Result<ProjectDto> {
        let reference = if reference.trim().is_empty() {
            "."
        } else {
            reference.trim()
        };
        if let Ok(project) = self.get_project(reference) {
            return Ok(project);
        }

        let canonical = Path::new(reference)
            .canonicalize()
            .ok()
            .map(|p| p.display().to_string());
        let conn = self.conn()?;
        let mut exact = conn.prepare(
            "SELECT p.id
             FROM projects p
             LEFT JOIN project_roots r ON r.project_id = p.id
             WHERE p.display_name = ?1 OR r.path = ?1 OR r.path = ?2
             ORDER BY p.updated_at DESC
             LIMIT 1",
        )?;
        if let Some(id) = exact
            .query_row(
                params![reference, canonical.as_deref().unwrap_or(reference)],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return self.get_project(&id);
        }

        let needle = format!("%{}%", reference.to_lowercase());
        let mut partial = conn.prepare(
            "SELECT DISTINCT p.id, p.display_name
             FROM projects p
             LEFT JOIN project_roots r ON r.project_id = p.id
             WHERE lower(p.display_name) LIKE ?1 OR lower(r.path) LIKE ?1
             ORDER BY p.updated_at DESC
             LIMIT 6",
        )?;
        let matches = partial
            .query_map(params![needle], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if matches.len() == 1 {
            return self.get_project(&matches[0].0);
        }
        if matches.len() > 1 {
            let names = matches
                .into_iter()
                .map(|(id, name)| format!("{name} ({id})"))
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("project reference is ambiguous: {reference}. Matches: {names}");
        }
        anyhow::bail!("project not found: {reference}");
    }

    pub fn search_projects(&self, query: &str, limit: usize) -> Result<Vec<ProjectDto>> {
        let conn = self.conn()?;
        let needle = format!("%{}%", query.to_lowercase());
        let mut stmt = conn.prepare(
            "SELECT DISTINCT p.id
             FROM projects p
             LEFT JOIN project_roots r ON r.project_id = p.id
             WHERE lower(p.id) LIKE ?1 OR lower(p.display_name) LIKE ?1 OR lower(r.path) LIKE ?1
             ORDER BY p.updated_at DESC
             LIMIT ?2",
        )?;
        let ids = stmt
            .query_map(params![needle, limit as i64], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.get_project(&id)).collect()
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectDto>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM projects
             ORDER BY COALESCE(
               (SELECT COALESCE(completed_at, started_at) FROM scan_runs WHERE id = projects.last_scan_run_id),
               updated_at
             ) DESC",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.get_project(&id)).collect()
    }

    pub fn get_project(&self, project_id: &str) -> Result<ProjectDto> {
        let conn = self.conn()?;
        let base = conn.query_row(
            "SELECT id, display_name, created_at, updated_at, storage_mode, sync_status, last_scan_run_id
             FROM projects WHERE id = ?1",
            params![project_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )?;

        let roots = self.roots_for_project(&conn, project_id)?;
        let latest_scan = base
            .6
            .as_deref()
            .and_then(|id| self.scan_run(id).ok().flatten());
        let latest_metrics = latest_scan.as_ref().and_then(|s| s.metrics.clone());
        let scan_count = conn.query_row(
            "SELECT COUNT(*) FROM scan_runs WHERE project_id = ?1",
            params![project_id],
            |row| row.get::<_, i64>(0),
        )? as u32;

        Ok(ProjectDto {
            id: base.0,
            display_name: base.1,
            created_at: base.2,
            updated_at: base.3,
            storage_mode: base.4,
            sync_status: base.5,
            last_scan_run_id: base.6,
            roots,
            latest_scan,
            latest_metrics,
            scan_count,
        })
    }

    pub fn project_detail(&self, project_id: &str) -> Result<ProjectDetailDto> {
        Ok(ProjectDetailDto {
            project: self.get_project(project_id)?,
            scans: self.list_scan_runs(project_id)?,
        })
    }

    pub fn list_scan_runs(&self, project_id: &str) -> Result<Vec<ScanRunDto>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id FROM scan_runs WHERE project_id = ?1 ORDER BY started_at DESC")?;
        let ids = stmt
            .query_map(params![project_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .filter_map(|id| self.scan_run(&id).transpose())
            .collect()
    }

    pub fn begin_scan(&self, record: BeginScanRecord) -> Result<()> {
        let conn = self.conn()?;
        let scan_id = record.scan_id.to_string();
        conn.execute(
            "INSERT OR REPLACE INTO scan_runs
             (id, project_id, root_id, started_at, status, app_version, engine_version, sync_status,
              primary_language, scan_languages, git_branch, git_commit, git_dirty, scan_trigger, requested_by)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, 'local_only', ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                scan_id,
                record.project_id,
                record.root_id,
                record.started_at.to_rfc3339(),
                record.app_version,
                record.engine_version,
                record.primary_language,
                serde_json::to_string(&record.scan_languages)?,
                record.git.branch,
                record.git.commit,
                record.git.dirty.map(|v| if v { 1 } else { 0 }),
                record.scan_trigger,
                record.requested_by,
            ],
        )?;
        conn.execute(
            "UPDATE projects SET updated_at = ?1, last_scan_run_id = ?2 WHERE id = ?3",
            params![Utc::now().to_rfc3339(), scan_id, record.project_id],
        )?;
        Ok(())
    }

    /// Mark a scan as completed.
    ///
    /// `primary_language_override` is the analyzer's canonical
    /// File-node-majority answer (A3). When `Some`, it replaces the
    /// pre-scan guess in `scan_runs.primary_language` so historical
    /// trends and the desktop's project list see the right label for
    /// polyglot repos. When `None`, the pre-scan value (set in
    /// `begin_scan` from CLI/desktop language detection) is kept —
    /// the analyzer either couldn't decide (empty graph) or the
    /// caller doesn't have the report (legacy path).
    pub fn complete_scan(
        &self,
        scan_id: Uuid,
        project_id: &str,
        report: &serde_json::Value,
        scratch_report_path: &Path,
        scratch_graph_path: &Path,
        primary_language_override: Option<&str>,
    ) -> Result<()> {
        let scan_id_s = scan_id.to_string();
        let report_path = self.reports_dir.join(format!("{scan_id_s}.report.json"));
        let graph_path = self.graphs_dir.join(format!("{scan_id_s}.sqlite"));
        let summary_json_path = self
            .reports_dir
            .join(format!("{scan_id_s}.ai-summary.json"));
        let summary_md_path = self.reports_dir.join(format!("{scan_id_s}.ai-summary.md"));

        std::fs::copy(scratch_report_path, &report_path).with_context(|| {
            format!(
                "copy durable report {} -> {}",
                scratch_report_path.display(),
                report_path.display()
            )
        })?;
        // Flush any pending WAL frames from the scratch graph DB into the
        // main `.sqlite` file *before* we copy it to the durable graphs
        // directory. The scratch DB is opened in `journal_mode=WAL`
        // (see `graphengine-parsing/src/infrastructure/storage/schema.rs`),
        // so writes performed by long-lived helpers like
        // `graphengine_analysis::health::persist_project_language` land
        // in `<scratch_graph>-wal` rather than the main file. `fs::copy`
        // takes only the main file, so without this checkpoint the
        // durable graph silently loses those frames.
        //
        // This is a defense-in-depth boundary: each writer (e.g.
        // `persist_project_language`) already checkpoints at its own
        // exit, but new writers should not have to remember to. The
        // boundary "we are about to copy this DB" is the canonical
        // moment to guarantee WAL is flushed, regardless of how many
        // writers touched it upstream.
        //
        // Failures here are non-fatal — a read-only mount, exotic
        // locking, or a missing file all mean "no checkpoint to do" or
        // "we'll catch it later"; the copy itself is the durability
        // requirement and we want to keep going so the user gets a
        // scan record. We log at WARN so the issue is visible in the
        // diagnostic log without being silent.
        if let Err(err) = checkpoint_wal(scratch_graph_path) {
            tracing::warn!(
                scratch_graph = %scratch_graph_path.display(),
                error = %err,
                "wal_checkpoint on scratch graph DB failed; copy will still proceed but writes from the analyzer's write-back path may not appear in the durable copy"
            );
        }
        std::fs::copy(scratch_graph_path, &graph_path).with_context(|| {
            format!(
                "copy durable graph {} -> {}",
                scratch_graph_path.display(),
                graph_path.display()
            )
        })?;

        let metrics = extract_metrics(report);
        let completed_at = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        let started_at: String = conn.query_row(
            "SELECT started_at FROM scan_runs WHERE id = ?1",
            params![scan_id_s],
            |row| row.get(0),
        )?;
        let duration_ms = DateTime::parse_from_rfc3339(&started_at)
            .ok()
            .map(|started| {
                Utc::now()
                    .signed_duration_since(started.with_timezone(&Utc))
                    .num_milliseconds()
            });

        let summary = self.build_ai_summary(
            project_id,
            &scan_id_s,
            report,
            &metrics,
            &report_path,
            &graph_path,
        )?;
        std::fs::write(&summary_json_path, serde_json::to_string_pretty(&summary)?)?;
        std::fs::write(&summary_md_path, ai_summary_markdown(&summary))?;

        conn.execute(
            "UPDATE scan_runs
             SET completed_at = ?1, status = 'ready', report_path = ?2, graph_artifact_path = ?3,
                 ai_summary_json_path = ?4, ai_summary_md_path = ?5, duration_ms = ?6
             WHERE id = ?7",
            params![
                completed_at,
                report_path.display().to_string(),
                graph_path.display().to_string(),
                summary_json_path.display().to_string(),
                summary_md_path.display().to_string(),
                duration_ms,
                scan_id_s,
            ],
        )?;
        // A3: overwrite the pre-scan language guess with the analyzer's
        // File-node-majority answer when the caller has it.
        if let Some(lang) = primary_language_override {
            conn.execute(
                "UPDATE scan_runs SET primary_language = ?1 WHERE id = ?2",
                params![lang, scan_id_s],
            )?;
        }
        upsert_metrics(&conn, &scan_id_s, &metrics)?;
        conn.execute(
            "UPDATE projects SET updated_at = ?1, last_scan_run_id = ?2 WHERE id = ?3",
            params![Utc::now().to_rfc3339(), scan_id_s, project_id],
        )?;
        Ok(())
    }

    pub fn fail_scan(&self, scan_id: Uuid, error: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE scan_runs SET completed_at = ?1, status = 'failed' WHERE id = ?2",
            params![Utc::now().to_rfc3339(), scan_id.to_string()],
        )?;
        tracing::warn!(scan_id = %scan_id, error, "project scan failed");
        Ok(())
    }

    pub fn load_report(&self, scan_id: &str) -> Result<serde_json::Value> {
        let conn = self.conn()?;
        let report_path: String = conn.query_row(
            "SELECT report_path FROM scan_runs WHERE id = ?1 AND report_path IS NOT NULL",
            params![scan_id],
            |row| row.get(0),
        )?;
        let raw = std::fs::read_to_string(&report_path)
            .with_context(|| format!("read report {}", report_path))?;
        serde_json::from_str(&raw).context("parse persisted report json")
    }

    pub fn load_ai_summary(&self, scan_id: &str) -> Result<serde_json::Value> {
        let conn = self.conn()?;
        let summary_path: String = conn.query_row(
            "SELECT ai_summary_json_path FROM scan_runs WHERE id = ?1 AND ai_summary_json_path IS NOT NULL",
            params![scan_id],
            |row| row.get(0),
        )?;
        let raw = std::fs::read_to_string(&summary_path)
            .with_context(|| format!("read AI summary {}", summary_path))?;
        serde_json::from_str(&raw).context("parse AI summary json")
    }

    fn roots_for_project(
        &self,
        conn: &Connection,
        project_id: &str,
    ) -> Result<Vec<ProjectRootDto>> {
        let mut stmt = conn.prepare(
            "SELECT id, project_id, path, kind, added_at FROM project_roots
             WHERE project_id = ?1 ORDER BY added_at ASC",
        )?;
        let roots = stmt
            .query_map(params![project_id], |row| {
                Ok(ProjectRootDto {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    path: row.get(2)?,
                    kind: row.get(3)?,
                    added_at: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(roots)
    }

    fn scan_run(&self, scan_id: &str) -> Result<Option<ScanRunDto>> {
        let conn = self.conn()?;
        let scan = conn
            .query_row(
                "SELECT id, project_id, root_id, started_at, completed_at, status, app_version,
                        engine_version, report_path, graph_artifact_path, ai_summary_json_path,
                        ai_summary_md_path, sync_status, duration_ms, primary_language, scan_languages, git_branch,
                        git_commit, git_dirty, scan_trigger, requested_by
                 FROM scan_runs WHERE id = ?1",
                params![scan_id],
                |row| {
                    let scan_languages_json: Option<String> = row.get(15)?;
                    let git_dirty: Option<i64> = row.get(18)?;
                    let scan_languages = scan_languages_json
                        .as_deref()
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or_default();
                    Ok(ScanRunDto {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        root_id: row.get(2)?,
                        started_at: row.get(3)?,
                        completed_at: row.get(4)?,
                        status: row.get(5)?,
                        app_version: row.get(6)?,
                        engine_version: row.get(7)?,
                        report_path: row.get(8)?,
                        graph_artifact_path: row.get(9)?,
                        ai_summary_json_path: row.get(10)?,
                        ai_summary_md_path: row.get(11)?,
                        sync_status: row.get(12)?,
                        duration_ms: row.get(13)?,
                        primary_language: row.get(14)?,
                        scan_languages,
                        git_branch: row.get(16)?,
                        git_commit: row.get(17)?,
                        git_dirty: git_dirty.map(|v| v != 0),
                        scan_trigger: row.get(19)?,
                        requested_by: row.get(20)?,
                        metrics: None,
                    })
                },
            )
            .optional()?;
        Ok(scan.map(|mut s| {
            s.metrics = self.metrics_for_scan(&conn, &s.id).ok().flatten();
            s
        }))
    }

    fn metrics_for_scan(
        &self,
        conn: &Connection,
        scan_id: &str,
    ) -> Result<Option<MetricSnapshotDto>> {
        conn.query_row(
            "SELECT health_score, finding_count, critical_count, high_count, total_nodes,
                    total_edges, total_modules, total_functions, total_files, cycle_count,
                    hotspot_count, dead_code_count, avg_coupling, propagation_cost, percentile,
                    included_modules, excluded_modules
             FROM metric_snapshots WHERE scan_run_id = ?1",
            params![scan_id],
            |row| {
                Ok(MetricSnapshotDto {
                    health_score: row.get(0)?,
                    finding_count: row.get(1)?,
                    critical_count: row.get(2)?,
                    high_count: row.get(3)?,
                    total_nodes: row.get(4)?,
                    total_edges: row.get(5)?,
                    total_modules: row.get(6)?,
                    total_functions: row.get(7)?,
                    total_files: row.get(8)?,
                    cycle_count: row.get(9)?,
                    hotspot_count: row.get(10)?,
                    dead_code_count: row.get(11)?,
                    avg_coupling: row.get(12)?,
                    propagation_cost: row.get(13)?,
                    percentile: row.get(14)?,
                    included_modules: row.get(15)?,
                    excluded_modules: row.get(16)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    fn build_ai_summary(
        &self,
        project_id: &str,
        scan_id: &str,
        report: &serde_json::Value,
        metrics: &MetricSnapshotDto,
        report_path: &Path,
        graph_path: &Path,
    ) -> Result<serde_json::Value> {
        let project = self.get_project(project_id)?;
        let latest_scan = self.scan_run(scan_id)?.context("scan row not found")?;
        let prior_scan = self
            .list_scan_runs(project_id)?
            .into_iter()
            .filter(|scan| scan.id != scan_id && scan.status == "ready")
            .find(|scan| scan.metrics.is_some());
        let deltas = prior_scan
            .as_ref()
            .and_then(|scan| scan.metrics.as_ref())
            .map(|prev| metric_deltas(prev, metrics));

        let priorities = serde_json::from_value::<HealthReport>(report.clone())
            .ok()
            .map(|health| priority::compute_priorities(&health, priority::DEFAULT_TOP_N))
            .unwrap_or_default();

        Ok(serde_json::json!({
            "schema": "gridseak.ai_summary.v1",
            "project": {
                "id": project.id,
                "display_name": project.display_name,
                "root_path": project.roots.first().map(|r| r.path.clone()),
                "storage_mode": project.storage_mode,
                "sync_status": project.sync_status,
            },
            "scan": {
                "id": scan_id,
                "started_at": latest_scan.started_at,
                "completed_at": latest_scan.completed_at,
                "branch": latest_scan.git_branch,
                "commit": latest_scan.git_commit,
                "dirty": latest_scan.git_dirty,
                "trigger": latest_scan.scan_trigger,
                "requested_by": latest_scan.requested_by,
                "primary_language": latest_scan.primary_language,
                "languages": latest_scan.scan_languages,
                "duration_ms": latest_scan.duration_ms,
            },
            "metrics": metrics,
            "deltas_from_previous_ready_scan": deltas,
            "top_recommendations": priorities,
            "artifacts": {
                "report_path": report_path.display().to_string(),
                "graph_artifact_path": graph_path.display().to_string(),
            }
        }))
    }
}

/// Open `path` in read-write mode and run `PRAGMA wal_checkpoint(TRUNCATE)`
/// so any pending WAL frames are flushed into the main `.sqlite` file
/// and the WAL is reset to zero length.
///
/// This exists because every consumer of the graph DB that performs
/// `fs::copy(scratch, durable)` needs the data to be in the main file,
/// not in the sidecar `-wal` / `-shm` files. Writers like
/// `graphengine_analysis::health::persist_project_language` already
/// checkpoint at their own exits, but the boundary that genuinely
/// requires it is the copy itself — see the call site in
/// `complete_scan` for the full rationale.
///
/// Returns a `rusqlite::Result` so the caller can choose its own
/// failure-tolerance policy. `complete_scan` logs and continues; other
/// callers may want to surface the failure differently.
fn checkpoint_wal(db_path: &Path) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // `wal_checkpoint(TRUNCATE)` is the strongest variant; it
    // returns three integers (busy, log, checkpointed) that we
    // do not need beyond surfacing as an error if the statement
    // itself fails. A return of (0, -1, -1) is the legal no-op
    // case for a DB that has no WAL frames or was never opened in
    // WAL mode — also fine.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

fn upsert_metrics(conn: &Connection, scan_id: &str, m: &MetricSnapshotDto) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO metric_snapshots
         (scan_run_id, health_score, finding_count, critical_count, high_count,
          total_nodes, total_edges, total_modules, total_functions, total_files, cycle_count,
          hotspot_count, dead_code_count, avg_coupling, propagation_cost, percentile,
          included_modules, excluded_modules)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            scan_id,
            m.health_score,
            m.finding_count,
            m.critical_count,
            m.high_count,
            m.total_nodes,
            m.total_edges,
            m.total_modules,
            m.total_functions,
            m.total_files,
            m.cycle_count,
            m.hotspot_count,
            m.dead_code_count,
            m.avg_coupling,
            m.propagation_cost,
            m.percentile,
            m.included_modules,
            m.excluded_modules,
        ],
    )?;
    Ok(())
}

fn extract_metrics(report: &serde_json::Value) -> MetricSnapshotDto {
    let findings = report
        .get("findings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let severity_count = |severity: &str| -> i64 {
        findings
            .iter()
            .filter(|f| f.get("severity").and_then(|s| s.as_str()) == Some(severity))
            .count() as i64
    };

    MetricSnapshotDto {
        health_score: report.get("health_score").and_then(|v| v.as_f64()),
        finding_count: findings.len() as i64,
        critical_count: severity_count("critical"),
        high_count: severity_count("high"),
        total_nodes: i64_at(report, &["summary", "total_nodes"]),
        total_edges: i64_at(report, &["summary", "total_edges"]),
        total_modules: i64_at(report, &["summary", "total_modules"]),
        total_functions: i64_at(report, &["summary", "total_functions"]),
        total_files: optional_i64_at(report, &["summary", "total_files"]),
        cycle_count: i64_at(report, &["metrics", "cycles", "count"])
            .max(i64_at(report, &["summary", "cycles_found"])),
        hotspot_count: i64_at(report, &["metrics", "hotspot_concentration", "count"])
            .max(i64_at(report, &["summary", "hotspot_count"])),
        dead_code_count: i64_at(report, &["metrics", "dead_code", "count"]),
        avg_coupling: optional_f64_at(report, &["metrics", "coupling", "avg_coupling"])
            .or_else(|| optional_f64_at(report, &["summary", "avg_module_coupling"])),
        propagation_cost: optional_f64_at(report, &["metrics", "propagation_cost", "value"])
            .or_else(|| optional_f64_at(report, &["summary", "propagation_cost"])),
        percentile: optional_f64_at(report, &["percentiles", "composite_percentile"]),
        included_modules: optional_i64_at(report, &["summary", "included_modules"]),
        excluded_modules: optional_i64_at(report, &["summary", "excluded_modules"]),
    }
}

fn ensure_column(conn: &Connection, table: &str, name: &str, ddl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|column| column == name);
    if !exists {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {name} {ddl}"), [])?;
    }
    Ok(())
}

fn metric_deltas(prev: &MetricSnapshotDto, next: &MetricSnapshotDto) -> serde_json::Value {
    let delta = |a: Option<f64>, b: Option<f64>| a.zip(b).map(|(a, b)| b - a);
    serde_json::json!({
        "health_score": delta(prev.health_score, next.health_score),
        "finding_count": next.finding_count - prev.finding_count,
        "critical_count": next.critical_count - prev.critical_count,
        "high_count": next.high_count - prev.high_count,
        "cycle_count": next.cycle_count - prev.cycle_count,
        "hotspot_count": next.hotspot_count - prev.hotspot_count,
        "dead_code_count": next.dead_code_count - prev.dead_code_count,
        "avg_coupling": delta(prev.avg_coupling, next.avg_coupling),
        "propagation_cost": delta(prev.propagation_cost, next.propagation_cost),
    })
}

fn ai_summary_markdown(summary: &serde_json::Value) -> String {
    let project = &summary["project"];
    let scan = &summary["scan"];
    let metrics = &summary["metrics"];
    let mut out = String::new();
    out.push_str("# GridSeak Scan Summary\n\n");
    out.push_str(&format!(
        "- Project: {}\n- Root: {}\n- Scan: {}\n- Branch: {}\n- Commit: {}\n- Dirty: {}\n- Completed: {}\n\n",
        project["display_name"].as_str().unwrap_or("unknown"),
        project["root_path"].as_str().unwrap_or("unknown"),
        scan["id"].as_str().unwrap_or("unknown"),
        scan["branch"].as_str().unwrap_or("unknown"),
        scan["commit"].as_str().unwrap_or("unknown"),
        scan["dirty"].as_bool().map(|v| v.to_string()).unwrap_or_else(|| "unknown".into()),
        scan["completed_at"].as_str().unwrap_or("unknown"),
    ));
    out.push_str("## Metrics\n\n");
    out.push_str(&format!(
        "- Health score: {}\n- Findings: {}\n- Critical: {}\n- High: {}\n- Cycles: {}\n- Hotspots: {}\n- Dead code: {}\n- Functions: {}\n- Modules: {}\n\n",
        display_json(&metrics["health_score"]),
        display_json(&metrics["finding_count"]),
        display_json(&metrics["critical_count"]),
        display_json(&metrics["high_count"]),
        display_json(&metrics["cycle_count"]),
        display_json(&metrics["hotspot_count"]),
        display_json(&metrics["dead_code_count"]),
        display_json(&metrics["total_functions"]),
        display_json(&metrics["total_modules"]),
    ));
    out.push_str("## Top Recommendations\n\n");
    if let Some(items) = summary["top_recommendations"].as_array() {
        for item in items {
            out.push_str(&format!(
                "{}. {} — {}\n   - Action: {}\n",
                item["rank"].as_u64().unwrap_or(0),
                item["target"].as_str().unwrap_or("unknown"),
                item["risk_narrative"]
                    .as_str()
                    .unwrap_or("No narrative available."),
                item["suggested_action"]
                    .as_str()
                    .unwrap_or("Review the finding."),
            ));
        }
    }
    out
}

fn display_json(value: &serde_json::Value) -> String {
    if value.is_null() {
        "unknown".to_string()
    } else if let Some(s) = value.as_str() {
        s.to_string()
    } else {
        value.to_string()
    }
}

fn git_output<const N: usize>(path: &Path, args: [&str; N]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("spawn git")?;
    if !output.status.success() {
        anyhow::bail!("git exited with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn i64_at(value: &serde_json::Value, path: &[&str]) -> i64 {
    optional_i64_at(value, path).unwrap_or(0)
}

fn optional_i64_at(value: &serde_json::Value, path: &[&str]) -> Option<i64> {
    path.iter()
        .try_fold(value, |acc, key| acc.get(*key))
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|n| n as i64)))
}

fn optional_f64_at(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    path.iter()
        .try_fold(value, |acc, key| acc.get(*key))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_scan_round_trip_persists_metrics_git_and_ai_summary() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo)?;
        let store = ProjectStore::open(
            temp.path().join("projects.sqlite"),
            temp.path().join("reports"),
            temp.path().join("graphs"),
        )?;

        let project = store.create_project_for_folder(&repo)?;
        assert_eq!(store.resolve_project(&project.id)?.id, project.id);
        assert_eq!(store.resolve_project(&project.display_name)?.id, project.id);
        assert_eq!(
            store.resolve_project(repo.to_str().unwrap())?.id,
            project.id
        );

        let scan_id = Uuid::new_v4();
        store.begin_scan(BeginScanRecord {
            scan_id,
            project_id: project.id.clone(),
            root_id: project.roots.first().map(|r| r.id.clone()),
            started_at: Utc::now(),
            app_version: "test-app".to_string(),
            engine_version: "test-engine".to_string(),
            primary_language: Some("rust".to_string()),
            scan_languages: vec!["rust".to_string()],
            git: GitContext {
                branch: Some("main".to_string()),
                commit: Some("abc123".to_string()),
                dirty: Some(false),
            },
            scan_trigger: "test".to_string(),
            requested_by: Some("agent".to_string()),
        })?;

        let report_path = temp.path().join("scratch.report.json");
        let graph_path = temp.path().join("scratch.sqlite");
        let report = serde_json::json!({
            "health_score": 88,
            "findings": [],
            "summary": {
                "total_nodes": 10,
                "total_edges": 9,
                "total_modules": 2,
                "total_functions": 7,
                "cycles_found": 1,
                "hotspot_count": 3,
                "avg_module_coupling": 0.25
            },
            "metrics": {
                "cycles": { "count": 1 },
                "hotspot_concentration": { "count": 3 },
                "dead_code": { "count": 2 },
                "coupling": { "avg_coupling": 0.25 }
            },
            "percentiles": { "composite_percentile": 92 }
        });
        std::fs::write(&report_path, serde_json::to_string(&report)?)?;
        std::fs::write(&graph_path, "sqlite-placeholder")?;

        store.complete_scan(
            scan_id,
            &project.id,
            &report,
            &report_path,
            &graph_path,
            None,
        )?;
        let detail = store.project_detail(&project.id)?;
        let scan = detail.scans.first().context("missing scan")?;
        assert_eq!(scan.status, "ready");
        assert_eq!(scan.git_branch.as_deref(), Some("main"));
        assert_eq!(scan.scan_trigger, "test");
        assert!(scan.ai_summary_json_path.is_some());
        assert!(scan.ai_summary_md_path.is_some());
        let metrics = scan.metrics.as_ref().context("missing metrics")?;
        assert_eq!(metrics.health_score, Some(88.0));
        assert_eq!(metrics.cycle_count, 1);
        assert_eq!(metrics.hotspot_count, 3);
        assert_eq!(metrics.dead_code_count, 2);

        Ok(())
    }

    /// A3 — when `complete_scan` is called with an analyzer-side
    /// language override, it replaces the pre-scan
    /// `scan_runs.primary_language` value (which `begin_scan` sets
    /// from the CLI/desktop language picker, often biased toward the
    /// wrong language in polyglot repos).
    #[test]
    fn complete_scan_applies_primary_language_override() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo)?;
        let store = ProjectStore::open(
            temp.path().join("projects.sqlite"),
            temp.path().join("reports"),
            temp.path().join("graphs"),
        )?;
        let project = store.create_project_for_folder(&repo)?;
        let scan_id = Uuid::new_v4();
        store.begin_scan(BeginScanRecord {
            scan_id,
            project_id: project.id.clone(),
            root_id: project.roots.first().map(|r| r.id.clone()),
            started_at: Utc::now(),
            app_version: "test-app".to_string(),
            engine_version: "test-engine".to_string(),
            // Pre-scan guess intentionally wrong for a polyglot repo
            // — exactly the failure mode A3 is fixing.
            primary_language: Some("javascript".to_string()),
            scan_languages: vec!["javascript".to_string(), "rust".to_string()],
            git: GitContext {
                branch: Some("main".into()),
                commit: None,
                dirty: Some(false),
            },
            scan_trigger: "test".into(),
            requested_by: Some("agent".into()),
        })?;

        let report_path = temp.path().join("scratch.report.json");
        let graph_path = temp.path().join("scratch.sqlite");
        let report = serde_json::json!({
            "health_score": 70,
            "findings": [],
            "summary": {},
            "metrics": {
                "cycles": { "count": 0 },
                "hotspot_concentration": { "count": 0 },
                "dead_code": { "count": 0 },
                "coupling": { "avg_coupling": 0.0 }
            },
            "primary_language": "rust"
        });
        std::fs::write(&report_path, serde_json::to_string(&report)?)?;
        std::fs::write(&graph_path, "sqlite-placeholder")?;

        store.complete_scan(
            scan_id,
            &project.id,
            &report,
            &report_path,
            &graph_path,
            Some("rust"),
        )?;
        let detail = store.project_detail(&project.id)?;
        let scan = detail.scans.first().context("missing scan")?;
        assert_eq!(
            scan.primary_language.as_deref(),
            Some("rust"),
            "analyzer-side language override must replace the pre-scan guess"
        );
        Ok(())
    }

    /// Reproduce the original A3 corruption case in isolation: a WAL-mode
    /// scratch DB receives a write through a connection that never
    /// explicitly checkpoints. Without the defensive `checkpoint_wal`
    /// inside `complete_scan`, `std::fs::copy` of the main `.sqlite`
    /// file would silently drop the new row — exactly the failure mode
    /// `persist_project_language` originally hit in the live pilots.
    ///
    /// This test pins the contract so any future code path that opens
    /// the scratch DB read-write — for any reason — has its writes
    /// guaranteed to land in the durable copy without needing to
    /// remember its own checkpoint.
    #[test]
    fn complete_scan_flushes_uncheckpointed_wal_writes_into_durable_copy() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo)?;
        let store = ProjectStore::open(
            temp.path().join("projects.sqlite"),
            temp.path().join("reports"),
            temp.path().join("graphs"),
        )?;
        let project = store.create_project_for_folder(&repo)?;
        let scan_id = Uuid::new_v4();
        store.begin_scan(BeginScanRecord {
            scan_id,
            project_id: project.id.clone(),
            root_id: project.roots.first().map(|r| r.id.clone()),
            started_at: Utc::now(),
            app_version: "test-app".to_string(),
            engine_version: "test-engine".to_string(),
            primary_language: Some("rust".to_string()),
            scan_languages: vec!["rust".to_string()],
            git: GitContext {
                branch: Some("main".into()),
                commit: None,
                dirty: Some(false),
            },
            scan_trigger: "test".into(),
            requested_by: Some("agent".into()),
        })?;

        let report_path = temp.path().join("scratch.report.json");
        let graph_path = temp.path().join("scratch.sqlite");
        let report = serde_json::json!({
            "health_score": 50,
            "findings": [],
            "summary": {},
            "metrics": {
                "cycles": { "count": 0 },
                "hotspot_concentration": { "count": 0 },
                "dead_code": { "count": 0 },
                "coupling": { "avg_coupling": 0.0 }
            }
        });
        std::fs::write(&report_path, serde_json::to_string(&report)?)?;

        // Build a real SQLite DB in WAL mode (the parser's default),
        // write a row through a connection, then deliberately leak the
        // connection so the OS's drop never gets a chance to flush the
        // WAL. This is the closest reproduction of the failure mode
        // `persist_project_language` originally hit: a long-lived
        // analyzer process writes the row, the connection is dropped
        // *during* the analyzer's own exit path, but the durable copy
        // happens later from a different process / scope.
        {
            let conn = Connection::open(&graph_path)?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.execute_batch(
                "CREATE TABLE marker (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO marker (key, value) VALUES ('wal_canary', 'survived');",
            )?;
            // Drop the connection WITHOUT calling
            // `wal_checkpoint(TRUNCATE)` — the entire failure mode this
            // test pins is that the write is in `<path>-wal` rather
            // than the main file at this exact moment.
            std::mem::forget(conn);
        }

        store.complete_scan(
            scan_id,
            &project.id,
            &report,
            &report_path,
            &graph_path,
            None,
        )?;

        let durable_graph = store.graphs_dir.join(format!("{}.sqlite", scan_id));
        assert!(durable_graph.exists(), "durable graph file missing");

        // Open the durable copy *without* attaching any sidecar WAL —
        // this matches what every downstream reader does (CLI
        // drilldowns, desktop viewer). If the checkpoint didn't run,
        // the `marker` row will be missing.
        let durable_conn = Connection::open_with_flags(
            &durable_graph,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let value: String = durable_conn.query_row(
            "SELECT value FROM marker WHERE key = 'wal_canary'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            value, "survived",
            "complete_scan must wal_checkpoint the scratch DB before copying so uncheckpointed writes survive"
        );
        Ok(())
    }

    /// Q5 regression: when the MCP agent passes an implicit
    /// reference (`""`, `"."`, `"./"`) and the current working
    /// directory has no registered project, lenient resolution
    /// must fall back to the most recently completed scan in the
    /// store rather than erroring with `project not found: "."`.
    #[test]
    fn lenient_resolve_falls_back_to_latest_scan_when_cwd_has_no_project() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("a_repo_we_will_scan");
        std::fs::create_dir_all(&repo)?;
        let store = ProjectStore::open(
            temp.path().join("projects.sqlite"),
            temp.path().join("reports"),
            temp.path().join("graphs"),
        )?;
        let project = store.create_project_for_folder(&repo)?;

        // Register a completed scan against this project so it qualifies
        // as the "latest" candidate.
        let scan_id = Uuid::new_v4();
        store.begin_scan(BeginScanRecord {
            scan_id,
            project_id: project.id.clone(),
            root_id: project.roots.first().map(|r| r.id.clone()),
            started_at: Utc::now(),
            app_version: "test-app".to_string(),
            engine_version: "test-engine".to_string(),
            primary_language: Some("rust".to_string()),
            scan_languages: vec!["rust".to_string()],
            git: GitContext {
                branch: Some("main".into()),
                commit: Some("abc".into()),
                dirty: Some(false),
            },
            scan_trigger: "test".into(),
            requested_by: Some("agent".into()),
        })?;
        let report_path = temp.path().join("scratch.report.json");
        let graph_path = temp.path().join("scratch.sqlite");
        let report = serde_json::json!({
            "health_score": 50,
            "findings": [],
            "summary": {
                "total_nodes": 1,
                "total_edges": 0,
                "total_modules": 1,
                "total_functions": 1,
                "cycles_found": 0,
                "hotspot_count": 0,
                "avg_module_coupling": 0.0
            },
            "metrics": {
                "cycles": { "count": 0 },
                "hotspot_concentration": { "count": 0 },
                "dead_code": { "count": 0 }
            }
        });
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
        std::fs::write(&graph_path, b"sqlite stub")?;
        store.complete_scan(
            scan_id,
            &project.id,
            &report,
            &report_path,
            &graph_path,
            None,
        )?;

        // Strict resolution on a reference that doesn't match anything
        // must still error (callers who typed a specific name didn't
        // ask for the latest scan).
        assert!(
            store.resolve_project("definitely-not-a-project").is_err(),
            "strict resolve_project must NOT silently fall back for explicit non-matching refs"
        );

        // But lenient resolution on an implicit reference MUST find
        // the project via the latest-scan fallback, even though the
        // ProjectStore can't see a project at "." (cwd is whatever
        // cargo test happens to run from, almost certainly not the
        // tempdir we just registered).
        for implicit_ref in ["", ".", "./", "  "] {
            let resolved = store.resolve_project_lenient(implicit_ref)?;
            assert_eq!(
                resolved.id, project.id,
                "lenient resolution of {implicit_ref:?} should yield the latest-scanned project"
            );
        }

        Ok(())
    }
}
