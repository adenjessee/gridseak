//! The commit-walk extractor itself.
//!
//! Execution flow for [`GitSignalExtractor::extract`]:
//!
//! 1. Classify the repository shape via
//!    [`crate::RepoShape::detect`]. Non-git paths short-circuit
//!    with [`crate::GitSignalReport::non_git`].
//! 2. Open the repository via `gix::open`. Failures at this stage
//!    are [`crate::OpenError`]; the caller decides whether to
//!    degrade gracefully or bubble.
//! 3. Walk commits reachable from HEAD in newest-first commit-time
//!    order, bounded by [`crate::HistoryWindow`]. Each commit
//!    contributes a diff-by-file against its first parent (or the
//!    empty tree for root commits). Merge commits are skipped —
//!    following both parents would double-count ordinary changes.
//!    This is the same heuristic `git log --no-merges
//!    --first-parent` produces and matches what developers mentally
//!    model as "the mainline history."
//! 4. Per-file [`crate::FileSignals`] accumulate from the per-commit
//!    diffs; per-commit `changed_files` sets feed co-change
//!    aggregation.
//! 5. After the walk, apply the **shallow-clone guard**: if the
//!    repository shape is anything other than
//!    [`crate::RepoShape::Full`], every `FileSignals.confidence` is
//!    forced to [`crate::Confidence::Low`] regardless of numeric
//!    signal strength. This is the single load-bearing invariant
//!    the crate exists to enforce.
//!
//! The extractor deliberately does **not** hold a
//! [`gix::Repository`] across `.await` points. `gix::Repository` is
//! `Send` but not `Sync`; all gix work happens synchronously inside
//! `extract`, and the extractor itself stores only the already-opened
//! `gix::Repository` plus the filesystem path.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gix::bstr::ByteSlice;
use gix::object::tree::diff::Change;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;
use tracing::{debug, warn};

use crate::caveats::{
    CAVEAT_LAYER0_GIT_SIGNALS_V1, CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1,
    CAVEAT_LAYER0_UNSUPPORTED_VCS_V1,
};
use crate::errors::{ExtractError, OpenError};
use crate::ownership::ownership_dispersion;
use crate::report::{CoChangeCluster, FileSignals, GitSignalReport};
use crate::{Confidence, HistoryWindow, RepoShape};

/// Minimum co-occurrence threshold for emitting a [`CoChangeCluster`]
/// from a `Full`-shape repo. Shallow-clone repos emit clusters at
/// threshold `1` (single commit available) but with
/// [`Confidence::Low`]; see [`assemble_clusters`] for the branching.
const CO_CHANGE_MIN_FULL: u32 = 3;

/// Maximum arity of a single co-change cluster. A commit touching
/// 200 files (rename sweeps, license-header updates, CI-config
/// rollouts) would otherwise produce a combinatorial explosion in
/// the pair-count map. We cap the cluster size to the top-N
/// most-frequent-together files per commit before aggregation.
const MAX_FILES_PER_COMMIT_FOR_CLUSTERING: usize = 50;

/// Top-level extractor. Constructed with an opened `gix::Repository`
/// plus the filesystem path it was opened from; every subsequent
/// [`extract`] call reuses the repository handle.
pub struct GitSignalExtractor {
    repo_root: PathBuf,
    repo_shape: RepoShape,
    repository: Option<gix::Repository>,
}

