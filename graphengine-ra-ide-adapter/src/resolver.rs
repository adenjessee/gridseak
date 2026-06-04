//! The Layer 2 semantic resolver itself. Owns the `AnalysisHost`
//! and the `Vfs`, translates [`SemanticQueryInput`] coordinates into
//! rust-analyzer's `FilePosition`, and dispatches
//! `Analysis::goto_definition`.
//!
//! ## UF-FU-009 resolution (constructor path)
//!
//! The P5 Phase B.B2 spike flagged that `ra_ap_ide 0.0.307`'s
//! `AnalysisHost` surface did not obviously expose a constructor that
//! wraps a `RootDatabase` returned by `ra_ap_load_cargo::load_workspace_at`.
//! The two candidate paths were:
//!
//! - **(a) Drive queries directly off `RootDatabase`.** Depends on the
//!   salsa-db trait impls being publicly reachable through `RootDatabase`
//!   alone; otherwise we cannot call `goto_definition` without an
//!   `Analysis` wrapper.
//! - **(b) Bootstrap a fresh `AnalysisHost::default()` + mirror the
//!   `load_workspace_at` change application.** Always possible but
//!   doubles the cold workspace-load cost.
//!
//! T6 implementation landed on a third, simpler option the spike had
//! already half-documented: **`AnalysisHost::with_database(db)`** is a
//! publicly re-exported constructor in `ra_ap_ide = 0.0.307`. Verified
//! by compilation on this crate and by `cargo test -p
//! graphengine-ra-ide-adapter`. Option (a) was ruled out — `Analysis`
//! queries are only reachable via `host.analysis()`; the query traits
//! on `RootDatabase` are salsa-internal. Option (b) was not needed;
//! it stays documented as a known fallback if a future `ra_ap_ide`
//! release removes `with_database`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ra_ap_ide::{AnalysisHost, FilePosition, GotoDefinitionConfig, LineCol, LineIndex};
use ra_ap_ide_db::MiniCore;
use ra_ap_load_cargo::{load_workspace_at, LoadCargoConfig, ProcMacroServerChoice};
use ra_ap_paths::AbsPathBuf;
use ra_ap_project_model::CargoConfig;
use ra_ap_vfs::{Vfs, VfsPath};
use thiserror::Error;
use tracing::{debug, warn};

use crate::query::SemanticQueryInput;

/// Structured errors the adapter emits. Every variant represents a
/// **named, observable** failure mode; callers wire each into the
/// engine's integrity-caveat surface in PR #2. Never silently
/// swallowed, never converted to a bare `Option::None`.
#[derive(Debug, Error)]
pub enum SemanticResolverError {
    /// `cargo metadata` / `load_workspace_at` failed. Root cause lives
    /// in the inner string (usually "Cargo.toml missing", "invalid
    /// manifest", "sysroot discovery failed"). Callers surface this as
    /// `CAVEAT_SEMANTIC_RESOLVER_PROJECT_MODEL_MISSING_V1` or
    /// `_INVALID_V1` per T6 §5.3.
    #[error("project-model load failed: {0}")]
    ProjectModelLoad(String),

    /// The input file was not in the loaded VFS. Usually means the
    /// caller passed a path outside the workspace the adapter was
    /// constructed with (e.g. a scratch file with no enclosing
    /// `Cargo.toml`). Fall through to heuristic resolution.
    #[error("file not tracked by project model: {0}")]
    FileNotInProjectModel(PathBuf),

    /// `goto_definition` returned multiple candidates and we refuse
    /// to pick one. T6 §2 non-goal: "laundering low-authority data
    /// into High." Callers downgrade to `Medium` per T6 §6.1
    /// `ambiguous_reference_downgrades_to_medium`.
    #[error("ambiguous reference: {candidates} candidates")]
    AmbiguousReference { candidates: usize },

    /// The reference sits inside a proc-macro-expanded body. Known
    /// limitation; tracked as `UF-FU-003`. Callers fall through to
    /// heuristic.
    #[error("proc-macro expansion not supported")]
    MacroExpansionUnsupported,

    /// rust-analyzer's `Analysis` handle was cancelled or the line
    /// index could not be built (usually indicates the file content
    /// changed mid-query, which does not happen in our one-shot
    /// scan flow but is a real variant in the upstream library).
    #[error("analysis snapshot error: {0}")]
    AnalysisSnapshot(String),
}

/// Confidence grade the adapter attaches to a resolved target. The
/// `graphengine-parsing` wiring layer (PR #2) maps this onto the
/// engine's full `Confidence` enum. Kept as an adapter-local type so
/// this crate stays free of engine domain imports per the T6 §4
/// "Chosen shape" architectural contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// `ra_ap_ide` returned exactly one unambiguous target.
    High,
    /// `ra_ap_ide` returned a target but one or more siblings were
    /// flagged as plausible (e.g. trait-method dispatch with multiple
    /// impl candidates). Caller should emit `Provenance { Lsp,
    /// Medium }`, not `High`.
    Medium,
}

