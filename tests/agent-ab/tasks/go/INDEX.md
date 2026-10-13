Go tasks live in [`../../tasks.json`](../../tasks.json) (`id` prefix `go-`). Ten tasks: safe delete, wrong twin, blast, false claim, diff, honesty, cycle.

| id | kind | gold |
|---|---|---|
| [go-01-safe-delete-newrouter](go-01-safe-delete-newrouter.md) | safe_delete | refuse; `no_callers(chi::NewRouter)` refuted |
| [go-02-safe-delete-newroutecontext](go-02-safe-delete-newroutecontext.md) | safe_delete | refuse; live callers |
| [go-03-wrong-twin-use](go-03-wrong-twin-use.md) | wrong_twin | edit `mux.go`, not `chi.go` |
| [go-04-wrong-twin-handlerfunc](go-04-wrong-twin-handlerfunc.md) | wrong_twin | stdlib; do not edit `chain.go` |
| [go-05-blast-gethead](go-05-blast-gethead.md) | blast | cite `NewRouteContext`, `RouteContext` |
| [go-06-false-claim-newrouter-gethead](go-06-false-claim-newrouter-gethead.md) | false_claim | `calls(NewRouter,GetHead)` refuted |
| [go-07-diff-gethead](go-07-diff-gethead.md) | diff_impact | review set includes `NewRouteContext` |
| [go-08-cycle-servehttp](go-08-cycle-servehttp.md) | cycle | use artifact `in_cycle` |
| [go-09-honesty-router-use](go-09-honesty-router-use.md) | honesty | unknown (interface dispatch) |
| [go-10-false-no-callers-use](go-10-false-no-callers-use.md) | false_claim | `no_callers(mux::Use)` refuted |
