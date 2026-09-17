# G2 stranger-repro (this machine)

`scripts/stranger-repro.sh` packs a local tarball, installs into a
throwaway `HOME`, runs `setup --verify`, then evaluates a NewRouter
delete against the pinned chi artifact `73abec07` with the *installed*
binary. Success token: `STRANGER_REPRO_OK`.

This is **not** a clean-Mac `curl | sh` against GitHub `releases/latest`.
No public tag was cut in the session that added this script.

Measured 2026-09-16: `STRANGER_REPRO_OK`. Installed binary
`gridseak 0.1.0` denied `chi::NewRouter` `[compiler]` in 10 ms
against pin `73abec07`.
