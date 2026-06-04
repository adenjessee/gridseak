//! Syntax module containing Tree-sitter based extraction and related helpers.

pub mod extractors;
pub mod language;
pub mod rust;
pub mod treesitter;
pub mod utils;

pub use treesitter::TreeSitterExtractor;
pub use utils::*;
