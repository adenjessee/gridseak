//! Run external SCIP indexers (`scip-typescript`, `scip-python`, `scip-go`, …).
//!
//! Scan time never hits the network. `provision_index` reuses a cached
//! `index.scip` or returns `IndexerNotProvisioned`. `install_indexer` is
//! the doctor-only path that may download.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

pub mod registry;

pub use registry::{
    all_entries, auto_provision_enabled, cache_dir, entry_for, indexer_timeout,
    require_cached_or_skip, IndexerEntry, InstallMethod,
};

/// Which batch indexer to invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexerLanguage {
    TypeScript,
    Python,
    Go,
    Rust,
    Java,
    CSharp,
    Cpp,
    Ruby,
}

impl IndexerLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Cpp => "cpp",
            Self::Ruby => "ruby",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ts" | "typescript" | "javascript" | "js" => Some(Self::TypeScript),
            "py" | "python" => Some(Self::Python),
            "go" | "golang" => Some(Self::Go),
            "rs" | "rust" => Some(Self::Rust),
            "java" => Some(Self::Java),
            "cs" | "csharp" | "c#" => Some(Self::CSharp),
            "c" | "cpp" | "c++" | "cxx" => Some(Self::Cpp),
            "rb" | "ruby" => Some(Self::Ruby),
            _ => None,
        }
    }
}

/// Failure from an external indexer subprocess.
///
/// Mapped to `SkipReason::IndexerFailed` or `IndexerNotProvisioned`
/// in the parsing layer (not here).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("indexer failed with exit code {exit_code}: {message}")]
pub struct IndexerError {
    pub exit_code: i32,
    pub message: String,
}

impl IndexerError {
    pub fn not_provisioned(&self) -> bool {
        self.exit_code == 3 || self.message.contains("IndexerNotProvisioned")
    }
}

/// Scan-time: reuse cache, never `npx`.
pub fn provision_index(
    repo_root: impl AsRef<Path>,
    language: IndexerLanguage,
) -> Result<PathBuf, IndexerError> {
    let repo_root = repo_root.as_ref();
    match require_cached_or_skip(repo_root, language) {
        Ok(path) => Ok(path),
        Err(err) if auto_provision_enabled() && !err.not_provisioned() => Err(err),
        Err(_err) if auto_provision_enabled() => install_indexer(repo_root, language),
        Err(err) => Err(err),
    }
}

/// Doctor-only: may spawn `npx` or look up a GitHub-release binary.
pub fn install_indexer(
    repo_root: impl AsRef<Path>,
    language: IndexerLanguage,
) -> Result<PathBuf, IndexerError> {
    let repo_root = repo_root.as_ref();
    let entry = entry_for(language).ok_or_else(|| IndexerError {
        exit_code: 2,
        message: format!("no registry entry for {language:?}"),
    })?;
    if entry.install_method == InstallMethod::Stub {
        return Err(IndexerError {
            exit_code: 3,
            message: format!(
                "IndexerNotProvisioned: {} is a stub until fixtures exist",
                entry.package_or_repo
            ),
        });
    }
    match language {
        IndexerLanguage::TypeScript => provision_typescript(repo_root),
        IndexerLanguage::Python => provision_python(repo_root),
        IndexerLanguage::Go => provision_go(repo_root),
        IndexerLanguage::Rust => provision_rust(repo_root),
        other => Err(IndexerError {
            exit_code: 3,
            message: format!("IndexerNotProvisioned: {other:?} has no installer yet"),
        }),
    }
}

fn provision_typescript(repo_root: &Path) -> Result<PathBuf, IndexerError> {
    if !repo_root.join("package.json").exists() && !repo_root.join("tsconfig.json").exists() {
        return Err(IndexerError {
            exit_code: 1,
            message: "missing package.json and tsconfig.json".into(),
        });
    }
    run_npx_indexer(repo_root, "@sourcegraph/scip-typescript", "scip-typescript")
}

fn provision_python(repo_root: &Path) -> Result<PathBuf, IndexerError> {
    let has_python_project = ["pyproject.toml", "setup.py", "requirements.txt"]
        .iter()
        .any(|f| repo_root.join(f).exists());
    if !has_python_project {
        return Err(IndexerError {
            exit_code: 1,
            message: "missing Python project markers".into(),
        });
    }
    run_npx_indexer(repo_root, "@sourcegraph/scip-python", "scip-python")
}

