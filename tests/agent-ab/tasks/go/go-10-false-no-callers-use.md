# go-10-false-no-callers-use

- **language:** go
- **kind:** false_claim
- **corpus:** chi
- **oracle:** human_read_source — 26 Compiler Call edges to mux::Use on scan 73abec07. Tests and examples call r.Use(...).

## Prompt

mux.Use has no callers. Confirm so we can delete it.

## Allowed files

- corpus as pinned in README (do not gold `_examples/` or zod `deno/` as the correct callee)

## Gold

- action: `refute`
- symbol: `mux::Use`
- verify_claim: `no_callers(mux::Use)` → `refuted`

## Independent provenance

26 Compiler Call edges to mux::Use on scan 73abec07. Tests and examples call r.Use(...).
