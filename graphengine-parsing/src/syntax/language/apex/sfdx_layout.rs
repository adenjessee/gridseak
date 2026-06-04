//! SFDX / MDAPI project layout detection for Apex scanning.
//!
//! Salesforce codebases ship in one of three shapes on disk:
//!
//! 1. **SFDX** — authoritative. A `sfdx-project.json` file at the repo root
//!    declares one or more `packageDirectories`, each typically rooted at
//!    `force-app/main/default/`. This is what the `sf` / `sfdx` CLI produces.
//! 2. **Force-App heuristic** — no manifest, but the conventional
//!    `force-app/main/default/classes/` tree exists. Common for forks and
//!    trimmed-down exports.
//! 3. **MDAPI** — legacy Metadata API retrieval. `classes/`, `triggers/`,
//!    `objects/`, etc. sit directly under the root with no manifest.
//!
//! Anything that doesn't match one of the above but still contains `.cls` /
//! `.trigger` / `*-meta.xml` files is classified as
//! [`LayoutKind::FileExtensionScan`] with a diagnostic so users understand
//! that their layout fell through every authoritative check.
//!
//! The detector is deliberately **read-only and side-effect-free**. It
//! performs no network calls, no git operations, and does not spawn
//! processes. It returns an [`SfdxLayout`] value that downstream stages feed
//! into:
//!
//! - `apex-jorje-lsp` initialization (`rootUri` and `workspaceFolders`).
//! - Metadata XML readers (object / trigger meta classification).
//! - The class-registry builder (which `.cls` files to index).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;
use url::Url;
use walkdir::WalkDir;

/// Hard cap on how many ancestor directories we'll walk looking for the
/// authoritative `sfdx-project.json` manifest. Prevents pathological walks
/// on symlink cycles or deeply nested inputs.
const MAX_ANCESTOR_WALK: usize = 32;

/// Hard cap on directory-walk depth during file classification. Well in
/// excess of any real SFDX layout (`force-app/main/default/objects/X/fields`
/// is 6) while still preventing runaway walks on unintended inputs.
const MAX_WALK_DEPTH: usize = 24;

/// Directory names we never descend into during classification. These are
/// never legitimate SFDX content roots but frequently contain huge file
/// counts that would inflate scan time and memory.
const SKIPPED_DIR_NAMES: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "target",
    "dist",
    "build",
    ".sfdx",
    ".sf",
    ".vscode",
    ".idea",
    ".cache",
];

/// How the layout was discovered. Recorded in the returned [`SfdxLayout`]
/// and surfaced in report diagnostics so users can see *why* a given set of
/// files was or wasn't picked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    /// Authoritative SFDX: `sfdx-project.json` was located.
    Sfdx,
    /// Heuristic SFDX: no manifest, but `force-app/main/default/...` exists.
    ForceAppHeuristic,
    /// Classic Metadata API retrieval: `classes/`, `triggers/`, `objects/`
    /// sit directly under the root with no manifest.
    Mdapi,
    /// None of the above matched, but Apex-looking files were found by
    /// extension. Lowest confidence — downstream stages should still run
    /// but the diagnostic tells the user their layout is non-standard.
    FileExtensionScan,
}

impl LayoutKind {
    /// Short identifier suitable for telemetry / report provenance.
    pub fn as_str(&self) -> &'static str {
        match self {
            LayoutKind::Sfdx => "sfdx",
            LayoutKind::ForceAppHeuristic => "force_app_heuristic",
            LayoutKind::Mdapi => "mdapi",
            LayoutKind::FileExtensionScan => "file_extension_scan",
        }
    }
}

