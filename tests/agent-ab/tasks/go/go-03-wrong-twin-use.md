# go-03-wrong-twin-use

- **language:** go
- **kind:** wrong_twin
- **corpus:** chi
- **oracle:** human_read_source — chi.go:72 is Router.Use interface method; mux.go has Mux.Use implementation. 26 Compiler callers of mux::Use.

## Prompt

Rename the Use function that Mux actually implements. There may be more than one Use.

## Allowed files

- `mux.go`

## Gold

- action: `edit_correct_file`
- symbol: `mux::Use`
- forbidden edit: `chi.go`

## Independent provenance

chi.go:72 is Router.Use interface method; mux.go has Mux.Use implementation. 26 Compiler callers of mux::Use.
