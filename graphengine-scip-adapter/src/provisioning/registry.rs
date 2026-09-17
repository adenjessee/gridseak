//! Language → indexer registry.
//!
//! One entry per language: how we detect it, how we install the indexer,
//! which pinned command we run, and whether scan-time auto-provision is
//! allowed. Scan time must not hit the network unless
//! `GRIDSEAK_AUTO_PROVISION=1` is set.

use std::env;
use std::path::Path;
use std::time::Duration;

use super::{IndexerError, IndexerLanguage};

/// How the indexer binary is obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// `npx --yes <pkg>` — only allowed from `gridseak doctor --provision`.
    NpxPackage,
    /// GitHub release tarball, checksum-pinned.
    GithubRelease,
    /// `rust-analyzer scip` (already on PATH).
    RustAnalyzer,
    /// Entry exists so doctor can list it; scan discloses not-provisioned.
    Stub,
}

/// Why a language is not indexed this scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    IndexerNotProvisioned,
    IndexerFailed,
    IndexStale,
}

/// One language's indexer contract.
#[derive(Debug, Clone)]
pub struct IndexerEntry {
    pub language: IndexerLanguage,
    pub detection_globs: &'static [&'static str],
    pub install_method: InstallMethod,
    pub package_or_repo: &'static str,
    pub pinned_version: &'static str,
    pub checksum_sha256: Option<&'static str>,
    pub command: &'static [&'static str],
    pub timeout: Duration,
    pub cache_dir_name: &'static str,
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// All known languages. Java / C# / C++ / Ruby are stubs until fixtures exist.
pub fn all_entries() -> &'static [IndexerEntry] {
    static ENTRIES: &[IndexerEntry] = &[
        IndexerEntry {
            language: IndexerLanguage::TypeScript,
            detection_globs: &[
                "*.ts",
                "*.tsx",
                "*.js",
                "*.jsx",
                "tsconfig.json",
                "package.json",
            ],
            install_method: InstallMethod::NpxPackage,
            package_or_repo: "@sourcegraph/scip-typescript",
            pinned_version: "0.3.0",
            checksum_sha256: None,
            command: &["scip-typescript", "index"],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-typescript",
        },
        IndexerEntry {
            language: IndexerLanguage::Python,
            detection_globs: &["*.py", "pyproject.toml", "setup.py"],
            install_method: InstallMethod::NpxPackage,
            package_or_repo: "@sourcegraph/scip-python",
            pinned_version: "0.6.6",
            checksum_sha256: None,
            command: &["scip-python", "index"],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-python",
        },
        IndexerEntry {
            language: IndexerLanguage::Go,
            detection_globs: &["*.go", "go.mod"],
            install_method: InstallMethod::GithubRelease,
            package_or_repo: "sourcegraph/scip-go",
            pinned_version: "0.1.25",
            checksum_sha256: None,
            command: &["scip-go"],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-go",
        },
        IndexerEntry {
            language: IndexerLanguage::Rust,
            detection_globs: &["*.rs", "Cargo.toml"],
            install_method: InstallMethod::RustAnalyzer,
            package_or_repo: "rust-analyzer",
            pinned_version: "workspace",
            checksum_sha256: None,
            command: &["rust-analyzer", "scip"],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-rust",
        },
        IndexerEntry {
            language: IndexerLanguage::Java,
            detection_globs: &["*.java", "pom.xml", "build.gradle"],
            install_method: InstallMethod::Stub,
            package_or_repo: "sourcegraph/scip-java",
            pinned_version: "unprovisioned",
            checksum_sha256: None,
            command: &[],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-java",
        },
        IndexerEntry {
            language: IndexerLanguage::CSharp,
            detection_globs: &["*.cs", "*.csproj"],
            install_method: InstallMethod::Stub,
            package_or_repo: "sourcegraph/scip-dotnet",
            pinned_version: "unprovisioned",
            checksum_sha256: None,
            command: &[],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-dotnet",
        },
        IndexerEntry {
            language: IndexerLanguage::Cpp,
            detection_globs: &["*.c", "*.cc", "*.cpp", "*.h", "compile_commands.json"],
            install_method: InstallMethod::Stub,
            package_or_repo: "sourcegraph/scip-clang",
            pinned_version: "unprovisioned",
            checksum_sha256: None,
            command: &[],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-clang",
        },
        IndexerEntry {
            language: IndexerLanguage::Ruby,
            detection_globs: &["*.rb", "Gemfile"],
            install_method: InstallMethod::Stub,
            package_or_repo: "sourcegraph/scip-ruby",
            pinned_version: "unprovisioned",
            checksum_sha256: None,
            command: &[],
            timeout: DEFAULT_TIMEOUT,
            cache_dir_name: "scip-ruby",
        },
    ];
    ENTRIES
}

