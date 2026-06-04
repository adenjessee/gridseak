use std::path::PathBuf;

use anyhow::{Context, Result};

/// Filesystem layout for the local-first GridSeak data + scratch space.
///
/// Two roots are involved, by deliberate platform convention:
///
/// - `data_dir` — durable user data (the projects DB, persisted reports,
///   persisted graphs). Lives under `Application Support` / `%APPDATA%` /
///   `$XDG_DATA_HOME`. Backed up by Time Machine etc.
/// - `cache_dir` — derived, regenerable scratch (per-scan SQLite shards
///   used during pipeline execution, intermediate stderr logs, parser
///   --clear targets). Lives under `Caches` / `%LOCALAPPDATA%` /
///   `$XDG_CACHE_HOME`. Specifically the kind of path the OS is allowed
///   to evict when low on disk.
///
/// Before this split, `scratch_dir` lived under `data_dir/cli-cache/scans`
/// — wrong category: ephemeral scan artefacts were sitting in the same
/// path Time Machine backs up nightly, and cleared scans never freed up
/// the durable backup. Moving them realigns the data with what each
/// path category actually means.
#[derive(Debug, Clone)]
pub struct LocalStorePaths {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub project_reports_dir: PathBuf,
    pub project_graphs_dir: PathBuf,
    pub scratch_dir: PathBuf,
    /// Root for persistent per-project parse SQLite DBs (S1-ε
    /// incremental scanning). Lives under `cache_dir` because the
    /// DB is fundamentally a cache — losing it triggers a slower
    /// full rescan on the next invocation, but is never a
    /// correctness regression (per-scan snapshots in
    /// [`Self::project_graphs_dir`] are the durable historical
    /// record). Per-project subdirectories created lazily by
    /// [`Self::parse_db_for_project`].
    pub parse_dbs_dir: PathBuf,
}

impl LocalStorePaths {
    pub fn resolve_default() -> Result<Self> {
        let data_dir = default_app_data_dir()?;
        let cache_dir = default_app_cache_dir()?;
        Self::from_dirs(data_dir, cache_dir)
    }

    /// Backwards-compat helper for callers that only know about `data_dir`
    /// (e.g. `--data-dir <path>` CLI flag). Derives the cache dir from
    /// the platform default and proceeds. Prefer [`Self::from_dirs`] when
    /// the consumer can supply both.
    pub fn from_data_dir(data_dir: PathBuf) -> Result<Self> {
        let cache_dir = default_app_cache_dir()?;
        Self::from_dirs(data_dir, cache_dir)
    }

    pub fn from_dirs(data_dir: PathBuf, cache_dir: PathBuf) -> Result<Self> {
        let project_reports_dir = data_dir.join("project-reports");
        let project_graphs_dir = data_dir.join("project-graphs");
        let scratch_dir = cache_dir.join("scans");
        let parse_dbs_dir = cache_dir.join("parse-dbs");

        for dir in [
            &data_dir,
            &cache_dir,
            &project_reports_dir,
            &project_graphs_dir,
            &scratch_dir,
            &parse_dbs_dir,
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }

        Ok(Self {
            data_dir,
            cache_dir,
            project_reports_dir,
            project_graphs_dir,
            scratch_dir,
            parse_dbs_dir,
        })
    }

    pub fn open_store(&self) -> Result<crate::ProjectStore> {
        crate::ProjectStore::open(
            self.data_dir.join("projects.sqlite"),
            self.project_reports_dir.clone(),
            self.project_graphs_dir.clone(),
        )
    }

    /// Resolve the persistent parse-SQLite path for `project_id`.
    /// The path is `{parse_dbs_dir}/{project_id}/parse.sqlite`; the
    /// parent directory is **NOT** created here — the engine-runner
    /// creates it just before opening the DB so a scan against a
    /// brand-new project never fails on a missing directory.
    pub fn parse_db_for_project(&self, project_id: &str) -> PathBuf {
        self.parse_dbs_dir.join(project_id).join("parse.sqlite")
    }
}

pub fn default_app_data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("GRIDSEAK_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("com.gridseak.desktop"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA").context("APPDATA is not set")?;
        Ok(PathBuf::from(appdata).join("com.gridseak.desktop"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .context("neither XDG_DATA_HOME nor HOME is set")?;
        Ok(base.join("com.gridseak.desktop"))
    }

    #[cfg(not(any(windows, unix)))]
    {
        anyhow::bail!("unsupported platform for GridSeak local store path")
    }
}

/// Platform-default cache directory for GridSeak.
///
/// Mirrors [`default_app_data_dir`] exactly in shape but resolves to
/// the OS's "regenerable cache" path category — `~/Library/Caches` on
/// macOS, `%LOCALAPPDATA%` on Windows, `$XDG_CACHE_HOME` on Linux.
/// Override with the `GRIDSEAK_CACHE_DIR` env var for sandboxed test
/// environments and for the parity integration test (which builds
/// isolated `tempfile::tempdir()` cache roots).
pub fn default_app_cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("GRIDSEAK_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("com.gridseak.desktop"))
    }

    #[cfg(target_os = "windows")]
    {
        // %LOCALAPPDATA% is the documented cache location on Windows;
        // %APPDATA% is the durable equivalent (used by `default_app_data_dir`).
        let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
        Ok(PathBuf::from(local)
            .join("com.gridseak.desktop")
            .join("Cache"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .context("neither XDG_CACHE_HOME nor HOME is set")?;
        Ok(base.join("com.gridseak.desktop"))
    }

    #[cfg(not(any(windows, unix)))]
    {
        anyhow::bail!("unsupported platform for GridSeak cache path")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn from_dirs_creates_parse_dbs_dir_under_cache_root() {
        // The S1-\u{03b5} incremental parse DB MUST land under the
        // cache root (regenerable), not under the durable data root.
        // Losing a parse DB is recoverable by a full rescan; misplacing
        // it under `data_dir` would tie the cache lifecycle to Time
        // Machine backups, which is the wrong invariant.
        let data = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let paths =
            LocalStorePaths::from_dirs(data.path().to_path_buf(), cache.path().to_path_buf())
                .unwrap();

        assert!(
            paths.parse_dbs_dir.starts_with(cache.path()),
            "parse_dbs_dir {:?} must be under cache_dir {:?}",
            paths.parse_dbs_dir,
            cache.path()
        );
        assert!(
            !paths.parse_dbs_dir.starts_with(data.path()),
            "parse_dbs_dir must NOT live under data_dir"
        );
        assert!(
            paths.parse_dbs_dir.exists(),
            "parse_dbs_dir is eagerly created"
        );
    }

    #[test]
    fn parse_db_for_project_yields_namespaced_path_without_creating_dir() {
        let data = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let paths =
            LocalStorePaths::from_dirs(data.path().to_path_buf(), cache.path().to_path_buf())
                .unwrap();

        let project_a = paths.parse_db_for_project("project-a");
        let project_b = paths.parse_db_for_project("project-b");

        assert_ne!(project_a, project_b);
        assert!(project_a.ends_with("project-a/parse.sqlite"));
        assert!(project_b.ends_with("project-b/parse.sqlite"));
        assert!(
            !project_a.parent().unwrap().exists(),
            "per-project dirs are lazy — created by the runner just before opening the DB"
        );
    }
}
