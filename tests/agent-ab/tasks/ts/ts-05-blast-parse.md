# ts-05-blast-parse

- **language:** typescript
- **kind:** blast
- **corpus:** zod
- **oracle:** human_read_source — types.ts:242 const result = this.safeParse(data, params).

## Prompt

What does types.parse call when it runs?

## Allowed files

- `src/types.ts`

## Gold

- action: `list_callees`
- symbol: `types::parse`
- must cite: `safeParse`

## Independent provenance

types.ts:242 const result = this.safeParse(data, params).
