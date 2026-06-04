//! Specialized resolvers for different edge types
//!
//! Each resolver handles a specific type of resolution (calls, imports, types)
//! with both LSP and heuristic fallback strategies.

pub mod call_resolver_lsp;
pub mod import_resolver;
pub mod module_dependency_resolver;
pub mod type_resolver;

pub use call_resolver_lsp::*;
pub use import_resolver::*;
pub use type_resolver::*;
