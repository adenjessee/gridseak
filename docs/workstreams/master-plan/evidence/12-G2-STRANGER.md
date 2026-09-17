# G2 stranger-repro

## Local tarball (this machine)

`scripts/stranger-repro.sh` packs a local tarball, installs into a
throwaway `HOME`, runs `setup --verify`, then evaluates a NewRouter
delete against the pinned chi artifact `73abec07` with the *installed*
binary. Success token: `STRANGER_REPRO_OK`.

Measured 2026-09-16: `STRANGER_REPRO_OK`. Installed binary
`gridseak 0.1.0` (workspace version at pack time) denied
`chi::NewRouter` `[compiler]` in 10 ms against pin `73abec07`.

## Public GitHub `curl | sh` (this machine, throwaway `GRIDSEAK_HOME`)

Public tag `cli-v0.1.1` is live:
https://github.com/adenjessee/gridseak/releases/tag/cli-v0.1.1

`releases/latest` redirects to `cli-v0.1.1`. Assets: macOS arm64 +
Intel, Linux x86_64, Windows x86_64, `cli-manifest.json`.

Measured 2026-09-17 on this Mac into `/tmp/gs-011-fresh` (did **not**
overwrite `~/.gridseak`):

```
curl -fsSL https://raw.githubusercontent.com/adenjessee/gridseak/main/scripts/install/install.sh \
  | GRIDSEAK_HOME=/tmp/gs-011-fresh bash
```

Installed `gridseak 0.1.1`. `gridseak gate --help` exists. SHA256 of
`gridseak-0.1.1-aarch64-apple-darwin.tar.gz` verified.

This is **not** a clean-Mac proof. This machine already has cargo,
hooks, and the chi pin. A second Mac with no prior GridSeak is still
required for stranger G2.