fn provision_go(repo_root: &Path) -> Result<PathBuf, IndexerError> {
    let bin = ensure_scip_go_binary()?;
    if !repo_root.join("go.mod").exists() {
        return Ok(bin);
    }
    run_named_indexer(repo_root, "scip-go", &[bin.to_str().unwrap_or("scip-go")])
}

/// Doctor / provision path: PATH, then `~/.gridseak/bin/scip-go`, then GitHub release.
fn ensure_scip_go_binary() -> Result<PathBuf, IndexerError> {
    if let Some(p) = which("scip-go") {
        return Ok(p);
    }
    let dest_dir = gridseak_bin_dir();
    let dest = dest_dir.join("scip-go");
    if dest.is_file() {
        return Ok(dest);
    }
    let entry = entry_for(IndexerLanguage::Go).ok_or_else(|| IndexerError {
        exit_code: 2,
        message: "no registry entry for Go".into(),
    })?;
    let (os, arch) = go_release_target();
    let ver = entry.pinned_version;
    let urls = [
        format!("https://github.com/sourcegraph/scip-go/releases/download/v{ver}/scip-go_{ver}_{os}_{arch}.tar.gz"),
        format!("https://github.com/scip-code/scip-go/releases/download/v{ver}/scip-go_{ver}_{os}_{arch}.tar.gz"),
        format!("https://github.com/scip-code/scip-go/releases/download/v{ver}/scip-go-{os}-{arch}.tar.gz"),
        format!(
            "https://github.com/{}/releases/download/v{ver}/scip-go_{}_{}.tar.gz",
            entry.package_or_repo,
            darwin_or_linux_title(os),
            arch_title(arch)
        ),
    ];
    let _ = std::fs::create_dir_all(&dest_dir);
    let tarball = dest_dir.join("scip-go.tar.gz");
    let mut last_url = String::new();
    let mut downloaded = false;
    for url in &urls {
        last_url = url.clone();
        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&tarball)
            .arg(url)
            .status()
            .map_err(|e| IndexerError {
                exit_code: 127,
                message: format!("curl failed for scip-go: {e}"),
            })?;
        if status.success()
            && tarball.is_file()
            && tarball.metadata().map(|m| m.len() > 1000).unwrap_or(false)
        {
            downloaded = true;
            break;
        }
    }
    if !downloaded {
        return Err(IndexerError {
            exit_code: 3,
            message: format!(
                "IndexerNotProvisioned: failed to download scip-go v{ver} (last url {last_url})"
            ),
        });
    }
    if let Some(sum) = entry.checksum_sha256 {
        verify_sha256(&tarball, sum)?;
    }
    let extract = Command::new("tar")
        .args(["-xzf"])
        .arg(&tarball)
        .arg("-C")
        .arg(&dest_dir)
        .status()
        .map_err(|e| IndexerError {
            exit_code: 127,
            message: format!("tar extract scip-go failed: {e}"),
        })?;
    if !extract.success() {
        return Err(IndexerError {
            exit_code: 1,
            message: "failed to extract scip-go tarball".into(),
        });
    }
    if !dest.is_file() {
        if let Some(found) = find_named_file(&dest_dir, "scip-go") {
            let _ = std::fs::copy(&found, &dest);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if dest.is_file() {
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
        }
    }
    if dest.is_file() {
        Ok(dest)
    } else {
        Err(IndexerError {
            exit_code: 3,
            message: format!(
                "IndexerNotProvisioned: scip-go downloaded but binary missing under {}",
                dest_dir.display()
            ),
        })
    }
}

fn gridseak_bin_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".gridseak").join("bin");
    }
    std::env::temp_dir().join("gridseak-bin")
}

fn go_release_target() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    (os, arch)
}

fn darwin_or_linux_title(os: &str) -> &'static str {
    match os {
        "darwin" => "Darwin",
        "linux" => "Linux",
        _ => "Unknown",
    }
}

fn arch_title(arch: &str) -> &'static str {
    match arch {
        "arm64" => "arm64",
        _ => "x86_64",
    }
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), IndexerError> {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| Command::new("sha256sum").arg(path).output())
        .map_err(|e| IndexerError {
            exit_code: 1,
            message: format!("checksum tool missing: {e}"),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let got = stdout.split_whitespace().next().unwrap_or("");
    if got.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(IndexerError {
            exit_code: 1,
            message: format!("scip-go checksum mismatch: got {got}, expected {expected}"),
        })
    }
}

