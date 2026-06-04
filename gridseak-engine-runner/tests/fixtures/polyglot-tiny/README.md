# polyglot-tiny

A two-language fixture for the runner's integration tests.

- `src/lib.rs` — Rust: free function, struct, impl block.
- `src/greet.ts` — TypeScript: function, class with method, const export.

Both files mirror each other's shape (function + class + module-level
binding) so a polyglot scan exercises both parser pipelines and the
analyzer's cross-language metric aggregation. Files are intentionally
**tiny** — the goal is to keep the parity test under a second on a cold
build cache while still emitting a non-empty `findings` array and a
deterministic `metrics` block.

This fixture is *not* a runnable Rust crate — there's no `Cargo.toml`.
The parser only cares about source files and language descriptors; a
crate manifest would just slow things down and add noise to the report.
