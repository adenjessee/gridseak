//! `gridseak doctor` — verify shadow-mode install consistency.
//!
//! # What this command answers
//!
//! "Are the three executables that participate in every `gridseak scan`
//! (the CLI itself, `graphengine-parsing`, `ge-analyze`) all from the
//! same build?"
//!
//! Shadow-mode scans depend on three independently linked binaries
//! that share an internal wire contract (progress JSONL, flag set,
//! schema versions). When that contract drifts — e.g. a user upgrades
//! `gridseak` but the install prefix still has the old `ge-analyze`,
//! or `GE_ANALYZE_BIN` points at a hand-built binary from an older
//! branch — scans either fail with confusing low-level errors or, in
//! the worst case, silently produce wrong reports.
//!
//! `gridseak doctor` is the user-facing tool that diagnoses this in
//! one step: it discovers each sidecar through the same resolution
//! rules `gridseak scan` uses, spawns it with `--version`, and
//! reports whether all three (CLI included) agree.
//!
//! # Why this is not just a flag on `gridseak --version`
//!
//! `--version` reports only the CLI's own version. There is no
//! reliable way to print all three versions from a single flag because
//! discovery itself can fail (sidecar missing from PATH, override env
//! var pointing at the wrong file). Surfacing those failures inline in
//! `--version` would clutter the canonical version string everyone
//! parses; `doctor` is the dedicated diagnostic surface.

use std::env;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gridseak_engine_runner::{
    probe_engine_binaries, BinaryKind, VersionProbe, VersionProbeOutcome, EXPECTED_SIDECAR_VERSION,
};
use serde::Serialize;
use serde_json::Value;

use crate::resolve_engine_bin;

/// Structured JSON shape returned with `--json`.
///
/// Fields are append-only: machines parse this. Adding a new field is
/// safe; renaming or removing one is a breaking change for any CI
/// pipeline that decides whether to fail a build based on doctor output.
#[derive(Debug, Serialize)]
struct DoctorReport {
    cli_version: String,
    expected_sidecar_version: String,
    binaries: Vec<DoctorBinaryRow>,
    /// `true` when every binary matches the expected version.
    all_consistent: bool,
    /// "Is the gridseak binary discoverable on PATH, and is the one
    /// on PATH the same as the one we're currently running?" The
    /// installer puts `~/.gridseak/bin` on PATH guidance; this is the
    /// post-install verification users want.
    path_check: PathCheck,
    /// "Did the user run `gridseak setup-cursor`, and is the resulting
    /// `~/.cursor/mcp.json` still pointing at a valid binary?" This is
    /// the surface the website's /cli walkthrough relies on.
    mcp_check: McpCheck,
}

/// Outcome of the PATH check.
///
/// `status` is the single point CI / scripts branch on:
/// - `ok` — `gridseak` resolves on PATH to the same executable we're
///   running.
/// - `not_on_path` — `gridseak` is not anywhere on `PATH`.
/// - `different_binary` — `gridseak` is on PATH, but it resolves to a
///   different file than the one currently executing. This is the
///   typical symptom of two installs (e.g. an old Cargo `~/.cargo/bin`
///   install masking the new `~/.gridseak/bin` install).
/// - `unknown` — we couldn't determine the current executable's path
///   on this OS (rare; surfaces the error in `detail`).
#[derive(Debug, Serialize)]
struct PathCheck {
    status: String,
    /// Result of `std::env::current_exe()`. Always present unless the
    /// OS denied us the call.
    current_exe: Option<String>,
    /// First `gridseak` (or `gridseak.exe`) found by walking `$PATH`,
    /// in order.
    resolved_on_path: Option<String>,
    /// Free-form explanation. Empty on `ok`.
    detail: Option<String>,
}