impl GitSignalExtractor {
    /// Open a git repository at `repo_root`. The shape is classified
    /// up-front so the caller can decide whether to call
    /// [`extract`] or treat the extractor as a placeholder for a
    /// non-git path.
    ///
    /// Returns `Err` only for the "path is unreadable" case; non-git
    /// paths are **not** errors — they return a placeholder
    /// extractor whose [`extract`] yields
    /// [`GitSignalReport::non_git`]. Distinguishing the two at
    /// construction time matches the sprint's measured-fallback
    /// discipline: don't error on absence, report absence.
    pub fn open(repo_root: &Path) -> Result<Self, OpenError> {
        let metadata = std::fs::metadata(repo_root).map_err(|err| OpenError::PathNotReadable {
            path: repo_root.to_path_buf(),
            source: err,
        })?;
        if !metadata.is_dir() {
            return Err(OpenError::NotAGitRepository {
                path: repo_root.to_path_buf(),
            });
        }

        let repo_shape = RepoShape::detect(repo_root);
        let repository = match repo_shape {
            RepoShape::NonGit => None,
            _ => match gix::open(repo_root) {
                Ok(mut repo) => {
                    // Cap the object cache. gix defaults to an
                    // uncapped-ish memory-capped hashmap whose
                    // steady-state footprint is dominated by every
                    // decoded commit + tree kept resident; on
                    // `gridseak-self` this pushed peak RSS past 280
                    // MB for 47 commits walked. A 16 MB cap is
                    // ample for the first-parent single-pass walk
                    // (we never revisit a commit or tree) and keeps
                    // the extractor within the stated T7 §5.7
                    // budget of "< 50 MB over baseline". Setting
                    // the cache to `None` (fully disabled) slowed
                    // the walk noticeably because adjacent commits
                    // share parent trees that get re-decoded;
                    // 16 MB is the empirical sweet-spot.
                    repo.object_cache_size(Some(16 * 1024 * 1024));
                    Some(repo)
                }
                Err(err) => {
                    return Err(OpenError::GixOpenFailed {
                        path: repo_root.to_path_buf(),
                        message: err.to_string(),
                    });
                }
            },
        };

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            repo_shape,
            repository,
        })
    }

    /// Classification of the repository as determined at open time.
    pub fn repo_shape(&self) -> RepoShape {
        self.repo_shape
    }

    /// Walk the repository with `window` and emit a report. See the
    /// module doc for the execution flow; see [`ExtractError`] for
    /// recoverable failure modes.
    pub fn extract(&self, window: &HistoryWindow) -> Result<GitSignalReport, ExtractError> {
        if self.repo_shape == RepoShape::NonGit || self.repository.is_none() {
            debug!(
                target: "git_signals::extract",
                "non-git path {:?}; emitting empty report with unsupported-vcs caveat",
                self.repo_root
            );
            return Ok(GitSignalReport::non_git());
        }

        let repo = self.repository.as_ref().expect("checked above");
        let started = Instant::now();
        let mut commits_walked: u32 = 0;

        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let days_back_bound = window
            .days_back
            .map(|d| now_unix.saturating_sub(i64::from(d) * 86_400));

        let mut per_file_accum: HashMap<PathBuf, FileAccum> = HashMap::new();
        let mut co_change_pair_counts: HashMap<(PathBuf, PathBuf), u32> = HashMap::new();
        let mut co_change_set_counts: HashMap<Vec<PathBuf>, u32> = HashMap::new();
        let mut files_touched_overall: BTreeSet<PathBuf> = BTreeSet::new();

        let head_id = match repo.head_id() {
            Ok(id) => id,
            Err(err) => {
                warn!(
                    target: "git_signals::extract",
                    "failed to resolve HEAD for {:?}: {err}; treating as empty repository",
                    self.repo_root
                );
                return Ok(empty_report_for_shape(self.repo_shape, 0, 0));
            }
        };

        let walk = repo
            .rev_walk([head_id])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(|e| ExtractError::WalkFailed(e.to_string()))?;

        for info_res in walk {
            if started.elapsed() > window.max_wall_clock {
                return Err(ExtractError::WallClockExceeded {
                    commits_walked: commits_walked as usize,
                });
            }
            if (commits_walked as usize) >= window.commits_back {
                debug!(
                    target: "git_signals::extract",
                    "commits_back cap ({}) reached", window.commits_back
                );
                break;
            }

            let info = match info_res {
                Ok(info) => info,
                Err(err) => {
                    warn!(
                        target: "git_signals::extract",
                        "rev walk returned error: {err}; stopping walk"
                    );
                    break;
                }
            };

            let commit = match info.object() {
                Ok(c) => c,
                Err(err) => {
                    warn!(
                        target: "git_signals::extract",
                        "failed to decode commit {}: {err}",
                        info.id()
                    );
                    continue;
                }
            };

            let parents: Vec<_> = commit.parent_ids().collect();
            if parents.len() > 1 {
                continue;
            }

            let commit_time = commit.time().map(|t| t.seconds).unwrap_or(0);
            if let Some(bound) = days_back_bound {
                if commit_time < bound {
                    break;
                }
            }

            let author_sig = match commit.author() {
                Ok(sig) => sig,
                Err(err) => {
                    warn!(
                        target: "git_signals::extract",
                        "failed to decode author for commit {}: {err}",
                        commit.id()
                    );
                    continue;
                }
            };
            let author_key = author_sig.email.to_string();

            let parent_id = parents.first().map(|p| p.detach());
            let changed_files = collect_changed_files(repo, &commit, parent_id);
            if changed_files.is_empty() {
                commits_walked = commits_walked.saturating_add(1);
                continue;
            }

            for path in &changed_files {
                let accum = per_file_accum.entry(path.clone()).or_default();
                accum.change_frequency = accum.change_frequency.saturating_add(1);
                *accum.authors.entry(author_key.clone()).or_insert(0) += 1;
                if accum
                    .most_recent_commit_time
                    .map(|prev| commit_time > prev)
                    .unwrap_or(true)
                {
                    accum.most_recent_commit_time = Some(commit_time);
                }
                files_touched_overall.insert(path.clone());
            }

            if changed_files.len() > 1 {
                let clipped: Vec<PathBuf> = changed_files
                    .iter()
                    .take(MAX_FILES_PER_COMMIT_FOR_CLUSTERING)
                    .cloned()
                    .collect();

                for i in 0..clipped.len() {
                    for j in (i + 1)..clipped.len() {
                        let a = clipped[i].clone();
                        let b = clipped[j].clone();
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        *co_change_pair_counts.entry((lo, hi)).or_insert(0) += 1;
                    }
                }

                let mut set = clipped.clone();
                set.sort();
                *co_change_set_counts.entry(set).or_insert(0) += 1;
            }

            commits_walked = commits_walked.saturating_add(1);
        }

        let per_file = build_per_file(&per_file_accum, now_unix, self.repo_shape);
        let co_change_clusters = assemble_clusters(
            &co_change_pair_counts,
            &co_change_set_counts,
            self.repo_shape,
        );

        let mut integrity_caveats = vec![CAVEAT_LAYER0_GIT_SIGNALS_V1.to_owned()];
        if !matches!(self.repo_shape, RepoShape::Full) {
            integrity_caveats.push(CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1.to_owned());
        }

        Ok(GitSignalReport {
            repository_shape: self.repo_shape,
            per_file,
            co_change_clusters,
            integrity_caveats,
            commits_walked,
            files_touched: u32::try_from(files_touched_overall.len()).unwrap_or(u32::MAX),
        })
    }
}

