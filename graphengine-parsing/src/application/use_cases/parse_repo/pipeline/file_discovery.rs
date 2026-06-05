//! File discovery for source files in a repository

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::SyntaxExtractor;
use std::collections::HashSet;
use tracing::{info, warn};

/// Directories that are always excluded from file discovery.
/// These contain third-party dependencies, build output, or VCS internals
/// that should never be part of a project's own source graph.
const EXCLUDED_DIR_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "vendor",
    "third_party",
    "third-party",
    "dist",
    "build",
    "out",
    ".build",
    ".next",
    ".nuxt",
    ".output",
    ".cache",
    ".parcel-cache",
    ".wrangler", // Cloudflare Workers / Astro dev bundles (not first-party source)
    ".astro",    // Astro generated types (see .gitignore)
    "archive",   // archived snapshots (e.g. docs/archive on gridseak.com)
    "__pycache__",
    ".tox",
    ".venv",
    "venv",
    "env",
    ".env",
    "target",   // Rust/Maven build output
    "Pods",     // CocoaPods
    "Carthage", // Carthage (iOS)
    ".gradle",
    ".idea",
    ".vs",
    ".vscode",
    "coverage",
    ".nyc_output",
    "bower_components",
];

/// Directory name prefixes that are excluded case-insensitively.
///
/// Unlike `EXCLUDED_DIR_NAMES` (which does exact, case-sensitive
/// match on the directory basename), these prefixes match against the
/// lowercased basename with `starts_with`. Use this when a directory
/// family spans multiple naming conventions across projects:
///
/// - `staticresource*` covers both the canonical SFDX
///   `staticresources/` directory and the common NPSP / EDA / DX
///   pre-bundle convention `StaticResourceSources/`. Both hold
///   minified JS/CSS bundles, images, archives, and other deploy-
///   time payloads — never first-party Apex source. Parsing these
///   produces phantom Function nodes from minified tokens
///   (see R19: NPSP emitted 924+ such nodes). Excluding at the walk
///   boundary is the only place the fix cannot be missed downstream.
///
/// Adding new entries here is preferable to adding multiple casing/
/// spelling variants to `EXCLUDED_DIR_NAMES`.
const EXCLUDED_DIR_PREFIXES_CI: &[&str] = &["staticresource"];

/// File discovery service
pub struct FileDiscovery;

