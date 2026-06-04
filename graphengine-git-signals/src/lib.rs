//! # graphengine-git-signals
//!
//! Layer 0 evidence stream for the universal-fidelity engine.
//!
//! This crate is the **floor** every other fidelity layer sits on. It
//! consults the `.git/` directory of the scanned repository and emits
//! a bounded, typed, confidence-tagged [`GitSignalReport`] that the
//! analysis pipeline consumes alongside Layer 1 (tree-sitter syntax)
//! and Layer 2 (semantic resolution) evidence.
//!
//! The sprint-level rationale lives in
//! [`docs/workstreams/universal-fidelity/tasks/T7-layer0-git-signals.md`].
//!
//! ## The three invariants this crate protects
//!
//! 1. **Graph shape is owned by Layer 1 + Layer 2.** Git signals never
//!    synthesise [`Edge`]-shaped data. Emitting a `co_change_cluster`
//!    does not produce an edge between two nodes in the code graph;
//!    it is a parallel evidence stream the classifier consumes.
//! 2. **Every signal carries explicit confidence.** Shallow clones
//!    (one-commit CI fixtures, `--depth 1` `git clone` operations,
//!    tarball extractions) cannot produce `Confidence::High` signals.
//!    The [`RepoShape::detect`] guard downgrades every
//!    [`FileSignals::confidence`] to [`Confidence::Low`] on any
//!    non-[`RepoShape::Full`] shape. **This is load-bearing from day
//!    one** because NPSP rev 9, commons-lang, django-site, and
//!    serilog canaries are all 1-commit clones — without the guard
//!    the classifier would see `change_frequency: 0` and falsely
//!    judge every file dead/cold.
//! 3. **Signals are consumed through predicates, not raw fields.**
//!    The classifier never reads `FileSignals.change_frequency`
//!    directly; it asks the [`GitSignalConsumer`] predicate trait
//!    ([`GitSignalConsumer::is_active_recent`],
//!    [`GitSignalConsumer::is_high_churn`], etc.). Adding a new
//!    signal type updates the predicate once; every metric consumer
//!    automatically picks up the new signal. This mirrors the
//!    `EdgeKind::is_call_like()` pattern introduced in P1.a.
//!
//! ## Out of scope (see T7 §2)
//!
//! - Using git signals to **override** call edges (graph shape is
//!   immutable to Layer 0).
//! - Cross-repository co-change.
//! - Non-git VCS adapters (Mercurial, Perforce, Fossil, Jujutsu).
//! - Commit-message classification.
//!
//! ## Performance envelope
//!
//! T7 §5.7 kill criterion: extracting signals on `gridseak-self`
//! (full clone, ~200 commits, 348 kloc) must complete in < 2 s
//! wall-clock and add < 50 MB RSS over the pre-T7 baseline. The
//! [`HistoryWindow::max_wall_clock`] bound enforces the time
//! ceiling at the walk level; exceeding it returns
//! [`ExtractError::WallClockExceeded`] rather than silently
//! truncating the signal.

// Internal module layout keeps each concern to its own file (project
// architecture rule: no single file becomes a kitchen sink). Public
// re-exports below form the crate's surface API.
mod caveats;
mod confidence;
mod errors;
mod extractor;
mod ownership;
/// Predicate-based consumption surface. Exposed as `pub mod` (not
/// re-exported flat) so downstream crates can reference the
/// threshold constants — `ACTIVE_RECENT_MAX_DAYS`,
/// `HIGH_CHURN_MIN_COMMITS`, `HOTSPOT_MIN_AUTHORS` — by path when
/// they replicate the consumer logic outside this crate (the
/// dead-code churn downgrade in `graphengine-analysis` does so).
/// The trait itself is still re-exported under
/// [`GitSignalConsumer`].
pub mod predicates;
mod repo_shape;
mod report;
mod window;

pub use caveats::{
    CAVEAT_LAYER0_GIT_SIGNALS_V1, CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1,
    CAVEAT_LAYER0_UNSUPPORTED_VCS_V1,
};
pub use confidence::Confidence;
pub use errors::{ExtractError, OpenError};
pub use extractor::GitSignalExtractor;
pub use predicates::GitSignalConsumer;
pub use repo_shape::RepoShape;
pub use report::{CoChangeCluster, FileSignals, GitSignalReport};
pub use window::HistoryWindow;
