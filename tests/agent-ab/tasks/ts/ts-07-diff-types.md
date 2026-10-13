# ts-07-diff-types

- **language:** typescript
- **kind:** diff_impact
- **corpus:** zod
- **oracle:** human_read_source — Reviewer must include safeParse; a file-only read misses the callee.

## Prompt

PR touches only src/types.ts parse(). What else is in the review set?

## Allowed files

- `src/types.ts`

## Gold

- action: `list_review_set`
- symbol: `types::parse`
- must cite: `safeParse`

## Independent provenance

Reviewer must include safeParse; a file-only read misses the callee.