impl FileDiscovery {
    /// Discover source files in the repository, automatically excluding
    /// common dependency and build output directories.
    ///
    /// # Arguments
    /// * `root` - Root directory of the repository
    /// * `language` - Programming language to parse
    /// * `syntax_extractor` - Extractor to check file support
    ///
    /// # Returns
    /// * `Vec<PathBuf>` - List of discovered source files
    /// * `ParsingError` - If discovery fails
    pub fn discover_source_files(
        root: &std::path::Path,
        language: &str,
        syntax_extractor: &dyn SyntaxExtractor,
    ) -> Result<Vec<std::path::PathBuf>, ParsingError> {
        info!(
            "Discovering files in {} for language {}",
            root.display(),
            language
        );

        let excluded: HashSet<&str> = EXCLUDED_DIR_NAMES.iter().copied().collect();
        let mut skipped_dirs: Vec<String> = Vec::new();

        let files = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|entry| {
                if entry.file_type().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if Self::is_excluded_dir_name(name, &excluded) {
                            skipped_dirs.push(
                                entry
                                    .path()
                                    .strip_prefix(root)
                                    .unwrap_or(entry.path())
                                    .display()
                                    .to_string(),
                            );
                            return false;
                        }
                    }
                }
                true
            })
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| Self::is_supported_file(entry.path(), syntax_extractor))
            .map(|entry| {
                let path = entry.into_path();
                match path.canonicalize() {
                    Ok(canonical) => canonical,
                    Err(err) => {
                        warn!(
                            "Failed to canonicalize path '{}': {}. Using original path.",
                            path.display(),
                            err
                        );
                        path
                    }
                }
            })
            .collect::<Vec<_>>();

        if !skipped_dirs.is_empty() {
            info!(
                "Skipped {} excluded directories: {}",
                skipped_dirs.len(),
                skipped_dirs.join(", ")
            );
        }

        info!("Discovered {} source files", files.len());
        for file in &files {
            info!("  Found file: {}", file.display());
        }

        Ok(files)
    }

    /// Check if a file is supported for the given language
    fn is_supported_file(path: &std::path::Path, syntax_extractor: &dyn SyntaxExtractor) -> bool {
        if let Some(extension) = path.extension() {
            if let Some(ext_str) = extension.to_str() {
                return syntax_extractor.supports_extension(&format!(".{}", ext_str));
            }
        }
        false
    }

    /// Returns the list of directory names excluded from discovery.
    /// Useful for tests and diagnostics.
    pub fn excluded_dir_names() -> &'static [&'static str] {
        EXCLUDED_DIR_NAMES
    }

    /// Returns the list of case-insensitive directory name prefixes
    /// excluded from discovery. Useful for tests and diagnostics.
    pub fn excluded_dir_prefixes_ci() -> &'static [&'static str] {
        EXCLUDED_DIR_PREFIXES_CI
    }

    /// Returns true if the directory basename `name` should be pruned
    /// from the discovery walk. Matches either an exact name in
    /// `excluded` (case-sensitive) or a case-insensitive prefix in
    /// `EXCLUDED_DIR_PREFIXES_CI`.
    fn is_excluded_dir_name(name: &str, excluded: &HashSet<&str>) -> bool {
        if excluded.contains(name) {
            return true;
        }
        let lower = name.to_ascii_lowercase();
        EXCLUDED_DIR_PREFIXES_CI
            .iter()
            .any(|prefix| lower.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staticresource_prefix_is_in_ci_exclusion_list() {
        // R19: Salesforce static-resource directories must be pruned
        // at the walk boundary so minified JS payloads never produce
        // phantom Function nodes. NPSP uses `StaticResourceSources/`
        // (non-canonical casing + extra suffix) alongside canonical
        // `staticresources/`; a case-insensitive prefix predicate
        // covers both variants.
        let prefixes = FileDiscovery::excluded_dir_prefixes_ci();
        assert!(
            prefixes.contains(&"staticresource"),
            "staticresource prefix must be in CI discovery exclusion list (R19)"
        );
    }

    #[test]
    fn is_excluded_dir_name_matches_canonical_staticresources() {
        let excluded: HashSet<&str> = EXCLUDED_DIR_NAMES.iter().copied().collect();
        assert!(FileDiscovery::is_excluded_dir_name(
            "staticresources",
            &excluded
        ));
    }

    #[test]
    fn is_excluded_dir_name_matches_npsp_static_resource_sources() {
        // The NPSP `StaticResourceSources/` directory holds the
        // unminified source for their static-resource bundles and
        // must be pruned. The original literal exclusion missed this
        // variant (hand-audit: 10/10 `visibility_private_unused`
        // samples came from here).
        let excluded: HashSet<&str> = EXCLUDED_DIR_NAMES.iter().copied().collect();
        assert!(FileDiscovery::is_excluded_dir_name(
            "StaticResourceSources",
            &excluded
        ));
    }

    #[test]
    fn is_excluded_dir_name_is_case_insensitive_for_prefix() {
        let excluded: HashSet<&str> = EXCLUDED_DIR_NAMES.iter().copied().collect();
        assert!(FileDiscovery::is_excluded_dir_name(
            "STATICRESOURCES",
            &excluded
        ));
        assert!(FileDiscovery::is_excluded_dir_name(
            "StaticResource_Bundle",
            &excluded
        ));
    }

    #[test]
    fn is_excluded_dir_name_does_not_overmatch_unrelated_dirs() {
        // `src/` must not be excluded just because it is lowercased;
        // confirm the CI prefix predicate isn't too aggressive.
        let excluded: HashSet<&str> = EXCLUDED_DIR_NAMES.iter().copied().collect();
        assert!(!FileDiscovery::is_excluded_dir_name("src", &excluded));
        assert!(!FileDiscovery::is_excluded_dir_name(
            "StaticAnalysis",
            &excluded
        ));
        assert!(!FileDiscovery::is_excluded_dir_name("resources", &excluded));
    }

    #[test]
    fn is_excluded_dir_name_matches_astro_cloudflare_dirs() {
        let excluded: HashSet<&str> = EXCLUDED_DIR_NAMES.iter().copied().collect();
        assert!(FileDiscovery::is_excluded_dir_name(".wrangler", &excluded));
        assert!(FileDiscovery::is_excluded_dir_name(".astro", &excluded));
        assert!(FileDiscovery::is_excluded_dir_name("archive", &excluded));
    }
}