/// Every Apex / metadata file classified by the detector, grouped by role.
/// Paths are **absolute and canonicalized** when possible so downstream
/// stages never have to re-resolve them relative to the workspace root.
#[derive(Debug, Default, Clone)]
pub struct ClassifiedFiles {
    /// `*.cls` source files. Fed into the Tree-sitter / LSP pipeline.
    pub apex_classes: Vec<PathBuf>,
    /// `*.cls-meta.xml` companion metadata (api version, status, etc.).
    pub apex_class_meta: Vec<PathBuf>,
    /// `*.trigger` source files. Also fed into the pipeline; the trigger's
    /// target SObject is captured from the source header, not the meta XML.
    pub apex_triggers: Vec<PathBuf>,
    /// `*.trigger-meta.xml` companion metadata.
    pub apex_trigger_meta: Vec<PathBuf>,
    /// `<ObjectName>.object-meta.xml` files — source of custom SObject
    /// node creation in Phase 1.
    pub object_meta: Vec<PathBuf>,
    /// `<FieldName>.field-meta.xml` files — reserved for Phase 2 scope;
    /// collected now so we don't need to rewalk the tree later.
    pub field_meta: Vec<PathBuf>,
    /// `*.apxc` (anonymous Apex) files — rare, but valid Apex source.
    pub apex_anonymous: Vec<PathBuf>,
    /// `*.page` Visualforce pages. Not parsed via tree-sitter — the
    /// Phase-A VF pass (TR-A.5) reads them via `quick-xml` to extract
    /// `controller` / `extensions` / `{!method}` bindings and synthesize
    /// `__vf_page__` caller nodes that bind VF pages to Apex methods.
    pub apex_vf_pages: Vec<PathBuf>,
}

impl ClassifiedFiles {
    /// `true` when no Apex-bearing file was found anywhere in the layout.
    /// Used to decide whether to emit a "no Apex sources detected"
    /// diagnostic rather than silently producing an empty graph.
    pub fn is_empty(&self) -> bool {
        self.apex_classes.is_empty()
            && self.apex_triggers.is_empty()
            && self.apex_anonymous.is_empty()
            && self.apex_class_meta.is_empty()
            && self.apex_trigger_meta.is_empty()
            && self.object_meta.is_empty()
            && self.field_meta.is_empty()
            && self.apex_vf_pages.is_empty()
    }

    /// Total number of classified files across every bucket.
    pub fn total(&self) -> usize {
        self.apex_classes.len()
            + self.apex_class_meta.len()
            + self.apex_triggers.len()
            + self.apex_trigger_meta.len()
            + self.object_meta.len()
            + self.field_meta.len()
            + self.apex_anonymous.len()
            + self.apex_vf_pages.len()
    }
}

/// Result of running [`detect`] against an input path.
#[derive(Debug, Clone)]
pub struct SfdxLayout {
    /// How the layout was identified.
    pub kind: LayoutKind,
    /// The directory used as the LSP workspace root. For SFDX, the
    /// directory containing `sfdx-project.json`. For all other shapes, the
    /// input path itself.
    pub workspace_root: PathBuf,
    /// Package directories scanned for Apex content. Always non-empty —
    /// the workspace root itself is used as the single entry when no
    /// explicit package layout is declared.
    pub package_directories: Vec<PathBuf>,
    /// Every classified file.
    pub files: ClassifiedFiles,
    /// Human-readable notes for the user: missing manifest, non-standard
    /// layout, unreadable directories, etc. These are *informational*, not
    /// hard errors — the detector still produces a usable result.
    pub diagnostics: Vec<String>,
}

impl SfdxLayout {
    /// `file://` URL for the workspace root, suitable for LSP `rootUri`
    /// and `workspace_folders[].uri`. Returns an error only if the root
    /// path is not valid UTF-8 on platforms where that matters (never on
    /// Windows/macOS/Linux).
    pub fn workspace_url(&self) -> Result<Url> {
        Url::from_directory_path(&self.workspace_root).map_err(|_| {
            anyhow::anyhow!(
                "failed to build file:// URL for workspace root {}",
                self.workspace_root.display()
            )
        })
    }