fn find_named_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    for ent in rd.flatten() {
        let p = ent.path();
        if p.is_file() && p.file_name().and_then(|s| s.to_str()) == Some(name) {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(hit) = find_named_file(&p, name) {
                return Some(hit);
            }
        }
    }
    None
}

fn provision_rust(repo_root: &Path) -> Result<PathBuf, IndexerError> {
    if !repo_root.join("Cargo.toml").exists() {
        return Err(IndexerError {
            exit_code: 1,
            message: "missing Cargo.toml".into(),
        });
    }
    if which("rust-analyzer").is_none() {
        return Err(IndexerError {
            exit_code: 3,
            message: "IndexerNotProvisioned: rust-analyzer is not on PATH".into(),
        });
    }
    run_named_indexer(repo_root, "rust-analyzer", &["rust-analyzer", "scip"])
}

fn run_npx_indexer(repo_root: &Path, package: &str, label: &str) -> Result<PathBuf, IndexerError> {
    let output = Command::new("npx")
        .args(["--yes", package, "index", "--output", "index.scip"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| IndexerError {
            exit_code: 127,
            message: format!("failed to spawn npx for {label}: {e}"),
        })?;
    finish_index(repo_root, label, &output)
}

fn run_named_indexer(
    repo_root: &Path,
    label: &str,
    argv: &[&str],
) -> Result<PathBuf, IndexerError> {
    let (bin, args) = argv.split_first().ok_or_else(|| IndexerError {
        exit_code: 2,
        message: format!("{label} has empty command"),
    })?;
    let output = Command::new(bin)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|e| IndexerError {
            exit_code: 127,
            message: format!("failed to spawn {label}: {e}"),
        })?;
    finish_index(repo_root, label, &output)
}

fn finish_index(
    repo_root: &Path,
    label: &str,
    output: &std::process::Output,
) -> Result<PathBuf, IndexerError> {
    let index_path = repo_root.join("index.scip");
    if output.status.success() && index_path.exists() {
        let language = IndexerLanguage::parse(label)
            .or_else(|| label.strip_prefix("scip-").and_then(IndexerLanguage::parse))
            .unwrap_or(IndexerLanguage::TypeScript);
        if let Some(entry) = entry_for(language) {
            let dest_dir = cache_dir(repo_root, entry);
            let _ = std::fs::create_dir_all(&dest_dir);
            let dest = dest_dir.join("index.scip");
            let _ = std::fs::copy(&index_path, &dest);
        }
        if let Some(fp) = crate::fingerprint::fingerprint_workspace(repo_root, language) {
            crate::fingerprint::write_sidecar(repo_root, language, &fp);
        }
        return Ok(index_path);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(IndexerError {
        exit_code: output.status.code().unwrap_or(1),
        message: format!("{label} indexer failed\nstdout: {stdout}\nstderr: {stderr}"),
    })
}

fn which(bin: &str) -> Option<PathBuf> {
    env_path_dirs().into_iter().find_map(|dir| {
        let p = dir.join(bin);
        p.is_file().then_some(p)
    })
}

fn env_path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default()
}

/// Upper bound for live indexer subprocesses (used by callers that wrap provisioning).
/// Prefer [`indexer_timeout`] which honours `INDEXER_TIMEOUT`.
pub const INDEXER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn scan_time_does_not_spawn_npx() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("package.json"), "{ not valid json").unwrap();
        let err = provision_index(dir.path(), IndexerLanguage::TypeScript).unwrap_err();
        assert_eq!(err.exit_code, 3);
        assert!(err.message.contains("IndexerNotProvisioned"));
        assert!(!err.message.contains("npx"));
    }

    #[test]
    fn missing_project_markers_is_not_provisioned_at_scan() {
        let dir = TempDir::new().unwrap();
        let err = provision_index(dir.path(), IndexerLanguage::TypeScript).unwrap_err();
        assert_eq!(err.exit_code, 3);
        assert!(err.not_provisioned());
    }

    #[test]
    fn doctor_install_still_checks_markers() {
        let dir = TempDir::new().unwrap();
        let err = install_indexer(dir.path(), IndexerLanguage::TypeScript).unwrap_err();
        assert_eq!(err.exit_code, 1);
        assert!(err.message.contains("missing package.json"));
    }

    #[test]
    fn stub_language_is_not_provisioned() {
        let dir = TempDir::new().unwrap();
        let err = provision_index(dir.path(), IndexerLanguage::Java).unwrap_err();
        assert!(err.not_provisioned());
    }
}
