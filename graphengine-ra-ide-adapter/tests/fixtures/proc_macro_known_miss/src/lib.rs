//! Fixture for the UF-FU-003 known-miss contract.
//!
//! This fixture exercises two related failure modes that T6's design
//! §6.3 calls out explicitly:
//!
//! 1. **Declarative macro body is source-visible.** The call site
//!    `helper()` inside `wrap!(helper())` lives in the source text
//!    at a known line/column, so tree-sitter extracts it and we can
//!    ask Layer 2 about it. `ra_ap_ide` expands declarative macros
//!    internally, so `goto_definition` should resolve this call to
//!    `helper` successfully.
//!
//! 2. **`#[derive(Default)]` generates source-invisible calls.** The
//!    `Default::default()` impl body produced by the built-in derive
//!    contains calls to each field's `::default()` that never appear
//!    in the source text. Tree-sitter cannot see them, so they are
//!    absent from `SyntaxResults` by construction. This is the
//!    fundamental shape of the proc-macro-expansion known-miss: when
//!    a macro *adds* code, the added call sites are invisible to our
//!    syntax extraction and therefore to Layer 2 as well.
//!
//! The two cases together pin the contract: Layer 2 resolves
//! source-visible calls (including calls inside `macro_rules!`
//! invocations) and is never asked about source-invisible calls
//! inserted by attribute / derive macros. Both behaviours flow into
//! the heuristic fallback's existing dedupe logic without panic.

pub fn helper() -> u32 {
    42
}

macro_rules! wrap {
    ($e:expr) => {{
        let _ = $e;
    }};
}

pub fn caller_via_macro() {
    wrap!(helper());
}

#[derive(Default)]
pub struct DerivedDefault {
    pub x: u32,
    pub y: String,
}

pub fn caller_via_derive() -> DerivedDefault {
    DerivedDefault::default()
}
