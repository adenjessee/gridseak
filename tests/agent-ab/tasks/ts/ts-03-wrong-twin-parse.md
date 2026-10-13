# ts-03-wrong-twin-parse

- **language:** typescript
- **kind:** wrong_twin
- **corpus:** zod
- **oracle:** human_read_source — deno/lib/types.ts is a Deno republish. First-path grep often hits deno/. Canonical TS source is src/types.ts:241.

## Prompt

Rename parse on the TypeScript package (not a republish tree).

## Allowed files

- `src/types.ts`

## Gold

- action: `edit_correct_file`
- symbol: `types::parse`
- forbidden edit: `deno/lib/types.ts`

## Independent provenance

deno/lib/types.ts is a Deno republish. First-path grep often hits deno/. Canonical TS source is src/types.ts:241.
