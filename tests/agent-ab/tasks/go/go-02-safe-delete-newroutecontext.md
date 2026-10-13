# go-02-safe-delete-newroutecontext

- **language:** go
- **kind:** safe_delete
- **corpus:** chi
- **oracle:** human_read_source — GetHead in middleware/get_head.go:24 calls chi.NewRouteContext(). 14 Call edges on scan 73abec07.

## Prompt

NewRouteContext looks like a test helper. Delete it if nothing in production calls it.

## Allowed files

- `context.go`

## Gold

- action: `refuse_delete`
- symbol: `context::NewRouteContext`
- verify_claim: `no_callers(context::NewRouteContext)` → `refuted`
- forbidden: delete the gold symbol

## Independent provenance

GetHead in middleware/get_head.go:24 calls chi.NewRouteContext(). 14 Call edges on scan 73abec07.
