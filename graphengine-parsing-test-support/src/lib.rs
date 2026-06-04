//! Test-only fixtures for `graphengine-parsing`.
//!
//! # Why this crate exists (R2 follow-up — v0.1.0-rc1)
//!
//! Prior to this crate, every mock implementation of a parsing-pipeline
//! port (`MockSyntaxExtractor`, `MockSemanticResolver`,
//! `MockGraphRepository`, `MockLspResolver`, `MockSessionSupervisor`)
//! lived under `graphengine-parsing/src/`. Three downstream consequences
//! were unacceptable:
//!
//! 1. **Public-API pollution.** `pub use mocks::*` in
//!    `application/mod.rs` and `infrastructure/mod.rs` made the mocks
//!    part of the published parsing-crate surface — anyone depending on
//!    `graphengine-parsing` could (and would) reach for them, breaking
//!    the dependency direction the clean-architecture layout meant to
//!    enforce.
//! 2. **Substring-resolution noise in the call graph.** Tree-sitter's
//!    heuristic resolver builds suffix/substring edges across the
//!    workspace symbol index. Short generic method names on mocks
//!    (`get`, `clone`, `new`, `clear`, `list`, `delete`) absorbed
//!    callers from every `.get(…)` / `.clone()` site in the codebase.
//!    Dogfood scan evidence (`docs/04-evidence/PILOT_REPORTS/
//!    gridseak-graphengine.graph-top-fan-in.txt`): 297 fan-in to
//!    `application::mocks::get`, 460 fan-in to `mock_session::clone`.
//!    Those edges were noise — no production path actually called the
//!    mocks — but they polluted hotspot / coupling / dead-code metrics.
//! 3. **Wrong "dead code" verdicts.** With the mocks in production
//!    source, `ge-analyze` treated them as reachable-via-`pub` symbols
//!    that "should be called" and never flagged them as dead. Moving
//!    them out lets dead-code analysis cleanly skip a crate that exists
//!    solely as a `[dev-dependencies]` test surface.
//!
//! # Wire compatibility
//!
//! Type definitions are byte-for-byte identical to the originals — only
//! the module path changed. Re-imports use the parsing crate's public
//! API (`graphengine_parsing::application::ports`,
//! `graphengine_parsing::domain`, …) the same way an external consumer
//! would. If a test previously did
//! `use graphengine_parsing::application::mocks::MockGraphRepository`,
//! the new path is
//! `use graphengine_parsing_test_support::MockGraphRepository`.
//!
//! # What's NOT here
//!
//! - **Configurable mocks defined inside individual integration test
//!   files** (e.g. `ConfigurableMockSyntaxExtractor` in
//!   `tests/application/integration_tests.rs`). Those are test-local
//!   and stay where they are; they were never part of the public surface
//!   so they don't pollute the call graph.
//! - **The `with_infrastructure` factory method** that previously
//!   instantiated `MockSyntaxExtractor` + `MockLspResolver` +
//!   `MockGraphRepository` from production code. R2 deleted that factory
//!   because its only callers were two tests / one bench; rewriting
//!   them to construct the use case directly with mocks from this
//!   crate is the cleaner separation.

pub mod application_mocks;
pub mod mock_extractor;
pub mod mock_lsp_resolver;
pub mod mock_lsp_session;

pub use application_mocks::{MockGraphRepository, MockSemanticResolver, MockSyntaxExtractor};
pub use mock_extractor::MockSyntaxExtractor as MockSyntaxExtractorWithConfig;
pub use mock_lsp_resolver::MockLspResolver;
pub use mock_lsp_session::{MockSessionState, MockSessionSupervisor};
