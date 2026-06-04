//! `gridseak feedback "<text>"` — append a free-form note to the local
//! `feedback` table.
//!
//! Stage 12 of the shadow-mode plan: the CLI signals what is free and
//! what would be paid, then gives the reader a one-shot command to say
//! "this is what would unlock me". This is intentionally **local-only**.
//! Nothing is sent over the network; the row sits in the user's local
//! ProjectStore (`feedback` table) until the user opts to export.
//! That keeps the trust contract simple — the hero report is the only
//! signal flowing out of the user's machine, and only when they hand
//! it over themselves.
//!
//! Project association is best-effort: if the user is inside a known
//! project root we record the project id; otherwise the row stays
//! unattached. Either way the row is captured.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::Args;
use gridseak_local_store::ProjectStore;

/// Arguments for `gridseak feedback "<text>"`.
#[derive(Args, Debug)]
pub struct FeedbackArgs {
    /// Free-form feedback text. Wrap in quotes; whitespace is preserved
    /// verbatim. The body is stored as-is; there is no character cap
    /// because long-form feedback is typically the most useful.
    pub text: String,

    /// Optional explicit project to attach the row to. Defaults to the
    /// current directory; pass `--no-project` to skip attachment.
    #[arg(default_value = ".")]
    pub project: String,

    /// Do not try to associate the feedback with any project. Useful
    /// when reporting "the CLI itself" rather than a specific scan.
    #[arg(long)]
    pub no_project: bool,

    /// List existing feedback rows after writing the new one. Useful
    /// during local development; in production the rows are private.
    #[arg(long)]
    pub list: bool,

    #[arg(long, hide = true)]
    pub data_dir: Option<PathBuf>,

    /// Emit a small JSON envelope describing the row id + timestamp.
    /// Off by default so the CLI's stdout stays human; agents that
    /// want the id should pass `--json`.
    #[arg(long)]
    pub json: bool,
}

/// Drive the `feedback` command end-to-end.
///
/// `app_version` is threaded through from `main.rs` so the recorded
/// row knows which CLI build wrote it. Using `env!("CARGO_PKG_VERSION")`
/// here directly would tie the recorded version to the CLI crate,
/// which is fine, but going through `main.rs` keeps the version
/// resolution in one place.
pub fn run_feedback(
    store: &ProjectStore,
    args: FeedbackArgs,
    app_version: &str,
    global_json: bool,
) -> Result<()> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err(anyhow!(
            "feedback text is empty. example: gridseak feedback \"would pay for a CI gate\""
        ));
    }

    let project_id = if args.no_project {
        None
    } else {
        match store.resolve_project(&args.project) {
            Ok(p) => Some(p.id),
            Err(_) => None,
        }
    };

    let id = store
        .record_feedback(project_id.as_deref(), text, "cli", app_version)
        .with_context(|| "recording feedback")?;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    let emit_json = args.json || global_json;
    if emit_json {
        let project_id_value = match &project_id {
            Some(id) => format!("\"{id}\""),
            None => "null".into(),
        };
        writeln!(
            out,
            "{{\"id\": \"{id}\", \"project_id\": {project_id_value}, \"recorded\": true}}"
        )?;
    } else {
        writeln!(out, "Thanks. Feedback recorded locally.")?;
        writeln!(out, "  id:         {id}")?;
        if let Some(pid) = &project_id {
            writeln!(out, "  project:    {pid}")?;
        } else if !args.no_project {
            writeln!(
                out,
                "  project:    (none — `{}` is not a known project root)",
                args.project
            )?;
        }
        writeln!(out, "  stored in:  local ProjectStore / feedback table")?;
        writeln!(out)?;
        writeln!(
            out,
            "Nothing was sent over the network. The row stays on this"
        )?;
        writeln!(out, "machine until you decide to share it.")?;
    }

    if args.list {
        let rows = store.list_feedback()?;
        if emit_json {
            // Best-effort secondary blob. The first JSON line already
            // confirmed the new row; this listing is informational.
            writeln!(out, "[")?;
            for (i, row) in rows.iter().enumerate() {
                let project = match &row.project_id {
                    Some(p) => format!("\"{p}\""),
                    None => "null".into(),
                };
                let comma = if i + 1 == rows.len() { "" } else { "," };
                writeln!(
                    out,
                    "  {{\"id\":\"{}\",\"project_id\":{},\"created_at\":\"{}\",\"app_version\":\"{}\",\"source\":\"{}\",\"text\":{}}}{}",
                    row.id,
                    project,
                    row.created_at,
                    row.app_version,
                    row.source,
                    json_quote(&row.text),
                    comma
                )?;
            }
            writeln!(out, "]")?;
        } else {
            writeln!(out)?;
            writeln!(out, "All feedback on this machine ({} rows):", rows.len())?;
            for row in &rows {
                writeln!(
                    out,
                    "  {} [{}] {} {}",
                    row.created_at,
                    row.source,
                    row.id,
                    row.text.lines().next().unwrap_or("")
                )?;
            }
        }
    }
    Ok(())
}

fn json_quote(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}
