# ts-04-wrong-twin-min

- **language:** typescript
- **kind:** wrong_twin
- **corpus:** zod
- **oracle:** human_read_source — Same name exists under deno/lib. Package source of truth is src/types.ts.

## Prompt

Change the min helper used by ZodNumber in src/. Do not touch republished trees.

## Allowed files

- `src/types.ts`

## Gold

- action: `edit_correct_file`
- symbol: `types::min`
- forbidden edit: `deno/lib/types.ts`

## Independent provenance

Same name exists under deno/lib. Package source of truth is src/types.ts.
