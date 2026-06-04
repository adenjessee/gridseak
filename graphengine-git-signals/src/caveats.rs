//! Versioned caveat tokens emitted onto any [`GitSignalReport`][super::GitSignalReport]
//! produced by this crate.
//!
//! These follow the same contract as the `CAVEAT_*` constants in
//! `graphengine-analysis::health::report`:
//!
//! - **Never rename a constant.** Downstream consumers match on the
//!   literal string value; a rename is a silent data-corruption bug.
//! - **Never remove a constant.** Reports carrying it still exist in
//!   the field and must decode.
//! - **Version bumps are additive.** A semantic change introduces a
//!   new `_V2` token; the old token keeps its historical meaning.

/// Stamped on every report produced by a T7-aware engine. Downstream
/// tools use its presence to distinguish "this engine ran the
/// Layer 0 stage" from "this engine predates T7 and therefore has no
/// Layer 0 signal at all." Absence implies the latter.
pub const CAVEAT_LAYER0_GIT_SIGNALS_V1: &str = "layer0_git_signals_v1";

/// Emitted when [`RepoShape::detect`][super::RepoShape::detect]
/// classifies the repository as anything other than
/// [`RepoShape::Full`][super::RepoShape::Full] (shallow clone, bare
/// repository, or git-less directory tree). The report still
/// contains every signal the engine could compute, but every
/// [`FileSignals::confidence`][super::FileSignals::confidence] is
/// forcibly downgraded to [`Confidence::Low`][super::Confidence::Low]
/// because a shallow / missing history cannot support a
/// `High`-confidence verdict. Consumers gating behaviour on
/// `Confidence::High` therefore silently skip Layer 0 on these
/// repositories — no behavioural surprise, no false-negative
/// "everything is cold" read.
pub const CAVEAT_LAYER0_INSUFFICIENT_HISTORY_V1: &str = "layer0_insufficient_history_v1";

/// Emitted when the scanned directory is not a git working tree at
/// all (no `.git/` folder, no `.git` file pointing at a worktree,
/// Mercurial / Perforce / Fossil / Jujutsu checkout, tarball
/// extract, etc.). The report's
/// [`GitSignalReport::per_file`][super::GitSignalReport::per_file]
/// is empty and every consumer predicate returns `false`. This is
/// the *correct* shape for a non-git workspace; customers on these
/// VCSes are not regressed, they simply receive no Layer 0 data
/// until a dedicated adapter ships (see T7 §8's T7.c follow-up).
pub const CAVEAT_LAYER0_UNSUPPORTED_VCS_V1: &str = "layer0_unsupported_vcs_v1";
