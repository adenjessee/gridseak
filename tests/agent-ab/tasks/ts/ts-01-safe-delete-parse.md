# ts-01-safe-delete-parse

- **language:** typescript
- **kind:** safe_delete
- **corpus:** zod
- **oracle:** human_read_source — src/types.ts:241 parse(). 114 Compiler callers on scan 4cb6e82a. Public API.

## Prompt

Is ZodType.parse dead? Delete it if unused.

## Allowed files

- `src/types.ts`

## Gold

- action: `refuse_delete`
- symbol: `types::parse`
- verify_claim: `no_callers(types::parse)` → `refuted`
- forbidden: delete the gold symbol

## Independent provenance

src/types.ts:241 parse(). 114 Compiler callers on scan 4cb6e82a. Public API.
