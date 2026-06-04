//! Infrastructure utilities.
//!
//! Stable node ID generation now lives in `crate::domain::node_id`
//! (see T2 of the universal-fidelity sprint). The legacy location-based
//! `IdGenerator` previously defined here was removed on 2026-04-18 because
//! it was dead code and its scheme (`SHA256(fqn || line || col)`) is the
//! exact anti-pattern T2 replaces.

pub mod position_converter;

pub use position_converter::*;