    /// Convenience for surfacing the detector's outcome in a one-line
    /// report summary.
    pub fn summary(&self) -> String {
        format!(
            "Apex layout: {} (root={}, packages={}, files={})",
            self.kind.as_str(),
            self.workspace_root.display(),
            self.package_directories.len(),
            self.files.total()
        )
    }
}

/// Detect the Apex project layout anchored at `start`.
///
/// The detector walks upward from `start` in search of `sfdx-project.json`
/// (authoritative) and, failing that, inspects `start` itself for the
/// force-app / MDAPI conventions. It always returns a layout — even when
/// nothing Apex-looking is found — so callers can uniformly surface a
/// diagnostic instead of splitting their code paths on Option.
pub fn detect(start: &Path) -> Result<SfdxLayout> {
    let start = canonicalize_best_effort(start);
    if !start.exists() {
        anyhow::bail!(
            "SFDX layout detection requires an existing path, got: {}",
            start.display()
        );
    }

    let mut diagnostics: Vec<String> = Vec::new();

    if let Some(root) = find_sfdx_project_root(&start, &mut diagnostics) {
        debug!("Apex layout: sfdx-project.json at {}", root.display());
        return build_sfdx_layout(root, diagnostics);
    }

    // No manifest — inspect `start` for conventional shapes.
    let scan_root = if start.is_file() {
        start.parent().unwrap_or(Path::new("/")).to_path_buf()
    } else {
        start.clone()
    };

    if has_force_app_layout(&scan_root) {
        diagnostics.push(format!(
            "No sfdx-project.json found; using force-app/main/default heuristic at {}",
            scan_root.display()
        ));
        return build_force_app_layout(scan_root, diagnostics);
    }

    if has_mdapi_layout(&scan_root) {
        diagnostics.push(format!(
            "No sfdx-project.json found; detected MDAPI layout at {}",
            scan_root.display()
        ));
        return build_mdapi_layout(scan_root, diagnostics);
    }

    // Last-ditch fallback: take whatever Apex files exist, regardless of
    // directory shape. Still a usable layout — just lowest-confidence.
    diagnostics.push(format!(
        "No sfdx-project.json, force-app, or MDAPI layout detected at {}. \
         Falling back to extension-only scan; results may miss metadata \
         that relies on canonical SFDX directory structure.",
        scan_root.display()
    ));
    build_file_extension_layout(scan_root, diagnostics)
}

// -----------------------------------------------------------------------------
// Authoritative SFDX (sfdx-project.json)
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SfdxProjectManifest {
    #[serde(default)]
    package_directories: Vec<SfdxPackageDir>,
}

#[derive(Debug, Deserialize)]
struct SfdxPackageDir {
    path: String,
}