/// Final answer the adapter returns for a single query. Minimal
/// shape — file/line/column/symbol-name — so this crate does not need
/// to model the engine's node-ID scheme.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    /// Absolute file path of the definition.
    pub target_file: PathBuf,
    /// 1-based line of the definition.
    pub target_line: u32,
    /// 0-based column (byte offset within the line) of the definition.
    pub target_column: u32,
    /// The symbol name rust-analyzer attached to the definition.
    /// Usually the function/method identifier; the wiring layer uses
    /// it to disambiguate when the engine's own `NodeKind` disagrees
    /// with rust-analyzer's classification.
    pub target_symbol_name: String,
    /// Confidence grade — see [`Confidence`].
    pub confidence: Confidence,
}

/// The resolver owns one `AnalysisHost` per workspace and runs
/// `goto_definition` queries against it. Construction is expensive
/// (cold `load_workspace_at` on `gridseak-self` is the B3 baseline
/// floor at 135 ms for the 2-file fixture; real workspaces scale with
/// crate count). Per-query cost is cheap — a salsa-memoised snapshot.
///
/// The struct is `Send` but not `Sync`; rust-analyzer snapshots are
/// thread-local by construction. The wiring layer (PR #2) keeps one
/// resolver per scan task and drives it serially.
pub struct RustAnalyzerSemanticResolver {
    host: AnalysisHost,
    vfs: Vfs,
    workspace_root: PathBuf,
    /// Cold-load elapsed, captured for the post-ship `UF-FU-011`
    /// production-file-filter re-measurement and the T6 §5.5
    /// wall-clock kill criterion.
    load_elapsed_ms: u128,
}

impl RustAnalyzerSemanticResolver {
    /// Load a Cargo workspace at `root` and return a resolver that
    /// can serve `goto_definition` queries against it.
    ///
    /// `root` may point at either a directory containing a
    /// `Cargo.toml` or at the `Cargo.toml` itself; both forms are
    /// accepted because callers upstream mix them. The path is
    /// canonicalised before handing to `load_workspace_at` so
    /// relative paths do not silently miss sysroot discovery.
    pub fn from_workspace_root(root: &Path) -> Result<Self, SemanticResolverError> {
        let canonical_root = crate::canonicalize_or_original(root);
        let cargo_toml = if canonical_root.is_file() {
            canonical_root.clone()
        } else {
            canonical_root.join("Cargo.toml")
        };

        if !cargo_toml.exists() {
            return Err(SemanticResolverError::ProjectModelLoad(format!(
                "Cargo.toml not found at {}",
                cargo_toml.display()
            )));
        }

        let cargo_config = CargoConfig::default();
        let load_config = LoadCargoConfig {
            load_out_dirs_from_check: false,
            with_proc_macro_server: ProcMacroServerChoice::None,
            prefill_caches: false,
        };

        let t0 = Instant::now();
        let progress = |msg: String| {
            debug!(target: "ra_ap_ide_adapter::load", "{}", msg);
        };
        let (db, vfs, _proc_macro) =
            load_workspace_at(&cargo_toml, &cargo_config, &load_config, &progress)
                .map_err(|e| SemanticResolverError::ProjectModelLoad(e.to_string()))?;
        let load_elapsed_ms = t0.elapsed().as_millis();

        // UF-FU-009: option (b) superseded — `AnalysisHost::with_database`
        // is public on `ra_ap_ide = 0.0.307`. Verified by compilation.
        let host = AnalysisHost::with_database(db);

        let workspace_root = cargo_toml
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(canonical_root);

        Ok(Self {
            host,
            vfs,
            workspace_root,
            load_elapsed_ms,
        })
    }

    /// The number of milliseconds `from_workspace_root` spent inside
    /// `load_workspace_at`. Exposed for the dogfood CI job and the
    /// UF-FU-011 re-measurement.
    pub fn load_elapsed_ms(&self) -> u128 {
        self.load_elapsed_ms
    }

