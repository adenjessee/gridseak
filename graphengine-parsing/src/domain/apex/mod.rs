//! Apex-specific domain types.
//!
//! `class_symbols` holds the Apex type-oracle shapes consumed by both
//! the syntax pass (extractor → `SyntaxResults.class_symbols`) and the
//! Apex heuristic resolver (`ApexClassRegistry` + `ApexClassSymbols`).
//!
//! These types live in the domain layer because they flow across
//! application-layer seams (`CallSite.arg_types`) and must obey the
//! `application → domain` dependency arrow. Relocated from
//! `syntax::language::apex::class_symbols` in Commit 1a of Phase A to
//! unblock `CallSite.arg_types: Vec<ApexTypeRef>` without creating a
//! layer inversion (see `docs/workstreams/proof-foundation-gap/COMMIT_1_SCOPING_DECISIONS.md` §Q1).

pub mod class_symbols;