/// Walk up from `start` looking for `sfdx-project.json`, capped at
/// [`MAX_ANCESTOR_WALK`] directories. Returns the directory that contains
/// the manifest (not the manifest path itself).
fn find_sfdx_project_root(start: &Path, diagnostics: &mut Vec<String>) -> Option<PathBuf> {
    let origin = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };

    let mut cur: Option<PathBuf> = Some(origin);
    for _ in 0..MAX_ANCESTOR_WALK {
        let dir = match cur {
            Some(d) => d,
            None => break,
        };
        let manifest = dir.join("sfdx-project.json");
        if manifest.is_file() {
            // Sanity-probe the file contents so a stray empty / malformed
            // `sfdx-project.json` doesn't make us silently misclassify
            // everything below it as SFDX. We succeed on any parseable JSON
            // even when `packageDirectories` is empty — that's a valid
            // SFDX project, just one with no declared package paths.
            match std::fs::read_to_string(&manifest) {
                Ok(raw) => {
                    if serde_json::from_str::<serde_json::Value>(&raw).is_ok() {
                        return Some(dir);
                    }
                    diagnostics.push(format!(
                        "Found sfdx-project.json at {} but it is not valid JSON; \
                         ignoring and continuing ancestor search.",
                        manifest.display()
                    ));
                }
                Err(e) => {
                    diagnostics.push(format!(
                        "Could not read sfdx-project.json at {} ({}); ignoring.",
                        manifest.display(),
                        e
                    ));
                }
            }
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    None
}

fn build_sfdx_layout(root: PathBuf, mut diagnostics: Vec<String>) -> Result<SfdxLayout> {
    let manifest_path = root.join("sfdx-project.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;

    // Parse tolerantly — invalid camelCase fields or extra content should
    // not kill the whole detection. We already validated parseability in
    // `find_sfdx_project_root`, but `serde_json::from_str::<SfdxProjectManifest>`
    // can still fail if `packageDirectories[].path` is missing or typed
    // wrong. Fall back to force-app in that case.
    let pkg_rel: Vec<String> = match serde_json::from_str::<SfdxProjectManifest>(&raw) {
        Ok(parsed) if !parsed.package_directories.is_empty() => parsed
            .package_directories
            .into_iter()
            .map(|d| d.path)
            .collect(),
        Ok(_) => {
            diagnostics.push(
                "sfdx-project.json has no packageDirectories entries; \
                 defaulting to force-app."
                    .to_string(),
            );
            vec!["force-app".to_string()]
        }
        Err(e) => {
            diagnostics.push(format!(
                "sfdx-project.json is structurally invalid ({e}); \
                 defaulting to force-app."
            ));
            vec!["force-app".to_string()]
        }
    };

    let mut package_directories: Vec<PathBuf> = Vec::with_capacity(pkg_rel.len());
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for rel in pkg_rel {
        let abs = canonicalize_best_effort(&root.join(&rel));
        if !abs.exists() {
            diagnostics.push(format!(
                "sfdx-project.json declares packageDirectories[].path = '{}' \
                 but {} does not exist on disk; skipping.",
                rel,
                abs.display()
            ));
            continue;
        }
        if seen.insert(abs.clone()) {
            package_directories.push(abs);
        }
    }

    if package_directories.is_empty() {
        // Every declared package was missing — degrade to scanning the
        // root so we still produce *some* output instead of nothing.
        diagnostics.push(
            "No valid packageDirectories resolved on disk; \
             scanning workspace root directly."
                .to_string(),
        );
        package_directories.push(root.clone());
    }

    let files = classify_files_in_roots(&package_directories, &mut diagnostics);

    Ok(SfdxLayout {
        kind: LayoutKind::Sfdx,
        workspace_root: root,
        package_directories,
        files,
        diagnostics,
    })
}

// -----------------------------------------------------------------------------
// Heuristic force-app layout (no manifest)
// -----------------------------------------------------------------------------

/// `true` if `<root>/force-app/main/default/classes` or
/// `<root>/force-app/main/default/triggers` exists. One-of is enough — some
/// trimmed exports ship only triggers, some only classes.
fn has_force_app_layout(root: &Path) -> bool {
    let base = root.join("force-app").join("main").join("default");
    base.join("classes").is_dir() || base.join("triggers").is_dir()
}

fn build_force_app_layout(root: PathBuf, mut diagnostics: Vec<String>) -> Result<SfdxLayout> {
    let pkg = canonicalize_best_effort(&root.join("force-app"));
    let package_directories = vec![pkg];
    let files = classify_files_in_roots(&package_directories, &mut diagnostics);
    Ok(SfdxLayout {
        kind: LayoutKind::ForceAppHeuristic,
        workspace_root: root,
        package_directories,
        files,
        diagnostics,
    })
}

// -----------------------------------------------------------------------------
// Classic MDAPI layout
// -----------------------------------------------------------------------------

/// `true` if at least one of the canonical MDAPI buckets
/// (`classes/`, `triggers/`, `objects/`) exists *directly* under `root`.
fn has_mdapi_layout(root: &Path) -> bool {
    root.join("classes").is_dir() || root.join("triggers").is_dir() || root.join("objects").is_dir()
}

fn build_mdapi_layout(root: PathBuf, mut diagnostics: Vec<String>) -> Result<SfdxLayout> {
    let package_directories = vec![root.clone()];
    let files = classify_files_in_roots(&package_directories, &mut diagnostics);
    Ok(SfdxLayout {
        kind: LayoutKind::Mdapi,
        workspace_root: root,
        package_directories,
        files,
        diagnostics,
    })
}

// -----------------------------------------------------------------------------
// Last-resort file-extension fallback
// -----------------------------------------------------------------------------

fn build_file_extension_layout(root: PathBuf, mut diagnostics: Vec<String>) -> Result<SfdxLayout> {
    let package_directories = vec![root.clone()];
    let files = classify_files_in_roots(&package_directories, &mut diagnostics);
    Ok(SfdxLayout {
        kind: LayoutKind::FileExtensionScan,
        workspace_root: root,
        package_directories,
        files,
        diagnostics,
    })
}

// -----------------------------------------------------------------------------
// Shared file-classification walk
// -----------------------------------------------------------------------------

/// Walk every entry in the given roots and classify it into a
/// [`ClassifiedFiles`] bucket. Skips `.git`, `node_modules`, and other
/// well-known noise directories (see [`SKIPPED_DIR_NAMES`]). Unreadable
/// directories surface as diagnostics rather than failing the walk.
fn classify_files_in_roots(roots: &[PathBuf], diagnostics: &mut Vec<String>) -> ClassifiedFiles {
    let mut files = ClassifiedFiles::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for root in roots {
        let walker = WalkDir::new(root)
            .max_depth(MAX_WALK_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                // Prune known-noise directories so we don't walk
                // millions of node_modules files. Files are never
                // filtered here — only dirs.
                if !entry.file_type().is_dir() {
                    return true;
                }
                match entry.file_name().to_str() {
                    Some(name) => !SKIPPED_DIR_NAMES.contains(&name),
                    None => true,
                }
            });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    diagnostics.push(format!(
                        "SFDX scan skipped an unreadable entry under {} ({e})",
                        root.display()
                    ));
                    continue;
                }
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path().to_path_buf();
            let canon = canonicalize_best_effort(&path);
            if !seen.insert(canon.clone()) {
                continue; // de-dupe symlinked or overlapping roots
            }

            classify_single(&canon, &mut files);
        }
    }

    files
}

