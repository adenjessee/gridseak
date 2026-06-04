//! Apex-specific LSP session wiring (Sprint F.2).
//!
//! Given a workspace root URL, this module:
//!
//! 1. Runs the SFDX layout detector to discover every declared
//!    `packageDirectories` entry (authoritative manifest path) or
//!    the conventional `force-app/main/default` / MDAPI layout.
//! 2. Builds one [`WorkspaceFolder`] per package directory, each
//!    anchored at an existing on-disk path. Jorje indexes only the
//!    advertised folders, so losing a package here equals losing all
//!    classes in it from LSP-backed resolution — we emit every
//!    declared folder.
//! 3. Picks a "canary" Apex source file — preferring an in-package
//!    `.cls` — to be used as the F.2 readiness probe target.
//! 4. Assembles a [`ReadinessStrategy::ProgressAndProbe`] with
//!    sensible defaults (60s deadline, 3s quiet period). The
//!    deadline matches the `lsp_indexing_timeout_secs` knob that
//!    will eventually be exposed via YAML; for now we keep it
//!    hard-coded with a single source of truth here.
//!
//! The caller (`ParseRepositoryUseCase::factory`) wires the returned
//! [`SessionOptions`] into `LspResolver::with_options`.

use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};
use url::Url;

use crate::infrastructure::lsp::definition_provider::SessionOptions;
use crate::infrastructure::lsp::protocol::WorkspaceFolder;
use crate::infrastructure::lsp::session::ReadinessStrategy;
use crate::syntax::language::apex::sfdx_layout::{self, SfdxLayout};

/// Default readiness-barrier total deadline. Jorje on a warm JVM
/// indexes apex-recipes (~60 classes) in ~10s and NPSP (~3k classes)
/// in ~45s on the pilot hardware. 60s covers both with some margin;
/// anything larger should opt into this knob explicitly via a
/// future `LanguageConfig.lsp_indexing_timeout_secs`.
const DEFAULT_DEADLINE: Duration = Duration::from_secs(60);

/// Default quiet period after *at least one* indexing signal before
/// we consider the server ready. A couple of seconds is enough to
/// absorb a staggered `$/progress` stream without waiting on a
/// never-arriving `end` frame.
const DEFAULT_QUIET: Duration = Duration::from_secs(3);

/// Build [`SessionOptions`] appropriate for an Apex scan anchored at
/// `workspace_root`. Falls back to sane defaults on any detection
/// failure (empty folders, `Immediate` readiness) so the LSP session
/// still starts — that's the pre-F.2 behaviour and is never worse
/// than the status quo.
pub fn build_apex_session_options(workspace_root: Option<&Url>) -> SessionOptions {
    let Some(root_url) = workspace_root else {
        debug!("Apex session options: no workspace_root provided; using defaults");
        return SessionOptions::default();
    };
    let Ok(root_path) = root_url.to_file_path() else {
        warn!(
            "Apex session options: workspace_root {} is not a file path; using defaults",
            root_url
        );
        return SessionOptions::default();
    };

    let layout = match sfdx_layout::detect(&root_path) {
        Ok(l) => l,
        Err(err) => {
            warn!(
                "Apex session options: SFDX detection failed at {}: {}. \
                 Falling back to Immediate readiness strategy.",
                root_path.display(),
                err
            );
            return SessionOptions::default();
        }
    };

    let folders = layout_to_workspace_folders(&layout);
    let canary = pick_canary_file(&layout);
    let readiness = ReadinessStrategy::ProgressAndProbe {
        canary_file: canary.clone(),
        deadline: DEFAULT_DEADLINE,
        quiet_period: DEFAULT_QUIET,
    };

    info!(
        layout = layout.kind.as_str(),
        folders = folders.len(),
        canary = ?canary,
        "Apex session options built"
    );

    SessionOptions {
        workspace_folders: folders,
        readiness,
    }
}