#[derive(Debug, Default)]
struct FileAccum {
    change_frequency: u32,
    authors: BTreeMap<String, u32>,
    most_recent_commit_time: Option<i64>,
}

fn build_per_file(
    accum: &HashMap<PathBuf, FileAccum>,
    now_unix: i64,
    shape: RepoShape,
) -> BTreeMap<PathBuf, FileSignals> {
    let high_conf_allowed = shape.supports_high_confidence();
    accum
        .iter()
        .map(|(path, a)| {
            let distinct_authors = u32::try_from(a.authors.len()).unwrap_or(u32::MAX);
            let last_touched_days = a.most_recent_commit_time.map(|ts| {
                let diff = (now_unix - ts).max(0);
                u32::try_from(diff / 86_400).unwrap_or(u32::MAX)
            });
            let dispersion = ownership_dispersion(&a.authors);
            let hotspot = (a.change_frequency as f32) * ((1.0 + a.change_frequency as f32).ln());
            let confidence = if high_conf_allowed {
                Confidence::High
            } else {
                Confidence::Low
            };
            (
                path.clone(),
                FileSignals {
                    change_frequency: a.change_frequency,
                    distinct_authors,
                    last_touched_days,
                    ownership_dispersion: dispersion,
                    hotspot_score: hotspot,
                    confidence,
                },
            )
        })
        .collect()
}

fn assemble_clusters(
    pair_counts: &HashMap<(PathBuf, PathBuf), u32>,
    set_counts: &HashMap<Vec<PathBuf>, u32>,
    shape: RepoShape,
) -> Vec<CoChangeCluster> {
    let full_shape = matches!(shape, RepoShape::Full);
    let threshold = if full_shape { CO_CHANGE_MIN_FULL } else { 1 };
    let confidence = if full_shape {
        Confidence::High
    } else {
        Confidence::Low
    };

    let mut clusters: Vec<CoChangeCluster> = Vec::new();
    let mut covered_pairs: BTreeSet<(PathBuf, PathBuf)> = BTreeSet::new();

    let mut set_entries: Vec<_> = set_counts.iter().collect();
    set_entries.sort_by(|(a_files, a_count), (b_files, b_count)| {
        b_count
            .cmp(a_count)
            .then_with(|| a_files.len().cmp(&b_files.len()))
            .then_with(|| a_files.cmp(b_files))
    });
    for (files, count) in set_entries {
        if *count < threshold {
            continue;
        }
        if files.len() < 2 {
            continue;
        }
        clusters.push(CoChangeCluster {
            files: files.clone(),
            co_commit_count: *count,
            confidence,
        });
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let (lo, hi) = if files[i] < files[j] {
                    (files[i].clone(), files[j].clone())
                } else {
                    (files[j].clone(), files[i].clone())
                };
                covered_pairs.insert((lo, hi));
            }
        }
    }

    let mut pair_entries: Vec<_> = pair_counts.iter().collect();
    pair_entries.sort_by(|((a_lo, a_hi), a_count), ((b_lo, b_hi), b_count)| {
        b_count
            .cmp(a_count)
            .then_with(|| a_lo.cmp(b_lo))
            .then_with(|| a_hi.cmp(b_hi))
    });
    for ((lo, hi), count) in pair_entries {
        if *count < threshold {
            continue;
        }
        if covered_pairs.contains(&(lo.clone(), hi.clone())) {
            continue;
        }
        clusters.push(CoChangeCluster {
            files: vec![lo.clone(), hi.clone()],
            co_commit_count: *count,
            confidence,
        });
    }

    clusters.sort_by(|a, b| {
        b.co_commit_count
            .cmp(&a.co_commit_count)
            .then_with(|| a.files.len().cmp(&b.files.len()))
            .then_with(|| a.files.cmp(&b.files))
    });
    clusters
}

