# go-07-diff-gethead

- **language:** go
- **kind:** diff_impact
- **corpus:** chi
- **oracle:** human_read_source — Reviewer must see NewRouteContext/RouteContext usage inside GetHead, not only the one file name.

## Prompt

We will edit middleware/get_head.go only. What else should a reviewer read?

## Allowed files

- `middleware/get_head.go`

## Gold

- action: `list_review_set`
- symbol: `get_head::GetHead`
- must cite: `NewRouteContext`

## Independent provenance

Reviewer must see NewRouteContext/RouteContext usage inside GetHead, not only the one file name.