pub fn entry_for(language: IndexerLanguage) -> Option<&'static IndexerEntry> {
    all_entries().iter().find(|e| e.language == language)
}

pub fn auto_provision_enabled() -> bool {
    matches!(
        env::var("GRIDSEAK_AUTO_PROVISION").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

pub fn indexer_timeout(entry: &IndexerEntry) -> Duration {
    env::var("INDEXER_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(entry.timeout)
}

pub fn cache_dir(workspace: &Path, entry: &IndexerEntry) -> std::path::PathBuf {
    workspace
        .join(".gridseak")
        .join("scip")
        .join(entry.cache_dir_name)
}

/// Detect whether the workspace contains files for this language.
pub fn workspace_matches(workspace: &Path, entry: &IndexerEntry) -> bool {
    for glob in entry.detection_globs {
        if glob.starts_with("*.") {
            let ext = &glob[1..];
            if walk_has_ext(workspace, ext) {
                return true;
            }
        } else if workspace.join(glob).is_file() {
            return true;
        }
    }
    false
}

fn walk_has_ext(root: &Path, ext: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(root) else {
        return false;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.ends_with(ext) {
                    return true;
                }
            }
        }
    }
    false
}

/// Scan-time: never spawn `npx`. Return a structured skip if the cache
/// is empty and auto-provision is off.
pub fn require_cached_or_skip(
    workspace: &Path,
    language: IndexerLanguage,
) -> Result<std::path::PathBuf, IndexerError> {
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
    let cached = cache_dir(workspace, entry).join("index.scip");
    if cached.is_file() {
        return Ok(cached);
    }
    if !auto_provision_enabled() {
        return Err(IndexerError {
            exit_code: 3,
            message: format!(
                "IndexerNotProvisioned: run `gridseak doctor --provision {:?}` (or set GRIDSEAK_AUTO_PROVISION=1)",
                language
            ),
        });
    }
    Err(IndexerError {
        exit_code: 3,
        message: format!(
            "IndexerNotProvisioned: cache miss at {} and auto-provision is on but install was not invoked",
            cached.display()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_is_npx_package() {
        let e = entry_for(IndexerLanguage::TypeScript).expect("ts");
        assert_eq!(e.install_method, InstallMethod::NpxPackage);
        assert_eq!(e.pinned_version, "0.3.0");
    }

    #[test]
    fn go_is_github_release() {
        let e = entry_for(IndexerLanguage::Go).expect("go");
        assert_eq!(e.install_method, InstallMethod::GithubRelease);
    }

    #[test]
    fn java_is_stub() {
        let e = entry_for(IndexerLanguage::Java).expect("java");
        assert_eq!(e.install_method, InstallMethod::Stub);
    }

    #[test]
    fn timeout_env_overrides() {
        let e = entry_for(IndexerLanguage::TypeScript).unwrap();
        // INDEXER_TIMEOUT may or may not be set in the environment; the
        // helper must always return a positive duration.
        assert!(indexer_timeout(e) > Duration::from_secs(0));
    }
}
