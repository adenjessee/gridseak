//! LSP-based semantic resolver implementation
//!
//! This module provides the semantic resolution layer using Language Server Protocol (LSP)
//! to resolve syntactic hints from Tree-sitter into accurate semantic relationships.
//! It implements the SemanticResolver port from the application layer.

pub mod call_resolver;
pub mod client;
pub mod column_utils;
pub mod command_locator;
pub mod def_trace;
pub mod definition_provider;
pub mod errors;
pub mod file_analyzer;
// `pub mod mock_resolver;` + `pub mod mock_session;` were removed in
// R2 (v0.1.0-rc1 follow-up). The mock LSP resolver + session
// supervisor moved to `graphengine-parsing-test-support` so they no
// longer live in production source.
pub mod notification_sink;
pub mod protocol;
pub mod real_resolver;
pub mod receiver_detector;
pub mod resolver;
pub mod resolvers;
pub mod security;
pub mod session;
pub mod simple_client;
pub mod stats;
pub mod synchronization;
pub mod telemetry_export;
pub mod timing;
pub mod utils;

// Re-export main types for convenience
pub use call_resolver::*;
pub use client::*;
pub use command_locator::*;
pub use definition_provider::*;
pub use errors::LspError as LspErrorType;
pub use file_analyzer::*;
// `pub use mock_resolver::*;` + `pub use mock_session::*;` removed in
// R2. See sibling note on the `mock_*` mod declarations above.
pub use protocol::{LspError as LspProtocolError, *};
pub use real_resolver::*;
pub use receiver_detector::*;
pub use resolver::*;
pub use security::*;
pub use session::*;
pub use simple_client::*;
pub use synchronization::*;
pub use timing::*;
