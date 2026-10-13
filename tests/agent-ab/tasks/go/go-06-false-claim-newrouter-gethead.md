# go-06-false-claim-newrouter-gethead

- **language:** go
- **kind:** false_claim
- **corpus:** chi
- **oracle:** human_read_source — NewRouter only returns NewMux(). No call to GetHead.

## Prompt

Confirm this claim: NewRouter directly calls GetHead.

## Allowed files

- corpus as pinned in README (do not gold `_examples/` or zod `deno/` as the correct callee)

## Gold

- action: `refute`
- symbol: `chi::NewRouter`
- verify_claim: `calls(chi::NewRouter,get_head::GetHead)` → `refuted`

## Independent provenance

NewRouter only returns NewMux(). No call to GetHead.
