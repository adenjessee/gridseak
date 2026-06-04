//! Specialized extractors for different syntax elements
//!
//! Each extractor handles a specific type of syntax extraction (symbols, calls, imports, etc.)

pub mod call_site_extractor;
pub mod complexity_extractor;
pub mod identifier_use_extractor;
pub mod import_extractor;
pub mod module_extractor;
pub mod symbol_extractor;
pub mod trait_context_detector;
pub mod type_ref_extractor;

pub use call_site_extractor::*;
pub use identifier_use_extractor::*;
pub use import_extractor::*;
pub use module_extractor::*;
pub use symbol_extractor::*;
pub use trait_context_detector::*;
pub use type_ref_extractor::*;
