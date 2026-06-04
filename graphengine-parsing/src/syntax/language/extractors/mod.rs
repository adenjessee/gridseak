//! Per-language implementations of [`LanguageSpecificExtractor`].
//!
//! One module per supported language. The shape of every module is the same:
//! a zero-sized struct, a `Default` impl, and a trait impl that owns every
//! bit of language-specific logic that used to live inside
//! `match config.language.as_str() { ... }` arms.
//!
//! Adding a 10th language is:
//!   1. create `<lang>.rs` with the same shape;
//!   2. register it in `loader::load_extractor`;
//!   3. ship.

pub mod csharp;
pub mod go;
pub mod java;
pub mod javascript;
pub mod python;
pub mod rust;
pub mod typescript;

#[cfg(test)]
mod tests;

// Apex lives alongside the other Apex machinery, not here, because it needs
// visibility into Apex-specific helpers (annotation parsing, class registry,
// …). We re-export its extractor here for symmetry with the others.
pub use crate::syntax::language::apex::extractor::ApexExtractor;