/// Classify a single absolute path into its bucket. Multi-suffix files
/// (`*.cls-meta.xml`, `*.trigger-meta.xml`, `*.object-meta.xml`,
/// `*.field-meta.xml`) are matched *before* the bare extension case so
/// we never confuse `Foo.cls-meta.xml` for an Apex class source file.
fn classify_single(path: &Path, files: &mut ClassifiedFiles) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let lower = name.to_ascii_lowercase();

    // Compound suffixes first. The XML metadata files always use these
    // exact double-suffix forms; bare `.xml` is not enough.
    if lower.ends_with(".cls-meta.xml") {
        files.apex_class_meta.push(path.to_path_buf());
        return;
    }
    if lower.ends_with(".trigger-meta.xml") {
        files.apex_trigger_meta.push(path.to_path_buf());
        return;
    }
    if lower.ends_with(".object-meta.xml") {
        files.object_meta.push(path.to_path_buf());
        return;
    }
    if lower.ends_with(".field-meta.xml") {
        files.field_meta.push(path.to_path_buf());
        return;
    }

    // Bare source extensions.
    if lower.ends_with(".cls") {
        files.apex_classes.push(path.to_path_buf());
        return;
    }
    if lower.ends_with(".trigger") {
        files.apex_triggers.push(path.to_path_buf());
        return;
    }
    if lower.ends_with(".apxc") {
        files.apex_anonymous.push(path.to_path_buf());
        return;
    }
    // TR-A.5: Visualforce pages. Read via quick-xml by the VF pass,
    // not by tree-sitter. Classified alongside Apex sources so the
    // single SFDX walk surfaces every binding relevant to the
    // Apex-pipeline graph.
    if lower.ends_with(".page") {
        files.apex_vf_pages.push(path.to_path_buf());
    }
}