    /// Absolute path of the workspace the resolver was built against.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Run a single `goto_definition` query. Returns:
    /// - `Ok(Some(target))` when rust-analyzer returned exactly one,
    ///   or ≥ 1 with a single best candidate.
    /// - `Ok(None)` when rust-analyzer found no target. Callers fall
    ///   through to the heuristic resolver.
    /// - `Err(...)` on any named failure (see
    ///   [`SemanticResolverError`]).
    ///
    /// Never panics. Every failure surfaces as a `Result` error.
    pub fn resolve(
        &self,
        input: &SemanticQueryInput,
    ) -> Result<Option<ResolvedTarget>, SemanticResolverError> {
        let file_id = self.vfs_file_id_for(&input.file)?;
        let analysis = self.host.analysis();

        let line_index: LineIndex = analysis
            .file_line_index(file_id)
            .map_err(|e| SemanticResolverError::AnalysisSnapshot(format!("line index: {e:?}")))?
            .as_ref()
            .clone();

        let offset = line_index.offset(LineCol {
            line: input.line.saturating_sub(1),
            col: input.column,
        });
        let Some(offset) = offset else {
            warn!(
                target: "ra_ap_ide_adapter::resolve",
                "line/column outside file bounds: {}:{}:{}",
                input.file.display(),
                input.line,
                input.column,
            );
            return Ok(None);
        };

        // `LineIndex::offset` only validates that the LINE exists — it computes
        // `line_start + col` without checking that `col` stays within that
        // line, so a column that overruns its line (e.g. the `+1` dot-skip in
        // `caret_for_callee`, or any byte-for-byte divergence between the text
        // tree-sitter parsed and the text rust-analyzer's VFS loaded) yields an
        // absolute offset past EOF. Passing that to `goto_definition` reaches
        // `rowan::token_at_offset`, which panics ("Bad offset") and takes down
        // the whole parser sidecar. We promise this resolver never panics and
        // the wiring layer relies on that to fall through to the heuristic
        // resolver, so we bound-check the offset here at the boundary.
        let file_len = analysis
            .file_text(file_id)
            .map_err(|e| SemanticResolverError::AnalysisSnapshot(format!("file text: {e:?}")))?
            .len();
        if !offset_within_file_bounds(usize::from(offset), file_len) {
            warn!(
                target: "ra_ap_ide_adapter::resolve",
                "computed offset {} past EOF ({} bytes) for {}:{}:{}; LineIndex::offset \
                 validated the line but not the column — falling back to heuristic resolver",
                u32::from(offset),
                file_len,
                input.file.display(),
                input.line,
                input.column,
            );
            return Ok(None);
        }

        let position = FilePosition { file_id, offset };
        // `GotoDefinitionConfig` carries one field — `minicore` — that
        // lets rust-analyzer inject a synthetic core/std source for
        // test fixtures. Production scans always want the real
        // sysroot, which `load_workspace_at` has already wired in, so
        // we pass the library's own default.
        let config = GotoDefinitionConfig {
            minicore: MiniCore::default(),
        };
        let nav_info = analysis.goto_definition(position, &config).map_err(|e| {
            SemanticResolverError::AnalysisSnapshot(format!("goto_definition: {e:?}"))
        })?;

        let Some(nav_info) = nav_info else {
            return Ok(None);
        };

        let candidates: Vec<_> = nav_info.info.into_iter().collect();
        if candidates.is_empty() {
            return Ok(None);
        }

        let confidence = if candidates.len() == 1 {
            Confidence::High
        } else {
            // Ambiguous reference — emit the first candidate but
            // downgrade to Medium per T6 §6.1
            // `ambiguous_reference_downgrades_to_medium`. The
            // wiring layer's `UF-FU-012` (if filed) captures any
            // population-level drift this introduces.
            Confidence::Medium
        };

        let nav = &candidates[0];
        let target_file = self.vfs_path_for(nav.file_id).ok_or_else(|| {
            SemanticResolverError::AnalysisSnapshot(format!(
                "target file_id {:?} not in VFS",
                nav.file_id
            ))
        })?;

        // rust-analyzer returns the full navigation range for the
        // target (covering the whole item body). We want just the
        // definition site, which is `focus_range` when available,
        // falling back to the full range.
        let def_range = nav.focus_range.unwrap_or(nav.full_range);

        let target_line_index: LineIndex = analysis
            .file_line_index(nav.file_id)
            .map_err(|e| {
                SemanticResolverError::AnalysisSnapshot(format!("target line index: {e:?}"))
            })?
            .as_ref()
            .clone();
        let line_col = target_line_index.line_col(def_range.start());

        let target_symbol_name = nav.name.as_str().to_string();

        Ok(Some(ResolvedTarget {
            target_file,
            target_line: line_col.line + 1,
            target_column: line_col.col,
            target_symbol_name,
            confidence,
        }))
    }

