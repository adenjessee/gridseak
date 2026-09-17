//! SCIP index ingestion and indexer provisioning for GridSeak's compiler tier.
//!
//! Uses the official crates.io `scip` crate (v0.9.0, Sourcegraph protobuf bindings)
//! rather than hand-vendored `scip.proto` + prost codegen.

pub mod fingerprint;
pub mod index;
pub mod provisioning;

pub use index::{DecodedRange, ScipDefinition, ScipIndex, ScipIndexError};
pub use provisioning::{
    install_indexer, provision_index, IndexerError, IndexerLanguage, INDEXER_TIMEOUT,
};