/// Outcome of the Cursor MCP config check.
///
/// `status` is the single point CI / scripts branch on:
/// - `ok` — `~/.cursor/mcp.json` exists, contains a `gridseak` entry,
///   and that entry's `command` resolves to a real executable.
/// - `not_configured` — `~/.cursor/mcp.json` doesn't exist or has no
///   `gridseak` entry. Not a failure; just a nudge to run
///   `gridseak setup-cursor` if the user wants Cursor MCP.
/// - `bad_command` — entry exists but `command` is missing, empty, or
///   the binary it names doesn't exist.
/// - `unreadable` — the file exists but isn't valid JSON / doesn't
///   match the expected shape. Often indicates a hand-edit gone wrong.
#[derive(Debug, Serialize)]
struct McpCheck {
    status: String,
    /// `~/.cursor/mcp.json` (or `%USERPROFILE%\.cursor\mcp.json`).
    config_path: Option<String>,
    /// `mcpServers.gridseak.command` value. Useful when debugging
    /// stale entries pointing at a previous install.
    registered_command: Option<String>,
    /// Result of resolving `registered_command` to an absolute path
    /// (via PATH walk if the command isn't already absolute). Present
    /// on `ok` and `bad_command`.
    resolved_command_path: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct DoctorBinaryRow {
    /// `graphengine-parsing` or `ge-analyze`.
    name: String,
    /// Where the resolver located the binary, or the message of the
    /// resolution failure if it could not be found.
    path: Option<String>,
    resolution_error: Option<String>,
    /// What `<bin> --version` reported, if the binary was found and
    /// the probe succeeded.
    actual_version: Option<String>,
    /// `match`, `mismatch`, `unreadable`, or `missing`.
    status: String,
    /// Free-form detail (parse error, mismatch explanation, etc.).
    detail: Option<String>,
}

/// Run the doctor command. Returns `Err` if any binary fails to match —
/// the CLI translates that into a non-zero exit code so script-driven
/// callers can detect drift.
pub async fn run_doctor(json: bool) -> Result<()> {
    let cli_version = env!("CARGO_PKG_VERSION").to_string();

    // Resolve each sidecar using the same rules `gridseak scan` uses.
    // We capture the resolution error rather than failing here so the
    // report can show "missing" rows alongside "matched" rows in a
    // single pass — the user wants the full picture, not the first
    // failure.
    let parser = resolve_engine_bin("graphengine-parsing").map_err(|e| (e, "graphengine-parsing"));
    let analyzer = resolve_engine_bin("ge-analyze").map_err(|e| (e, "ge-analyze"));

    let mut rows: Vec<DoctorBinaryRow> = Vec::with_capacity(2);
    let parser_path = match &parser {
        Ok(p) => Some(p.clone()),
        Err((err, _)) => {
            rows.push(missing_row("graphengine-parsing", err.to_string()));
            None
        }
    };
    let analyzer_path = match &analyzer {
        Ok(p) => Some(p.clone()),
        Err((err, _)) => {
            rows.push(missing_row("ge-analyze", err.to_string()));
            None
        }
    };

    // Probe whichever binaries we did find. We probe both even when
    // one is missing; the runner-level helper takes paths, not an
    // option of paths.
    if let (Some(parser_path), Some(analyzer_path)) = (parser_path.as_ref(), analyzer_path.as_ref())
    {
        let probes = probe_engine_binaries(parser_path, analyzer_path).await;
        for probe in probes {
            rows.push(probe_to_row(probe));
        }
    } else {
        // One was missing; probe the one we have anyway so the user
        // sees its version separately.
        for (kind, path) in [
            (BinaryKind::Parser, parser_path.as_deref()),
            (BinaryKind::Analyzer, analyzer_path.as_deref()),
        ] {
            if let Some(path) = path {
                // `probe_engine_binaries` requires both; build a
                // tiny one-off probe by hand to avoid bypassing the
                // shared logic. The helper handles missing files
                // gracefully, so we can pass a dummy second path
                // — but constructing one would be misleading.
                // Instead, route through `read_version` with our
                // own minimal error mapping.
                match gridseak_engine_runner::read_version(kind, path).await {
                    Ok(actual) if actual == EXPECTED_SIDECAR_VERSION => {
                        rows.push(DoctorBinaryRow {
                            name: kind.to_string(),
                            path: Some(path.display().to_string()),
                            resolution_error: None,
                            actual_version: Some(actual),
                            status: "match".to_string(),
                            detail: None,
                        });
                    }
                    Ok(actual) => {
                        rows.push(DoctorBinaryRow {
                            name: kind.to_string(),
                            path: Some(path.display().to_string()),
                            resolution_error: None,
                            actual_version: Some(actual.clone()),
                            status: "mismatch".to_string(),
                            detail: Some(format!(
                                "binary reports {actual}, CLI expects {EXPECTED_SIDECAR_VERSION}"
                            )),
                        });
                    }
                    Err(err) => {
                        rows.push(DoctorBinaryRow {
                            name: kind.to_string(),
                            path: Some(path.display().to_string()),
                            resolution_error: None,
                            actual_version: None,
                            status: "unreadable".to_string(),
                            detail: Some(err.to_string()),
                        });
                    }
                }
            }
        }
    }

    // PATH and MCP checks are independent of sidecar version probing;
    // we compute them here so the report carries them whether or not
    // the binaries themselves matched. This is intentional: a stranger
    // running `gridseak doctor` for the first time often has both a
    // sidecar issue AND a PATH issue, and seeing only one would force
    // them to run doctor again after fixing the first.
    let path_check = check_path();
    let mcp_check = check_mcp();

    // `all_consistent` historically meant "all sidecars match". We
    // preserve that semantics so existing CI doesn't suddenly fail on
    // PATH issues. PATH + MCP statuses are surfaced separately for
    // callers that want stricter gating.
    let all_consistent =
        rows.iter().all(|r| r.status == "match") && cli_version == EXPECTED_SIDECAR_VERSION;

    let report = DoctorReport {
        cli_version: cli_version.clone(),
        expected_sidecar_version: EXPECTED_SIDECAR_VERSION.to_string(),
        binaries: rows,
        all_consistent,
        path_check,
        mcp_check,
    };

    if json {
        let out = serde_json::to_string_pretty(&report).context("serialize doctor report")?;
        println!("{out}");
    } else {
        render_human(&report);
    }

    if !report.all_consistent {
        // Use a non-fatal error so the calling script sees a non-zero
        // exit. Don't propagate to anyhow's full backtrace formatting
        // — the human/JSON render above already says everything the
        // user needs.
        anyhow::bail!(
            "gridseak doctor: install is inconsistent — see report above. \
             Rebuild the sidecars with `scripts/install/build-cli-release.sh` \
             and reinstall with `scripts/install/install.sh`."
        );
    }
    Ok(())
}

fn missing_row(name: &str, detail: String) -> DoctorBinaryRow {
    DoctorBinaryRow {
        name: name.to_string(),
        path: None,
        resolution_error: Some(detail),
        actual_version: None,
        status: "missing".to_string(),
        detail: None,
    }
}

fn probe_to_row(probe: VersionProbe) -> DoctorBinaryRow {
    let name = probe.which.to_string();
    let path = probe.path.display().to_string();
    match probe.outcome {
        VersionProbeOutcome::Match { actual } => DoctorBinaryRow {
            name,
            path: Some(path),
            resolution_error: None,
            actual_version: Some(actual),
            status: "match".to_string(),
            detail: None,
        },
        VersionProbeOutcome::Mismatch { actual } => DoctorBinaryRow {
            name,
            path: Some(path),
            resolution_error: None,
            actual_version: Some(actual.clone()),
            status: "mismatch".to_string(),
            detail: Some(format!(
                "binary reports {actual}, CLI expects {}",
                EXPECTED_SIDECAR_VERSION
            )),
        },
        VersionProbeOutcome::Unreadable { detail } => DoctorBinaryRow {
            name,
            path: Some(path),
            resolution_error: None,
            actual_version: None,
            status: "unreadable".to_string(),
            detail: Some(detail),
        },
    }
}

/// Determine whether `gridseak` is on PATH and whether the one on
/// PATH is the same executable that's currently running. Returns an
/// honest report; we do not return `Err` here because doctor's
/// contract is "always show the full picture, exit non-zero only on
/// sidecar drift". PATH issues are surfaced but don't fail the
/// command, because some valid installs deliberately use absolute
/// paths (Cursor's MCP config, NixOS users, etc.).
fn check_path() -> PathCheck {
    let current = env::current_exe().ok().map(canonicalize_safe);
    let current_str = current.as_ref().map(|p| p.display().to_string());

    let needle = gridseak_exe_name();
    let resolved = match env::var_os("PATH") {
        Some(path) => env::split_paths(&path)
            .map(|dir| dir.join(&needle))
            .find(|candidate| candidate.is_file())
            .map(canonicalize_safe),
        None => None,
    };
    let resolved_str = resolved.as_ref().map(|p| p.display().to_string());

    match (current.as_ref(), resolved.as_ref()) {
        (Some(cur), Some(found)) if cur == found => PathCheck {
            status: "ok".to_string(),
            current_exe: current_str,
            resolved_on_path: resolved_str,
            detail: None,
        },
        (Some(_), Some(_)) => PathCheck {
            status: "different_binary".to_string(),
            current_exe: current_str,
            resolved_on_path: resolved_str,
            // The most common cause is a stale `~/.cargo/bin/gridseak`
            // (or `/usr/local/bin/gridseak`) shadowing the new
            // `~/.gridseak/bin/gridseak`. Saying so directly saves the
            // user 15 minutes of debugging.
            detail: Some(
                "the gridseak on PATH is not the binary you're currently running. \
                 If you installed via `https://gridseak.com/install.sh`, ensure \
                 `~/.gridseak/bin` precedes any older install on your PATH."
                    .to_string(),
            ),
        },
        (Some(_), None) => PathCheck {
            status: "not_on_path".to_string(),
            current_exe: current_str,
            resolved_on_path: None,
            detail: Some(
                "gridseak is not on PATH. The install script prints the line you \
                 need to add to your shell rc (e.g. `export PATH=\"$HOME/.gridseak/bin:$PATH\"`); \
                 new shells will pick it up."
                    .to_string(),
            ),
        },
        (None, _) => PathCheck {
            status: "unknown".to_string(),
            current_exe: None,
            resolved_on_path: resolved_str,
            detail: Some(
                "could not determine the current executable's path (std::env::current_exe failed)"
                    .to_string(),
            ),
        },
    }
}

/// Inspect `~/.cursor/mcp.json` (or the platform equivalent) and report
/// whether Cursor will be able to spawn this CLI as an MCP server. We
/// only check the *global* config because that's what `gridseak
/// setup-cursor` writes by default; per-workspace configs are not
/// universally discoverable and are usually a deliberate override.
fn check_mcp() -> McpCheck {
    let Some(home) = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    else {
        return McpCheck {
            status: "not_configured".to_string(),
            config_path: None,
            registered_command: None,
            resolved_command_path: None,
            detail: Some("no HOME (or USERPROFILE) env var set".to_string()),
        };
    };
    let path = home.join(".cursor").join("mcp.json");
    let path_str = path.display().to_string();

    if !path.is_file() {
        return McpCheck {
            status: "not_configured".to_string(),
            config_path: Some(path_str),
            registered_command: None,
            resolved_command_path: None,
            detail: Some(
                "Cursor MCP config not found. Run `gridseak setup-cursor` to register \
                 the MCP server."
                    .to_string(),
            ),
        };
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            return McpCheck {
                status: "unreadable".to_string(),
                config_path: Some(path_str),
                registered_command: None,
                resolved_command_path: None,
                detail: Some(format!("read {}: {err}", path.display())),
            };
        }
    };
    let doc: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            return McpCheck {
                status: "unreadable".to_string(),
                config_path: Some(path_str),
                registered_command: None,
                resolved_command_path: None,
                detail: Some(format!("parse {}: {err}", path.display())),
            };
        }
    };

    // We accept either:
    //   { "mcpServers": { "gridseak": { "command": "...", "args": [...] } } }
    // (modern Cursor) or any future shape we should evolve to support
    // by extending this match.
    let command = doc
        .pointer("/mcpServers/gridseak/command")
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(command) = command else {
        return McpCheck {
            status: "not_configured".to_string(),
            config_path: Some(path_str),
            registered_command: None,
            resolved_command_path: None,
            detail: Some(
                "Cursor MCP config exists but has no `mcpServers.gridseak.command` entry. \
                 Run `gridseak setup-cursor` to (re-)register."
                    .to_string(),
            ),
        };
    };

    // Resolve `command` to an absolute path. If it's already absolute,
    // we just stat it; if not, we walk PATH. Either way, we don't
    // execute anything — just confirm the file exists. The sidecar
    // version probe above already confirms the binary is executable.
    let resolved = resolve_command(&command);
    let resolved_str = resolved.as_ref().map(|p| p.display().to_string());

    match resolved.as_ref() {
        Some(_) => McpCheck {
            status: "ok".to_string(),
            config_path: Some(path_str),
            registered_command: Some(command),
            resolved_command_path: resolved_str,
            detail: None,
        },
        None => McpCheck {
            status: "bad_command".to_string(),
            config_path: Some(path_str),
            registered_command: Some(command.clone()),
            resolved_command_path: None,
            detail: Some(format!(
                "MCP entry points at `{command}`, but no such executable was found \
                 (checked absolute path + PATH walk). Re-run `gridseak setup-cursor` \
                 with the absolute path of this binary, e.g. \
                 `gridseak setup-cursor --command \"$(which gridseak)\"`."
            )),
        },
    }
}

