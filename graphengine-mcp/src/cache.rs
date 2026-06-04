use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use std::time::SystemTime;

use graphengine_analysis::health::report::HealthReport;

struct CacheEntry {
    report: HealthReport,
    db_modified: SystemTime,
}

/// In-memory cache mapping db_path → (HealthReport, db_file_mtime).
///
/// Cache is invalidated when the SQLite file's modification time changes,
/// meaning a re-parse or external mutation occurred since the last analysis.
pub struct ReportCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl ReportCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Return a cached report if the DB file has not been modified since it was cached.
    pub fn get(&self, db_path: &str) -> Option<HealthReport> {
        let current_mtime = fs::metadata(db_path).ok()?.modified().ok()?;
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(db_path)?;
        if entry.db_modified == current_mtime {
            Some(entry.report.clone())
        } else {
            None
        }
    }

    /// Store a report in the cache, keyed by db_path with the file's current mtime.
    pub fn insert(&self, db_path: &str, report: HealthReport) {
        let mtime = fs::metadata(db_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                db_path.to_string(),
                CacheEntry {
                    report,
                    db_modified: mtime,
                },
            );
        }
    }

    /// Remove a specific entry (e.g. after re-parse invalidates it).
    pub fn invalidate(&self, db_path: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(db_path);
        }
    }
}
