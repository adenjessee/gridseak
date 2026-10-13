# go-05-blast-gethead

- **language:** go
- **kind:** blast
- **corpus:** chi
- **oracle:** human_read_source — get_head.go:13 chi.RouteContext; :24 chi.NewRouteContext; :32 next.ServeHTTP. Downstream, not only the test file.

## Prompt

If we change GetHead, which functions are definitely affected? List them.

## Allowed files

- `middleware/get_head.go`

## Gold

- action: `list_callees`
- symbol: `get_head::GetHead`
- must cite: `NewRouteContext`, `RouteContext`

## Independent provenance

get_head.go:13 chi.RouteContext; :24 chi.NewRouteContext; :32 next.ServeHTTP. Downstream, not only the test file.
