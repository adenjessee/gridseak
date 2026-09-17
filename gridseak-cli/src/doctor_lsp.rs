//! `gridseak doctor` — LSP server dependency probe.
//!
//! # What this answers
//!
//! "For each language this build knows about, is its language server
//! actually installed and locatable?"
//!
//! `gridseak doctor` already proves the CLI, sidecars, and (now)
//! configs agree. But a perfectly consistent install still produces
//! `syntactic_only` scans if `pyright-langserver`, `typescript-language-server`,
//! `jdtls`, or `gopls` aren't on the machine. This probe makes that
//! explicit instead of letting it surface as a silent heuristic
//! fallback at scan time.
//!
//! # Resolution rule (shared with runtime)
//!
//! Availability is decided by the single canonical resolver
//! [`graphengine_parsing::infrastructure::lsp::command_locator::is_command_available`]
//! — path existence / `PATH` resolution only, never by running the
//! binary. This is the same function the LSP client uses at startup, so
//! the doctor cannot report "available" for a server the client would
//! reject (or vice versa).
//!
//! Missing servers are informational, not a hard failure: a user who
//! only scans Python should not have `doctor` fail because `gopls`
//! isn't installed.

use std::collections::BTreeMap;
use std::path::Path;

use graphengine_parsing::infrastructure::config::{
    get_available_languages, load_language_descriptor, set_configs_dir_override,
};
use graphengine_parsing::infrastructure::lsp::command_locator::resolve_executable;
use serde::Serialize;

/// One row per distinct LSP server command, listing which languages use
/// it (e.g. `typescript-language-server` serves both TS and JS).
#[derive(Debug, Serialize)]
pub struct LspServerRow {
    pub command: String,
    pub languages: Vec<String>,
    /// `found` or `missing`.
    pub status: String,
    pub resolved_path: Option<String>,
    pub note: Option<String>,
}

/// Result of the LSP dependency probe.
///
/// `status`:
/// - `ok` — the configs resolved and every server was probed (some may
///   be `missing`, which is informational).
/// - `configs_unresolved` — no configs dir, so we cannot know which
///   servers to look for. The configs check reports the root cause.
#[derive(Debug, Serialize)]
pub struct LspServersCheck {
    pub status: String,
    pub servers: Vec<LspServerRow>,
    pub detail: Option<String>,
}

/// Probe the LSP server for every language declared in `configs_dir`.
///
/// Reads each language descriptor through the parser's own loader (so
/// it sees exactly what a scan would), groups languages by their
/// declared `lsp_command`, and resolves each command with the canonical
/// availability rule.
pub fn check_lsp_servers(configs_dir: Option<&Path>) -> LspServersCheck {
    let Some(dir) = configs_dir else {
        return LspServersCheck {
            status: "configs_unresolved".to_string(),
            servers: Vec::new(),
            detail: Some(
                "skipped: no configs directory resolved, so the set of expected LSP \
                 servers is unknown. See the Configs section."
                    .to_string(),
            ),
        };
    };

    // Point the parser's loader at the doctor-resolved dir. `set_..` is a
    // one-shot OnceLock; ignoring the error is correct because if it is
    // already set (another command ran earlier in-process) the value is
    // the one we want anyway.
    let _ = set_configs_dir_override(dir.to_path_buf());

    let languages = match get_available_languages() {
        Ok(langs) => langs,
        Err(err) => {
            return LspServersCheck {
                status: "configs_unresolved".to_string(),
                servers: Vec::new(),
                detail: Some(format!("could not enumerate language configs: {err}")),
            };
        }
    };

    // command -> sorted set of languages that declare it.
    let mut by_command: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for language in languages {
        let Ok(descriptor) = load_language_descriptor(&language) else {
            continue;
        };
        if descriptor.discovery_only {
            continue;
        }
        if let Some(command) = descriptor.lsp_command.filter(|c| !c.trim().is_empty()) {
            by_command.entry(command).or_default().push(language);
        }
    }

    let mut servers = Vec::with_capacity(by_command.len());
    for (command, mut languages) in by_command {
        languages.sort();
        languages.dedup();

        let (status, resolved_path) = match resolve_executable(&command) {
            Ok(path) => ("found".to_string(), Some(path.display().to_string())),
            Err(_) => ("missing".to_string(), None),
        };

        // Apex declares `java` as its command, but the server is the
        // vendored jorje jar driven by java. Flag that so a user with
        // java but no jar isn't misled by a green "found".
        let note = if command == "java" && languages.iter().any(|l| l == "apex") {
            Some(
                "Apex runs jorje via java and also needs apex-jorje-lsp.jar \
                 (bundled by the desktop installer, or set GRAPHENGINE_APEX_JORJE_JAR)."
                    .to_string(),
            )
        } else {
            None
        };

        servers.push(LspServerRow {
            command,
            languages,
            status,
            resolved_path,
            note,
        });
    }

    LspServersCheck {
        status: "ok".to_string(),
        servers,
        detail: None,
    }
}
