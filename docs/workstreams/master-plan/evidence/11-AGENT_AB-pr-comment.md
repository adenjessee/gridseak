<!-- gridseak-pr-comment -->
## GridSeak structural review

**Headline:** The other run said it was unused. GridSeak's compiler said callers on `chi::NewRouter`. I would have shipped a production break.

### Agent A/B (deterministic grep control vs GridSeak treatment)

- Control accuracy: 0.00
- Treatment accuracy: 1.00
- Tree-sitter-only ablation: 0.75
- False-delete rate control/treatment: 1.00 / 0.00
- Wrong-twin error control/treatment: 1.00 / 0.00

### Share task `go-01-safe-delete-newrouter` (`chi::NewRouter`)
The control approved a delete. Treatment ran `no_callers` and **refused** (verdict `refuted`).

Compiler-tier callers (first 5):
- `gridseak-graphengine::middleware::strip_test::TestStripSlashes`
- `gridseak-graphengine::middleware::strip_test::TestStripSlashesInRoute`
- `gridseak-graphengine::middleware::strip_test::TestRedirectSlashes`
- `gridseak-graphengine::middleware::strip_test::TestStripPrefix`
- `gridseak-graphengine::middleware::content_charset_test::TestContentCharset`

Tiers: treatment quotes the scan artifact (Compiler Call edges). Control is grep.
