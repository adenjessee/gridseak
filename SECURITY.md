# Security

GridSeak is **pre-1.0** software maintained by **one person** in spare time.
It runs **entirely on your machine**. There is no GridSeak cloud, security
team, or bug-bounty program.

## Use at your own risk

Install and scan codebases like any other local developer tool. If something
breaks or worries you, stop using it and open a normal
[GitHub issue](https://github.com/adenjessee/gridseak/issues).

## What is worth reporting privately

Use a
[GitHub Security Advisory](https://github.com/adenjessee/gridseak/security/advisories/new)
(**not** a public issue) only if you believe you have found:

- Remote code execution from scanning an untrusted repository
- A way to exfiltrate your source or `.gridseak/` data without your intent
- A compromised install path (backdoored release binary or tampered install
  script)

Everything else — wrong analysis, crashes, dependency CVEs already tracked in
[`THIRD-PARTY.md`](THIRD-PARTY.md) — belongs in a public issue.

## Supported versions

Latest release tag and `main` only. Older tags are not supported.

## Response

I'll respond when I can. There is **no SLA** and no coordinated-disclosure
timeline. Thank you for reporting serious issues; I may not be able to fix
them quickly.

## Contact

GitHub Security Advisories only (link above). There is no `security@` mailbox yet.