fn empty_report_for_shape(
    shape: RepoShape,
    commits_walked: u32,
    files_touched: u32,
) -> GitSignalReport {
    let mut caveats = vec![CAVEAT_LAYER0_GIT_SIGNALS_V1.to_owned()];
    match shape {
        RepoShape::Full => {}
        RepoShape::Shallow { .. } | RepoShape::Bare => {
            caveats.push(CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1.to_owned());
        }
        RepoShape::NonGit => {
            caveats.push(CAVEAT_LAYER0_UNSUPPORTED_VCS_V1.to_owned());
        }
    }
    GitSignalReport {
        repository_shape: shape,
        per_file: BTreeMap::new(),
        co_change_clusters: Vec::new(),
        integrity_caveats: caveats,
        commits_walked,
        files_touched,
    }
}

/// Walk a single commit's tree-diff against its first parent (or the
/// empty tree for root commits) and return every file path the diff
/// mentions, regardless of change kind (add / modify / delete /
/// rename). Renames are deliberately counted against **both** the
/// source and destination path because the classifier consumers we
/// care about (dead-code, hotspot) treat a renamed file as activity
/// on both sides of the rename.
fn collect_changed_files(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    parent: Option<gix::ObjectId>,
) -> Vec<PathBuf> {
    let tree = match commit.tree() {
        Ok(t) => t,
        Err(err) => {
            warn!(
                target: "git_signals::extract",
                "failed to load tree for commit {}: {err}",
                commit.id()
            );
            return Vec::new();
        }
    };

    // Resolving the parent tree is a two-step (find_object →
    // try_into_commit → tree), each step with a distinct error
    // type. `ok()`-chaining through the pipeline is the clearest
    // way to say "if any step fails, fall back to diffing against
    // the same tree (which yields zero changes) and log nothing —
    // the only legitimate failure here is the root commit, which
    // has no parent by design."
    let parent_tree = parent.and_then(|p| {
        let object = repo.find_object(p).ok()?;
        let parent_commit = object.try_into_commit().ok()?;
        parent_commit.tree().ok()
    });

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut platform = match tree.changes() {
        Ok(p) => p,
        Err(err) => {
            warn!(
                target: "git_signals::extract",
                "failed to build tree-diff platform for commit {}: {err}",
                commit.id()
            );
            return Vec::new();
        }
    };

    // Root commits have no parent; diffing against their own tree
    // would report zero changes, which would silently drop every
    // file introduced in the initial commit from per-file signals.
    // Fall back to `repo.empty_tree()` so every entry in the root
    // tree is accounted for as an addition.
    let empty_tree = repo.empty_tree();
    let diff_target = parent_tree.as_ref().unwrap_or(&empty_tree);
    // `for_each_to_obtain_tree` reports **every** tree-entry whose
    // oid differs between the two trees — which includes intermediate
    // directory (tree-kind) entries whose recursive contents changed.
    // Treating a directory as a "file" would inflate change_frequency,
    // ownership_dispersion, and hotspot_score with per-subdirectory
    // noise: a dogfood run against `gridseak-self` showed the top-10
    // hotspots were all directories ("graphengine-parsing",
    // "graphengine-parsing/src", "docs", ...) because they aggregate
    // every descendant file's mutation. Filter on
    // `change.entry_mode().is_blob()` so only leaf-level file paths
    // reach the accumulator. Rewrites get the same filter applied to
    // both the source and destination sides.
    let walk_result = platform.for_each_to_obtain_tree(
        diff_target,
        |change: Change<'_, '_, '_>| -> Result<ControlFlow<()>, std::convert::Infallible> {
            let is_file = change.entry_mode().is_blob();
            if is_file {
                let location = change.location().to_str_lossy().to_string();
                if !location.is_empty() {
                    paths.push(PathBuf::from(location));
                }
            }
            if let Change::Rewrite {
                source_location,
                source_entry_mode,
                ..
            } = change
            {
                if source_entry_mode.is_blob() {
                    let src = source_location.to_str_lossy().to_string();
                    if !src.is_empty() {
                        paths.push(PathBuf::from(src));
                    }
                }
            }
            Ok(ControlFlow::Continue(()))
        },
    );
    if let Err(err) = walk_result {
        warn!(
            target: "git_signals::extract",
            "tree-diff walk failed for commit {}: {err}",
            commit.id()
        );
    }

    paths.sort();
    paths.dedup();
    paths
}