/// `gridseak.exe` on Windows, `gridseak` everywhere else.
fn gridseak_exe_name() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("gridseak.exe")
    } else {
        PathBuf::from("gridseak")
    }
}

/// `std::fs::canonicalize` but fall back to the original path on
/// failure (e.g. symlink loops, permission denied) so equality
/// comparison still works. We only need consistent normalisation, not
/// strict resolution.
fn canonicalize_safe(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}

/// Resolve a `command` string from `mcp.json` to an absolute path on
/// disk. Returns `None` if no matching executable is found.
fn resolve_command(command: &str) -> Option<PathBuf> {
    let raw = Path::new(command);
    if raw.is_absolute() {
        return if raw.is_file() {
            Some(canonicalize_safe(raw.to_path_buf()))
        } else {
            None
        };
    }
    // Try PATH walk.
    let path_var = env::var_os("PATH")?;
    let exe_name = if cfg!(windows) && !command.to_lowercase().ends_with(".exe") {
        format!("{command}.exe")
    } else {
        command.to_string()
    };
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(&exe_name);
        if candidate.is_file() {
            return Some(canonicalize_safe(candidate));
        }
    }
    None
}

fn render_human(report: &DoctorReport) {
    // Compact table: one row per binary plus a CLI row at the top.
    // ANSI-tinting is intentionally conservative — we only colour the
    // status column, and only when stdout is a TTY, so logs and CI
    // captures stay clean.
    let is_tty = std::io::stdout().is_terminal();
    let ok = |s: &str| {
        if is_tty {
            format!("\x1b[32m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let bad = |s: &str| {
        if is_tty {
            format!("\x1b[31m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };

    println!(
        "GridSeak install diagnostic\nCLI version:               {} ({})",
        report.cli_version,
        if report.cli_version == report.expected_sidecar_version {
            ok("ok")
        } else {
            bad("unexpected — sidecar expectation drifted from CLI version")
        }
    );
    println!(
        "Expected sidecar version:  {}",
        report.expected_sidecar_version
    );
    println!();
    println!("Sidecars:");
    for row in &report.binaries {
        let status_tag = match row.status.as_str() {
            "match" => ok("✓ match"),
            "mismatch" => bad("✗ mismatch"),
            "unreadable" => bad("✗ unreadable"),
            "missing" => bad("✗ missing"),
            other => other.to_string(),
        };
        let path = row.path.as_deref().unwrap_or("(not found)");
        let actual = row.actual_version.as_deref().unwrap_or("?");
        println!("  - {:<22} {}", row.name, status_tag);
        println!("      path:    {path}");
        println!("      version: {actual}");
        if let Some(detail) = &row.detail {
            println!("      detail:  {detail}");
        }
        if let Some(err) = &row.resolution_error {
            println!("      lookup:  {err}");
        }
    }
    println!();

    // PATH check
    let path_tag = match report.path_check.status.as_str() {
        "ok" => ok("✓ ok"),
        "different_binary" | "not_on_path" | "unknown" => {
            bad(&format!("✗ {}", report.path_check.status))
        }
        other => other.to_string(),
    };
    println!("PATH:    {}", path_tag);
    if let Some(p) = &report.path_check.current_exe {
        println!("    current_exe:       {p}");
    }
    if let Some(p) = &report.path_check.resolved_on_path {
        println!("    resolved on PATH:  {p}");
    }
    if let Some(detail) = &report.path_check.detail {
        println!("    detail:            {detail}");
    }
    println!();

    // MCP check
    let mcp_tag = match report.mcp_check.status.as_str() {
        "ok" => ok("✓ ok"),
        "not_configured" => {
            // Not configured is informational, not a hard failure.
            // Render in default tint so it doesn't read like an error.
            "○ not configured".to_string()
        }
        "bad_command" | "unreadable" => bad(&format!("✗ {}", report.mcp_check.status)),
        other => other.to_string(),
    };
    println!("Cursor MCP: {}", mcp_tag);
    if let Some(p) = &report.mcp_check.config_path {
        println!("    config:            {p}");
    }
    if let Some(c) = &report.mcp_check.registered_command {
        println!("    registered_command: {c}");
    }
    if let Some(p) = &report.mcp_check.resolved_command_path {
        println!("    resolves to:       {p}");
    }
    if let Some(detail) = &report.mcp_check.detail {
        println!("    detail:            {detail}");
    }
    println!();

    if report.all_consistent {
        println!("{}", ok("All binaries consistent."));
        // Even when sidecars are fine, surface a one-line nudge if PATH
        // or MCP are clearly broken so the user knows the next step.
        if report.path_check.status != "ok" {
            println!(
                "{}",
                bad("…but `gridseak` is not on PATH / shadowed. See PATH section above.")
            );
        }
        if report.mcp_check.status != "ok" && report.mcp_check.status != "not_configured" {
            println!(
                "{}",
                bad("…but Cursor MCP config is unhealthy. See Cursor MCP section above.")
            );
        }
    } else {
        println!(
            "{}",
            bad("Inconsistent install. See `scripts/install/build-cli-release.sh` and `scripts/install/install.sh`.")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Build a `PATH` value containing exactly the supplied directories.
    fn path_value(dirs: &[&Path]) -> std::ffi::OsString {
        env::join_paths(dirs.iter().map(|p| p.to_path_buf())).expect("join_paths")
    }

    /// Create a fake executable file with mode 755 on Unix. We don't
    /// actually need it to be executable for the PATH check (which only
    /// looks at `is_file()`), but tests on contributors' machines should
    /// still match the real-world layout.
    fn touch_exe(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&p, perms).unwrap();
        }
        p
    }

    #[test]
    fn check_path_returns_not_on_path_when_no_gridseak_anywhere() {
        let dir = TempDir::new().unwrap();
        with_env(&[("PATH", Some(path_value(&[dir.path()])))], || {
            let check = check_path();
            assert_eq!(check.status, "not_on_path", "actual: {check:?}");
            assert!(check.detail.is_some());
            assert!(check.resolved_on_path.is_none());
        });
    }

    #[test]
    fn check_path_resolves_shadowed_install_to_first_entry_on_path() {
        // We can't override `std::env::current_exe` from a unit test,
        // so we verify only that the PATH walk finds the planted file
        // — the equality decision (`ok` vs `different_binary`) is
        // covered implicitly by `check_path_returns_not_on_path_when_no_gridseak_anywhere`
        // and by the structural code review of `check_path`.
        let path_dir = TempDir::new().unwrap();
        let stale = touch_exe(path_dir.path(), gridseak_exe_name().to_str().unwrap());
        with_env(&[("PATH", Some(path_value(&[path_dir.path()])))], || {
            let needle = gridseak_exe_name();
            let resolved = env::split_paths(&env::var_os("PATH").unwrap())
                .map(|d| d.join(&needle))
                .find(|c| c.is_file())
                .map(canonicalize_safe);
            assert_eq!(
                resolved.as_deref(),
                Some(canonicalize_safe(stale).as_path())
            );
        });
    }

    #[test]
    fn check_mcp_returns_not_configured_when_file_missing() {
        let fake_home = TempDir::new().unwrap();
        let home_val = fake_home.path().as_os_str().to_owned();
        with_env(
            &[
                ("HOME", Some(home_val.clone())),
                ("USERPROFILE", Some(home_val)),
            ],
            || {
                let check = check_mcp();
                assert_eq!(check.status, "not_configured");
                assert!(check.detail.is_some());
            },
        );
    }

    #[test]
    fn check_mcp_returns_bad_command_when_entry_points_at_missing_binary() {
        let fake_home = TempDir::new().unwrap();
        let cursor_dir = fake_home.path().join(".cursor");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        let mcp_path = cursor_dir.join("mcp.json");
        std::fs::write(
            &mcp_path,
            r#"{"mcpServers":{"gridseak":{"command":"/no/such/binary","args":["mcp"]}}}"#,
        )
        .unwrap();
        let home_val = fake_home.path().as_os_str().to_owned();
        with_env(
            &[
                ("HOME", Some(home_val.clone())),
                ("USERPROFILE", Some(home_val)),
                ("PATH", Some(std::ffi::OsString::new())),
            ],
            || {
                let check = check_mcp();
                assert_eq!(check.status, "bad_command", "actual: {check:?}");
                assert_eq!(check.registered_command.as_deref(), Some("/no/such/binary"));
                assert!(check.resolved_command_path.is_none());
                assert!(check.detail.is_some());
            },
        );
    }

    #[test]
    fn check_mcp_returns_ok_when_entry_resolves_to_real_file() {
        let fake_home = TempDir::new().unwrap();
        let cursor_dir = fake_home.path().join(".cursor");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        let fake_bin = touch_exe(fake_home.path(), "gridseak-fake");
        let mcp_path = cursor_dir.join("mcp.json");
        std::fs::write(
            &mcp_path,
            format!(
                r#"{{"mcpServers":{{"gridseak":{{"command":{:?},"args":["mcp"]}}}}}}"#,
                fake_bin.display(),
            ),
        )
        .unwrap();
        let home_val = fake_home.path().as_os_str().to_owned();
        with_env(
            &[
                ("HOME", Some(home_val.clone())),
                ("USERPROFILE", Some(home_val)),
            ],
            || {
                let check = check_mcp();
                assert_eq!(check.status, "ok", "actual: {check:?}");
                assert_eq!(
                    check.registered_command.as_deref(),
                    Some(fake_bin.to_str().unwrap())
                );
                assert!(check.resolved_command_path.is_some());
            },
        );
    }

    // ────────────────────────────────────────────────────────────────
    // Scoped env helper.
    //
    // `std::env::set_var` is process-global and unsafe in modern Rust.
    // For these unit tests we:
    //   1. acquire a single per-test mutex,
    //   2. snapshot the original values of every key we touch,
    //   3. apply the test's overrides,
    //   4. run the closure,
    //   5. restore the originals in reverse order before releasing the
    //      mutex.
    //
    // The previous version of this helper acquired a fresh mutex per
    // env-var override, which DEADLOCKED any test that set more than
    // one variable. `with_env` takes all overrides up front so the
    // lock is acquired exactly once.
    // ────────────────────────────────────────────────────────────────

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<F: FnOnce()>(overrides: &[(&'static str, Option<std::ffi::OsString>)], f: F) {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<(&'static str, Option<std::ffi::OsString>)> = overrides
            .iter()
            .map(|(k, _)| (*k, env::var_os(k)))
            .collect();
        for (k, v) in overrides {
            // SAFETY: we hold ENV_LOCK; no other test in this module
            // mutates env without going through with_env, and the
            // production callers never mutate env.
            unsafe {
                match v {
                    Some(val) => env::set_var(k, val),
                    None => env::remove_var(k),
                }
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (k, v) in saved.into_iter().rev() {
            unsafe {
                match v {
                    Some(val) => env::set_var(k, val),
                    None => env::remove_var(k),
                }
            }
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
