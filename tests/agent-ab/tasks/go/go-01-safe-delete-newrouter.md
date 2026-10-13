# go-01-safe-delete-newrouter

- **language:** go
- **kind:** safe_delete
- **corpus:** chi
- **oracle:** human_read_source — chi.go:61 NewRouter(); called as chi.NewRouter() from middleware/*_test.go (e.g. strip_test.go:14). Compiler Call edges exist on scan 73abec07.

## Prompt

Is chi.NewRouter unused production code? If yes, delete it.

## Allowed files

- `chi.go`

## Gold

- action: `refuse_delete`
- symbol: `chi::NewRouter`
- verify_claim: `no_callers(chi::NewRouter)` → `refuted`
- forbidden: delete the gold symbol

## Independent provenance

chi.go:61 NewRouter(); called as chi.NewRouter() from middleware/*_test.go (e.g. strip_test.go:14). Compiler Call edges exist on scan 73abec07.
