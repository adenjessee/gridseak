# Third-Party Software

GridSeak depends on a number of third-party Rust crates, each
governed by its own license. This file is auto-generated and
should not be edited by hand.

## How this file is generated

We use [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)
to materialize the dependency license list from `Cargo.lock`. To
regenerate:

```sh
cargo install cargo-about       # one-time
cargo about generate about.hbs > THIRD-PARTY.md
```

The `about.hbs` template lives at `.cargo/about.hbs` and is checked
into the repo. The license-acceptance policy (which licenses are
permitted in our dependency tree) lives at `.cargo/about.toml`.

We refresh this file before every release tag. If you are reading
this between releases, the list reflects the dependency tree at the
most recent tag, not necessarily what is on `main`.

## Permitted licenses

We accept dependencies under any of these licenses (the standard
Rust-ecosystem set):

- `Apache-2.0`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `ISC`
- `MPL-2.0` (weak copyleft, file-scope only)
- `Unicode-DFS-2016` (for `unicode-ident` and similar)
- `CC0-1.0` (public-domain-equivalent)
- `Zlib`

We do **not** accept:

- `GPL-*` (incompatible with MIT-OR-Apache-2.0 distribution)
- `LGPL-*` (license-compatibility complexity not worth it for this
  project's threat model)
- `AGPL-*` (network-copyleft incompatible)
- Any license requiring patent retaliation clauses we do not already
  carry under Apache-2.0
- Any custom / "source-available" license

`cargo-about` is configured to fail the build if a new transitive
dependency arrives with a non-accepted license. If that happens,
either pin to an older version that uses an accepted license or
file an issue.

## Notable bundled or vendored code

### Tree-sitter grammars

We vendor `graphengine-parsing/vendor/tree-sitter-sfapex/` because
the upstream Apex grammar requires custom compilation steps that
are easier to ship in-tree than as an external dependency. The
vendored source is licensed under the same terms as the upstream
grammar — see that directory's `LICENSE`.

All other tree-sitter grammars are consumed as published crates and
appear in the generated license list below.

### `rust-analyzer` crates

We depend on a subset of `rust-analyzer`'s `ra_*` crates via the
`graphengine-ra-ide-adapter` crate. These are Apache-2.0 OR MIT
licensed and appear in the generated list below.

## Generated dependency list

> *Placeholder until the first `cargo about generate` run lands.*
>
> Run `cargo about generate about.hbs > THIRD-PARTY.md` to populate
> this section. Re-run before every release.

<!-- BEGIN_AUTO_GENERATED -->
<!-- The list below is replaced by `cargo about generate`. -->
<!-- END_AUTO_GENERATED -->