/// Best-effort canonicalization. Symlink resolution is a nicety, not a
/// requirement — on failure we fall back to the input path so the detector
/// still works on systems with sandboxed paths or removed parent dirs.
fn canonicalize_best_effort(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => c,
        Err(_) => p.to_path_buf(),
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build a minimal SFDX project tree inside `root`.
    fn write_sfdx_project(root: &Path) {
        fs::write(
            root.join("sfdx-project.json"),
            r#"{
                "packageDirectories": [
                    { "path": "force-app", "default": true }
                ],
                "name": "test-project",
                "namespace": "",
                "sfdcLoginUrl": "https://login.salesforce.com",
                "sourceApiVersion": "59.0"
            }"#,
        )
        .unwrap();
        let classes = root.join("force-app/main/default/classes");
        let triggers = root.join("force-app/main/default/triggers");
        let objects_acc = root.join("force-app/main/default/objects/Account");
        let objects_fields = objects_acc.join("fields");
        fs::create_dir_all(&classes).unwrap();
        fs::create_dir_all(&triggers).unwrap();
        fs::create_dir_all(&objects_fields).unwrap();

        fs::write(classes.join("Foo.cls"), "public class Foo {}").unwrap();
        fs::write(
            classes.join("Foo.cls-meta.xml"),
            r#"<?xml version="1.0"?><ApexClass/>"#,
        )
        .unwrap();
        fs::write(
            triggers.join("OnAccount.trigger"),
            "trigger OnAccount on Account (before insert) {}",
        )
        .unwrap();
        fs::write(
            triggers.join("OnAccount.trigger-meta.xml"),
            r#"<?xml version="1.0"?><ApexTrigger/>"#,
        )
        .unwrap();
        fs::write(
            objects_acc.join("Account.object-meta.xml"),
            r#"<?xml version="1.0"?><CustomObject/>"#,
        )
        .unwrap();
        fs::write(
            objects_fields.join("External_Id__c.field-meta.xml"),
            r#"<?xml version="1.0"?><CustomField/>"#,
        )
        .unwrap();
    }

    #[test]
    fn detects_authoritative_sfdx_from_root() {
        let tmp = tempfile::tempdir().unwrap();
        write_sfdx_project(tmp.path());

        let layout = detect(tmp.path()).unwrap();

        assert_eq!(layout.kind, LayoutKind::Sfdx);
        assert_eq!(
            layout.workspace_root,
            canonicalize_best_effort(tmp.path()),
            "workspace root should equal manifest directory"
        );
        assert_eq!(layout.package_directories.len(), 1);
        assert!(layout.package_directories[0].ends_with("force-app"));
        assert_eq!(layout.files.apex_classes.len(), 1);
        assert_eq!(layout.files.apex_class_meta.len(), 1);
        assert_eq!(layout.files.apex_triggers.len(), 1);
        assert_eq!(layout.files.apex_trigger_meta.len(), 1);
        assert_eq!(layout.files.object_meta.len(), 1);
        assert_eq!(layout.files.field_meta.len(), 1);
    }

    #[test]
    fn detects_authoritative_sfdx_from_deep_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        write_sfdx_project(tmp.path());
        let deep = tmp.path().join("force-app/main/default/classes");

        let layout = detect(&deep).unwrap();

        assert_eq!(layout.kind, LayoutKind::Sfdx);
        assert_eq!(
            layout.workspace_root,
            canonicalize_best_effort(tmp.path()),
            "ancestor walk must find manifest above the starting subdir"
        );
    }

    #[test]
    fn falls_back_to_force_app_heuristic_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let classes = tmp.path().join("force-app/main/default/classes");
        fs::create_dir_all(&classes).unwrap();
        fs::write(classes.join("Bar.cls"), "public class Bar {}").unwrap();

        let layout = detect(tmp.path()).unwrap();

        assert_eq!(layout.kind, LayoutKind::ForceAppHeuristic);
        assert_eq!(layout.package_directories.len(), 1);
        assert!(layout.package_directories[0].ends_with("force-app"));
        assert_eq!(layout.files.apex_classes.len(), 1);
        assert!(
            layout
                .diagnostics
                .iter()
                .any(|d| d.contains("force-app/main/default")),
            "force-app fallback must emit a diagnostic explaining why"
        );
    }

    #[test]
    fn detects_classic_mdapi_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let classes = tmp.path().join("classes");
        let triggers = tmp.path().join("triggers");
        let obj_dir = tmp.path().join("objects/MyObject__c");
        fs::create_dir_all(&classes).unwrap();
        fs::create_dir_all(&triggers).unwrap();
        fs::create_dir_all(&obj_dir).unwrap();

        fs::write(classes.join("Baz.cls"), "public class Baz {}").unwrap();
        fs::write(
            triggers.join("OnContact.trigger"),
            "trigger OnContact on Contact (after update) {}",
        )
        .unwrap();
        fs::write(
            obj_dir.join("MyObject__c.object-meta.xml"),
            r#"<?xml version="1.0"?><CustomObject/>"#,
        )
        .unwrap();

        let layout = detect(tmp.path()).unwrap();

        assert_eq!(layout.kind, LayoutKind::Mdapi);
        assert_eq!(layout.files.apex_classes.len(), 1);
        assert_eq!(layout.files.apex_triggers.len(), 1);
        assert_eq!(layout.files.object_meta.len(), 1);
    }

    #[test]
    fn last_resort_extension_scan_still_classifies() {
        let tmp = tempfile::tempdir().unwrap();
        // No sfdx-project.json, no force-app, no classes/ — just Apex
        // files scattered at odd depths.
        let weird = tmp.path().join("a/b/c");
        fs::create_dir_all(&weird).unwrap();
        fs::write(weird.join("Lonely.cls"), "public class Lonely {}").unwrap();

        let layout = detect(tmp.path()).unwrap();

        assert_eq!(layout.kind, LayoutKind::FileExtensionScan);
        assert_eq!(layout.files.apex_classes.len(), 1);
        assert!(
            layout
                .diagnostics
                .iter()
                .any(|d| d.contains("extension-only")),
            "extension-only fallback must emit a clear diagnostic"
        );
    }

    #[test]
    fn empty_directory_returns_diagnostic_and_empty_files() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = detect(tmp.path()).unwrap();

        assert_eq!(layout.kind, LayoutKind::FileExtensionScan);
        assert!(layout.files.is_empty());
        assert!(!layout.diagnostics.is_empty());
    }

    #[test]
    fn missing_package_directory_produces_diagnostic_and_degrades_gracefully() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sfdx-project.json"),
            r#"{ "packageDirectories": [ { "path": "does-not-exist" } ] }"#,
        )
        .unwrap();
        // Add a class directly under root so the extension scan still has content.
        fs::write(tmp.path().join("Orphan.cls"), "public class Orphan {}").unwrap();

        let layout = detect(tmp.path()).unwrap();

        assert_eq!(layout.kind, LayoutKind::Sfdx);
        assert!(
            layout
                .diagnostics
                .iter()
                .any(|d| d.contains("does-not-exist")),
            "missing package dir must be surfaced verbatim"
        );
        // With every declared package missing we degrade to workspace
        // root; the orphan class must still be picked up.
        assert_eq!(layout.files.apex_classes.len(), 1);
    }

    #[test]
    fn malformed_sfdx_project_json_does_not_poison_detection() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("sfdx-project.json"), "{{ not json").unwrap();
        // Add a force-app layout so the detector can succeed via heuristic.
        let classes = tmp.path().join("force-app/main/default/classes");
        fs::create_dir_all(&classes).unwrap();
        fs::write(classes.join("Ok.cls"), "public class Ok {}").unwrap();

        let layout = detect(tmp.path()).unwrap();

        // Malformed manifest must NOT be treated as authoritative.
        assert_eq!(layout.kind, LayoutKind::ForceAppHeuristic);
        assert!(
            layout
                .diagnostics
                .iter()
                .any(|d| d.contains("not valid JSON") || d.contains("structurally invalid"))
                || layout.diagnostics.iter().any(|d| d.contains("force-app"))
        );
        assert_eq!(layout.files.apex_classes.len(), 1);
    }

    #[test]
    fn classify_single_distinguishes_meta_from_source() {
        let mut files = ClassifiedFiles::default();
        classify_single(Path::new("/x/Foo.cls"), &mut files);
        classify_single(Path::new("/x/Foo.cls-meta.xml"), &mut files);
        classify_single(Path::new("/x/Bar.trigger"), &mut files);
        classify_single(Path::new("/x/Bar.trigger-meta.xml"), &mut files);
        classify_single(Path::new("/x/Acc.object-meta.xml"), &mut files);
        classify_single(Path::new("/x/f.field-meta.xml"), &mut files);
        classify_single(Path::new("/x/Anon.apxc"), &mut files);
        classify_single(Path::new("/x/ignored.md"), &mut files);

        assert_eq!(files.apex_classes.len(), 1);
        assert_eq!(files.apex_class_meta.len(), 1);
        assert_eq!(files.apex_triggers.len(), 1);
        assert_eq!(files.apex_trigger_meta.len(), 1);
        assert_eq!(files.object_meta.len(), 1);
        assert_eq!(files.field_meta.len(), 1);
        assert_eq!(files.apex_anonymous.len(), 1);
    }

    #[test]
    fn walker_skips_known_noise_directories() {
        let tmp = tempfile::tempdir().unwrap();
        write_sfdx_project(tmp.path());

        // Drop a bogus .cls file inside node_modules — must NOT be picked up.
        let noise = tmp.path().join("force-app/node_modules");
        fs::create_dir_all(&noise).unwrap();
        fs::write(
            noise.join("EvilShouldBeIgnored.cls"),
            "public class Evil {}",
        )
        .unwrap();

        // Also drop one inside .git — must NOT be picked up.
        let git_noise = tmp.path().join(".git/objects");
        fs::create_dir_all(&git_noise).unwrap();
        fs::write(git_noise.join("AlsoEvil.cls"), "public class AlsoEvil {}").unwrap();

        let layout = detect(tmp.path()).unwrap();
        let names: Vec<String> = layout
            .files
            .apex_classes
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(
            !names.iter().any(|n| n.contains("Evil")),
            "classes inside node_modules/.git must be pruned, got: {names:?}"
        );
        assert!(names.iter().any(|n| n == "Foo.cls"));
    }

    #[test]
    fn workspace_url_is_file_scheme() {
        let tmp = tempfile::tempdir().unwrap();
        write_sfdx_project(tmp.path());
        let layout = detect(tmp.path()).unwrap();
        let url = layout.workspace_url().unwrap();
        assert_eq!(url.scheme(), "file");
    }

    #[test]
    fn nonexistent_input_is_a_hard_error() {
        let err = detect(Path::new("/this/does/not/exist/anywhere_12345"))
            .expect_err("nonexistent input must error, not silently produce empty layout");
        let msg = format!("{err}");
        assert!(msg.contains("existing path"), "got: {msg}");
    }

    #[test]
    fn sfdx_layout_summary_is_one_line() {
        let tmp = tempfile::tempdir().unwrap();
        write_sfdx_project(tmp.path());
        let layout = detect(tmp.path()).unwrap();
        let summary = layout.summary();
        assert!(summary.starts_with("Apex layout: sfdx"));
        assert!(!summary.contains('\n'));
    }
}
