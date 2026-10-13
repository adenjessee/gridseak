# go-08-cycle-servehttp

- **language:** go
- **kind:** cycle
- **corpus:** chi
- **oracle:** human_read_source — Answer must come from cycle query, not a guessed 'routers always cycle'. Scorer compares treatment/control to the artifact in_cycle verdict (independent of SCIP labels).

## Prompt

Is mux.ServeHTTP in a call-graph cycle?

## Allowed files

- corpus as pinned in README (do not gold `_examples/` or zod `deno/` as the correct callee)

## Gold

- action: `use_oracle`
- symbol: `mux::ServeHTTP`
- verify_claim: `in_cycle(mux::ServeHTTP)` → `either`

## Independent provenance

Answer must come from cycle query, not a guessed 'routers always cycle'. Scorer compares treatment/control to the artifact in_cycle verdict (independent of SCIP labels).