    /// Translate a filesystem path into a `ra_ap_vfs::FileId`. The VFS
    /// uses `AbsPathBuf` keys; if the input path is not absolute we
    /// canonicalise first and short-circuit to
    /// [`SemanticResolverError::FileNotInProjectModel`] if
    /// canonicalisation fails.
    ///
    /// `Vfs::file_id` returns `(FileId, FileExcluded)`; we discard the
    /// `FileExcluded` flag because rust-analyzer's exclusion list
    /// (e.g. auto-generated files, vendored crates) is not a
    /// semantic-authority signal we want to surface. The heuristic
    /// fallback already re-admits those files at a lower confidence.
    fn vfs_file_id_for(&self, path: &Path) -> Result<ra_ap_vfs::FileId, SemanticResolverError> {
        let canonical = path
            .canonicalize()
            .map_err(|_| SemanticResolverError::FileNotInProjectModel(path.to_path_buf()))?;
        let abs = AbsPathBuf::assert_utf8(canonical.clone());
        let vfs_path = VfsPath::from(abs);
        match self.vfs.file_id(&vfs_path) {
            Some((file_id, _excluded)) => Ok(file_id),
            None => Err(SemanticResolverError::FileNotInProjectModel(canonical)),
        }
    }

    /// Reverse lookup: turn a `ra_ap_vfs::FileId` back into its
    /// absolute filesystem path. Used when rendering
    /// [`ResolvedTarget::target_file`]. Returns `None` only for
    /// virtual-path VFS entries (e.g. sysroot injected sources);
    /// callers treat that as "target out of project scope" and fall
    /// through to heuristic.
    fn vfs_path_for(&self, file_id: ra_ap_vfs::FileId) -> Option<PathBuf> {
        let vfs_path = self.vfs.file_path(file_id);
        // `AbsPath: AsRef<Path>` gives us the std `Path` without going
        // through the (non-existent) `as_std_path` method.
        vfs_path.as_path().map(|p| {
            let std_path: &Path = p.as_ref();
            std_path.to_path_buf()
        })
    }
}

/// Whether a byte `offset` is a position rust-analyzer can safely turn into
/// a [`FilePosition`].
///
/// `LineIndex::offset` validates that the *line* exists but computes
/// `line_start + col` without checking the column stays within that line, so
/// an overrunning column (or any divergence between the text tree-sitter
/// parsed and the text rust-analyzer's VFS loaded) yields an offset past EOF.
/// An offset exactly at `file_len` is the valid end-of-file caret; only a
/// strictly-greater offset is out of bounds and would panic
/// `rowan::token_at_offset` ("Bad offset") downstream.
fn offset_within_file_bounds(offset: usize, file_len: usize) -> bool {
    offset <= file_len
}

#[cfg(test)]
mod resolver_internal_tests {
    use super::*;

    /// Regression guard for the rowan "Bad offset" scan panic: an offset at
    /// EOF is valid, one byte past EOF is not. The bound is `<=`, not `<`.
    #[test]
    fn offset_bounds_reject_only_past_eof() {
        assert!(offset_within_file_bounds(0, 10), "start of file is valid");
        assert!(offset_within_file_bounds(10, 10), "exactly EOF is valid");
        assert!(
            !offset_within_file_bounds(11, 10),
            "one byte past EOF must be rejected (this is the panic case)"
        );
        assert!(offset_within_file_bounds(0, 0), "empty file: EOF == 0");
        assert!(
            !offset_within_file_bounds(1, 0),
            "empty file: 1 is past EOF"
        );
    }

    /// Exhaustiveness smoke test: every `SemanticResolverError`
    /// variant round-trips through its `Display` impl without panic.
    /// Mirrors T6 §6.1 `semantic_resolver_error_variants_are_exhaustive`.
    #[test]
    fn semantic_resolver_error_variants_are_exhaustive() {
        let variants = [
            SemanticResolverError::ProjectModelLoad("x".into()),
            SemanticResolverError::FileNotInProjectModel(PathBuf::from("/x")),
            SemanticResolverError::AmbiguousReference { candidates: 2 },
            SemanticResolverError::MacroExpansionUnsupported,
            SemanticResolverError::AnalysisSnapshot("x".into()),
        ];
        for v in variants {
            let s = format!("{v}");
            assert!(!s.is_empty(), "Display produced empty string for {v:?}");
        }
    }

    #[test]
    fn confidence_high_is_distinct_from_medium() {
        assert_ne!(Confidence::High, Confidence::Medium);
    }

    #[test]
    fn from_workspace_root_errors_on_missing_cargo_toml() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        match RustAnalyzerSemanticResolver::from_workspace_root(root) {
            Ok(_) => panic!("expected an error, got Ok"),
            Err(SemanticResolverError::ProjectModelLoad(msg)) => {
                assert!(msg.contains("Cargo.toml"), "unexpected message: {msg}");
            }
            Err(other) => panic!("expected ProjectModelLoad, got {other:?}"),
        }
    }
}
