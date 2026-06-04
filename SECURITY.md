# Security Policy

## Supported versions

GridSeak is pre-1.0 software. Only the **latest tag and the `main`
branch** receive security updates. Earlier versions are not supported.

| Version | Supported |
| --- | --- |
| `main` (HEAD) | Yes |
| Latest release tag | Yes |
| Older release tags | No |
| Older `main` SHAs | No |

When we cut 1.0, we will introduce a more formal "previous-minor"
support window. Until then, please update before reporting a
vulnerability.

## What counts as a vulnerability

We treat as security-relevant any defect that:

- Allows an attacker to execute code on a developer's machine via a
  scan of an untrusted codebase. (Our threat model assumes a developer
  scans a repo they may not fully trust. The scan must be a read-only
  operation.)
- Allows an attacker to exfiltrate code, file contents, or
  `.gridseak/` ledger contents via the MCP transport or the install
  script.
- Allows an attacker to corrupt the local SQLite parse DB or the
  `.gridseak/` ledger in a way that produces wrong analysis output
  the user would otherwise trust.
- Allows an attacker to take over the install pipeline
  (`gridseak.com/install.sh` or the release binaries on
  GitHub Releases) to ship a backdoored binary.
- Is a buffer-overflow / use-after-free / double-free in any
  `unsafe` block in the workspace. (We keep `unsafe` to a minimum;
  see [`BUILD.md`](BUILD.md) §"Unsafe inventory".)

We do not consider security-relevant:

- Bugs that produce incorrect analysis output without an attacker.
  (Those are correctness bugs — please file an issue.)
- Bugs that crash the CLI or MCP server on input we already document
  as unsupported (e.g., binary files in a source tree).
- Issues in third-party dependencies that are already published as
  RUSTSEC advisories and that we are tracking via
  [`THIRD-PARTY.md`](THIRD-PARTY.md) updates.

## How to report

**Please do not file public GitHub issues for security vulnerabilities.**

Email: **security@gridseak.com** — not monitored yet for v0.1.0.

**GitHub Security Advisory (preferred for v0.1.0):**
<https://github.com/adenjessee/gridseak/security/advisories/new>

Please include:

- A description of the vulnerability and its impact.
- Steps to reproduce. If you have a proof-of-concept, attach it.
- Your name and how you'd like to be credited (or "anonymous").

We aim to respond within **72 hours** during weekdays and to issue a
fix within **14 days** for critical vulnerabilities (RCE,
exfiltration, supply-chain compromise) and within **30 days** for
high-severity ones.

If we miss those windows, we will tell you why publicly in the
advisory once it's resolved.

## Coordinated disclosure

We follow standard coordinated disclosure:

1. You report privately.
2. We confirm receipt within 72 hours.
3. We work on a fix.
4. We release the fix with a `RUSTSEC-` advisory and a
   `CHANGELOG.md` entry.
5. After 7 days of the fix being available (or sooner with your
   agreement), we publish a public advisory crediting you.

If the vulnerability is being actively exploited in the wild, we will
publish the advisory immediately and skip the embargo.

## PGP key

We do not currently publish a PGP key. The GitHub Security Advisory
mechanism uses GitHub's transport encryption and is sufficient for
our threat model. If your reporting requires PGP, email us first
asking for a key and we will generate and publish one.

## Acknowledgments

We will maintain a `SECURITY-CREDITS.md` once we have any. As of
this writing, none.
