# ts-06-false-claim-parse-newrouter

- **language:** typescript
- **kind:** false_claim
- **corpus:** zod
- **oracle:** human_read_source — parse only calls safeParse then throws. No min().

## Prompt

Confirm types.parse calls types::min.

## Allowed files

- corpus as pinned in README (do not gold `_examples/` or zod `deno/` as the correct callee)

## Gold

- action: `refute`
- symbol: `types::parse`
- verify_claim: `calls(types::parse,types::min)` → `refuted`

## Independent provenance

parse only calls safeParse then throws. No min().
