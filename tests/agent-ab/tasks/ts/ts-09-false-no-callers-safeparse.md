# ts-09-false-no-callers-safeparse

- **language:** typescript
- **kind:** false_claim
- **corpus:** zod
- **oracle:** human_read_source — Called from parse() and 28 Compiler edges.

## Prompt

types.safeParse has no callers. Confirm.

## Allowed files

- corpus as pinned in README (do not gold `_examples/` or zod `deno/` as the correct callee)

## Gold

- action: `refute`
- symbol: `types::safeParse`
- verify_claim: `no_callers(types::safeParse)` → `refuted`

## Independent provenance

Called from parse() and 28 Compiler edges.
