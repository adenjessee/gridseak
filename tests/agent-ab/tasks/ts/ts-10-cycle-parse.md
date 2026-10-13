# ts-10-cycle-parse

- **language:** typescript
- **kind:** cycle
- **corpus:** zod
- **oracle:** human_read_source — Must use in_cycle on the artifact, not assume 'parse is recursive'.

## Prompt

Is types.parse in a call-graph cycle?

## Allowed files

- corpus as pinned in README (do not gold `_examples/` or zod `deno/` as the correct callee)

## Gold

- action: `use_oracle`
- symbol: `types::parse`
- verify_claim: `in_cycle(types::parse)` → `either`

## Independent provenance

Must use in_cycle on the artifact, not assume 'parse is recursive'.
