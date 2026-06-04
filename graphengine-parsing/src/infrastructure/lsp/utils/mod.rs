//! Utility modules for LSP resolution
//!
//! These modules provide pure utility functions that don't depend on resolver state,
//! making them reusable across different resolution components.

pub mod call_site_utils;
pub mod document_sync;
pub mod range_utils;
pub mod source_code_utils;
pub mod symbol_lookup;

pub use call_site_utils::*;
pub use document_sync::*;
pub use range_utils::*;
pub use source_code_utils::*;
pub use symbol_lookup::*;