/// Emit one [`WorkspaceFolder`] per declared package directory. Each
/// folder's `name` is the final path component (e.g. `force-app`),
/// which is what jorje's log lines reference — making trace output
/// easier to correlate.
fn layout_to_workspace_folders(layout: &SfdxLayout) -> Vec<WorkspaceFolder> {
    let mut out = Vec::with_capacity(layout.package_directories.len());
    for pkg in &layout.package_directories {
        if !pkg.exists() {
            // `detect()` already filters these out, but defend in
            // depth — a later detection change must not accidentally
            // send a nonexistent folder to the server.
            continue;
        }
        match Url::from_directory_path(pkg) {
            Ok(uri) => {
                let name = pkg
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "package".to_string());
                out.push(WorkspaceFolder {
                    uri: uri.to_string(),
                    name,
                });
            }
            Err(_) => {
                warn!(
                    "Apex session options: could not URL-encode package dir {}; skipping",
                    pkg.display()
                );
            }
        }
    }
    out
}

/// Pick a canary file we'll `documentSymbol`-probe as the readiness
/// barrier. The first `.cls` under any declared package wins (Apex
/// classes are the canonical jorje payload); if there are only
/// triggers, fall through to the first trigger; if neither, `None`.
fn pick_canary_file(layout: &SfdxLayout) -> Option<String> {
    layout
        .files
        .apex_classes
        .iter()
        .chain(layout.files.apex_triggers.iter())
        .find_map(|p| path_to_file_uri(p))
}

fn path_to_file_uri(p: &Path) -> Option<String> {
    Url::from_file_path(p).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_minimal_sfdx(root: &Path) {
        fs::write(
            root.join("sfdx-project.json"),
            r#"{"packageDirectories":[{"path":"force-app","default":true}]}"#,
        )
        .unwrap();
        let classes = root.join("force-app/main/default/classes");
        fs::create_dir_all(&classes).unwrap();
        fs::write(classes.join("Foo.cls"), "public class Foo {}").unwrap();
    }

    #[test]
    fn build_from_real_sfdx_layout_yields_folders_and_canary() {
        let tmp = tempdir().unwrap();
        write_minimal_sfdx(tmp.path());
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_apex_session_options(Some(&root));
        assert_eq!(opts.workspace_folders.len(), 1);
        assert!(opts.workspace_folders[0].uri.ends_with("force-app/"));
        match &opts.readiness {
            ReadinessStrategy::ProgressAndProbe { canary_file, .. } => {
                let c = canary_file
                    .as_deref()
                    .expect("expected a canary for a layout containing Foo.cls");
                assert!(c.ends_with("Foo.cls"), "canary was {c}");
            }
            other => panic!("expected ProgressAndProbe, got {other:?}"),
        }
    }

    #[test]
    fn build_without_workspace_root_returns_defaults() {
        let opts = build_apex_session_options(None);
        assert!(opts.workspace_folders.is_empty());
        assert!(
            matches!(opts.readiness, ReadinessStrategy::Immediate),
            "no workspace root must degrade to Immediate readiness, \
             preserving pre-F.2 behaviour"
        );
    }

    #[test]
    fn empty_directory_yields_defaults_but_not_a_panic() {
        let tmp = tempdir().unwrap();
        let root = Url::from_directory_path(tmp.path()).unwrap();
        let opts = build_apex_session_options(Some(&root));
        // Extension-scan layout with zero files: folders list is the
        // scan root but canary is None.
        match &opts.readiness {
            ReadinessStrategy::ProgressAndProbe { canary_file, .. } => {
                assert!(
                    canary_file.is_none(),
                    "empty layout should not invent a canary"
                );
            }
            other => panic!("expected ProgressAndProbe, got {other:?}"),
        }
    }

    #[test]
    fn multi_package_sfdx_emits_every_folder() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("sfdx-project.json"),
            r#"{"packageDirectories":[
                {"path":"packageA","default":true},
                {"path":"packageB"}
            ]}"#,
        )
        .unwrap();
        for pkg in ["packageA", "packageB"] {
            let dir = tmp.path().join(pkg).join("classes");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(format!("{pkg}_Main.cls")), "public class X {}").unwrap();
        }
        let root = Url::from_directory_path(tmp.path()).unwrap();

        let opts = build_apex_session_options(Some(&root));
        let names: Vec<_> = opts
            .workspace_folders
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["packageA", "packageB"],
            "every declared SFDX package must appear as a workspaceFolder"
        );
    }
}
