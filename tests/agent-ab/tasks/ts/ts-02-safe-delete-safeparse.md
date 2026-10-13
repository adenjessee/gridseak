# ts-02-safe-delete-safeparse

- **language:** typescript
- **kind:** safe_delete
- **corpus:** zod
- **oracle:** human_read_source — parse() at types.ts:242 calls this.safeParse. 28 Compiler callers.

## Prompt

safeParse looks internal. Delete if nothing calls it.

## Allowed files

- `src/types.ts`

## Gold

- action: `refuse_delete`
- symbol: `types::safeParse`
- verify_claim: `no_callers(types::safeParse)` → `refuted`
- forbidden: delete the gold symbol

## Independent provenance

parse() at types.ts:242 calls this.safeParse. 28 Compiler callers.
