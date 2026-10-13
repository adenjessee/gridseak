# go-04-wrong-twin-handlerfunc

- **language:** go
- **kind:** wrong_twin
- **corpus:** chi
- **oracle:** human_read_source — get_head.go:11 returns http.HandlerFunc(...). chain.HandlerFunc is a different in-repo func. Do not edit chain.go.

## Prompt

Fix HandlerFunc used as http.HandlerFunc(fn) in GetHead. Edit the definition you think it is.

## Allowed files

- `stdlib`

## Gold

- action: `do_not_edit_repo_twin`
- symbol: `net/http.HandlerFunc`
- forbidden edit: `chain.go`

## Independent provenance

get_head.go:11 returns http.HandlerFunc(...). chain.HandlerFunc is a different in-repo func. Do not edit chain.go.
