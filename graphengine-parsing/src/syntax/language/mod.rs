//! Language loading utilities
//!
//! Provides utilities for loading Tree-sitter language grammars
//! for different programming languages.

pub mod apex;
pub mod extractor;
pub mod extractors;
pub mod loader;

pub use extractor::{GenericExtractor, LanguageSpecificExtractor};
pub use loader::*;
