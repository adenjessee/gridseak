//! GraphSeak Desktop - Graph Engine Parsing CLI
//!
//! A command-line tool for parsing code repositories and extracting
//! semantic graphs with high-confidence relationships.

use clap::{Parser, Subcommand};
use graphengine_parsing::application::errors::ParsingError;
use graphengine_parsing::application::ports::GraphRepository;
use graphengine_parsing::application::use_cases::ParseRepositoryUseCase;
use graphengine_parsing::domain::Confidence;
use graphengine_parsing::infrastructure::config::{
    get_available_languages, load_config, load_language_descriptor, set_config_file_override,
    set_configs_dir_override,
};
use graphengine_parsing::infrastructure::lsp::command_locator::resolve_lsp_command;
use graphengine_parsing::infrastructure::SqliteRepository;
use graphengine_progress::{EngineEvent, EngineEventEmitter, StdoutEngineEventEmitter};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser)]
#[command(name = "graphengine-parsing")]
#[command(about = "Parse code repositories and extract semantic graphs")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Directory containing `configs/<language>.yaml` files (overrides auto-resolution)
    #[arg(long, global = true)]
    configs_dir: Option<PathBuf>,

    /// Explicit language config YAML file (overrides configs_dir + auto-resolution)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse a repository and extract its semantic graph
    Parse {
        /// Root directory of the repository to parse
        #[arg(short, long)]
        root: PathBuf,

        /// Programming language to parse
        #[arg(short, long)]
        lang: String,

        /// Output database path
        #[arg(short, long)]
        db: Option<PathBuf>,

        /// Minimum confidence threshold (0.0-1.0)
        #[arg(long)]
        min_confidence: Option<f32>,

        /// Disable incremental parsing (force full reparse). S1 ships
        /// incremental scanning as the default: discovered files are
        /// blake3-hashed and matched against the previous parse DB's
        /// `file_cache` rows; cache hits skip re-extraction. Pass
        /// `--no-incremental` to bypass the cache and reparse every
        /// file — useful for verification, or after a corruption is
        /// suspected.
        #[arg(long = "no-incremental")]
        no_incremental: bool,

        /// Clear database before parsing
        #[arg(long)]
        clear: bool,

        /// Export format (json, db)
        #[arg(short, long, default_value = "db")]
        output: String,

        /// Emit machine-readable progress events as JSON Lines (JSONL) to stdout.
        ///
        /// Intended for desktop sidecar integrations (e.g. Unreal) so the UI can show
        /// deterministic progress without scraping human logs.
        #[arg(long)]
        progress_json: bool,

        /// Write end-of-scan LSP resolution telemetry as a single JSON
        /// document to the given path.
        ///
        /// The payload mirrors the counters persisted to the database
        /// (`resolution_lsp_edges`, `resolution_heuristic_edges`,
        /// `resolution_heuristic_call_fallbacks`, etc.) and adds
        /// derived fields (`fallback_rate`, `total_resolution_work`,
        /// `scan_duration_ms`). This file is the source-of-truth input
        /// for the `ResolutionDegraded` analysis finding (Sprint D.2)
        /// when the downstream analyzer runs on a different host that
        /// doesn't have direct DB access — e.g. CI pipelines that
        /// upload only the telemetry JSON.
        #[arg(long, value_name = "PATH")]
        lsp_telemetry: Option<PathBuf>,
    },

    /// Query the parsed graph database
    Query {
        /// Database path
        #[arg(short, long)]
        db: PathBuf,

        /// Query type
        #[arg(short, long)]
        query_type: String,

        /// Query parameters
        #[arg(short, long)]
        params: Vec<String>,
    },

    /// List supported programming languages
    Languages {
        /// Emit machine-readable JSON (array of `{language, file_extensions, lsp_command}`)
        /// instead of the human-readable table. Desktop and CI consumers use this to
        /// stay in lock-step with the YAML configs (single source of truth).
        #[arg(long)]
        json: bool,
    },

    /// Show database statistics
    Stats {
        /// Database path
        #[arg(short, long)]
        db: PathBuf,
    },

    /// Check LSP server availability for configured languages
    Doctor {
        /// Optional language to check (defaults to all)
        #[arg(short, long)]
        language: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Config resolution overrides must be applied early so all components see them.
    if let Some(dir) = cli.configs_dir.clone() {
        set_configs_dir_override(dir)?;
    }
    if let Some(file) = cli.config.clone() {
        set_config_file_override(file)?;
    }

    // Initialize tracing
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    // Logs go to stderr so stdout stays reserved for machine-readable payloads
    // (e.g. `languages --json`, `parse --progress-json`). Any consumer parsing
    // stdout would otherwise choke on interleaved tracing lines.
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Commands::Parse {
            root,
            lang,
            db,
            min_confidence,
            no_incremental,
            clear,
            output,
            progress_json,
            lsp_telemetry,
        } => {
            parse_command(
                root,
                lang,
                db,
                min_confidence,
                no_incremental,
                clear,
                output,
                progress_json,
                lsp_telemetry,
            )
            .await
        }
        Commands::Query {
            db,
            query_type,
            params,
        } => query_command(db, query_type, params).await,
        Commands::Languages { json } => languages_command(json).await,
        Commands::Stats { db } => stats_command(db).await,
        Commands::Doctor { language } => doctor_command(language).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn parse_command(
    root: PathBuf,
    lang: String,
    db: Option<PathBuf>,
    min_confidence: Option<f32>,
    no_incremental: bool,
    clear: bool,
    output: String,
    progress_json: bool,
    lsp_telemetry: Option<PathBuf>,
) -> anyhow::Result<()> {
    // Parser writes structured progress to **stdout** because its
    // tracing logs go to stderr. R3 consolidated the emitter
    // abstraction into `graphengine-progress`; the wire format is
    // unchanged so the runner / CLI consumers see identical bytes.
    let emitter: Arc<dyn EngineEventEmitter> =
        Arc::new(StdoutEngineEventEmitter::to_stdout(progress_json));

    let emit = {
        let emitter = Arc::clone(&emitter);
        move |percent: u8, phase: &str, status: &str, message: &str| -> anyhow::Result<()> {
            emitter
                .emit(EngineEvent::progress(percent, phase, status, message))
                .map_err(|e| anyhow::anyhow!(e))
        }
    };

    emit(0, "parse", "start", "Starting parse")?;
    info!(
        "Starting parse of {} repository at {}",
        lang,
        root.display()
    );

    // Refuse discovery-only languages up-front with a clear, actionable
    // message instead of letting the strict `load_config` validation surface
    // a confusing YAML-schema error three frames deeper. Discovery-only
    // configs (e.g. Visualforce) are absorbed by another language's pipeline
    // (the Apex parser reads `.page` files via `vf_page_reader`), so asking
    // us to parse one directly is a caller mistake. The shell filters these
    // out before getting here; this guard exists for CLI/scripted users and
    // as defense in depth.
    if let Ok(desc) = load_language_descriptor(&lang) {
        if desc.discovery_only {
            anyhow::bail!(
                "language '{lang}' is discovery-only (no standalone parser); \
                 it is processed as part of the host language's pipeline \
                 (e.g. Visualforce is read by the Apex parser). \
                 Omit it from --lang and parse the host language instead."
            );
        }
    }

    // Determine database path
    let db_path = db.unwrap_or_else(|| root.join(format!("{}.db", lang)));

    // Convert confidence threshold
    let confidence = min_confidence
        .map(|f| {
            if f <= 0.0 {
                Confidence::Low
            } else if f <= 0.5 {
                Confidence::Medium
            } else {
                Confidence::High
            }
        })
        .unwrap_or(Confidence::Medium);

    // Clear database if requested
    if clear {
        emit(2, "db", "clear", "Clearing database")?;
        info!("Clearing database before parsing...");
        let repo = SqliteRepository::new(&db_path.to_string_lossy())?;
        repo.clear().await?;
    }

    // Create use case with REAL Tree-sitter + REAL LSP components
    // Canonicalize the root path to an absolute path for URL construction
    emit(5, "canonicalize", "start", "Canonicalizing root path")?;
    info!(
        "Canonicalizing root path: {} (this may take a moment for large repositories)...",
        root.display()
    );
    let canonical_start = std::time::Instant::now();
    let canonical_root = root.canonicalize().map_err(|e| {
        ParsingError::config(format!(
            "Failed to canonicalize root path '{}': {}",
            root.display(),
            e
        ))
    })?;
    let canonical_duration = canonical_start.elapsed();
    if canonical_duration.as_secs() > 1 {
        warn!(
            "Path canonicalization took {:?} - consider using absolute paths directly",
            canonical_duration
        );
    } else {
        info!("Path canonicalized in {:?}", canonical_duration);
    }
    emit(10, "canonicalize", "done", "Canonicalization complete")?;
    let workspace_url = url::Url::from_file_path(&canonical_root).ok();
    let use_case = ParseRepositoryUseCase::with_real_components_progress(
        lang.clone(),
        confidence,
        &db_path.to_string_lossy(),
        workspace_url,
        Arc::clone(&emitter),
    )
    .await?;

    // Parse the repository. S1 incremental scanning is on by default;
    // `--no-incremental` flips the orchestrator's `ParseOptions` to
    // force a full reparse, bypassing the `file_cache` lookup.
    emit(15, "pipeline", "start", "Executing parsing pipeline")?;
    let start_time = std::time::Instant::now();
    let parse_options = graphengine_parsing::application::use_cases::parse_repo::pipeline::orchestrator::ParseOptions {
        incremental: !no_incremental,
    };
    let result = use_case
        .parse_with_options(root.clone(), lang.clone(), parse_options)
        .await;
    let duration = start_time.elapsed();

    match result {
        Ok(resolved_graph) => {
            let graph = resolved_graph.graph();
            let stats = resolved_graph.stats();
            emit(
                90,
                "pipeline",
                "done",
                &format!(
                    "Pipeline complete (nodes={}, edges={}, duration_ms={})",
                    graph.node_count(),
                    graph.edge_count(),
                    duration.as_millis()
                ),
            )?;
            info!("Parse completed successfully in {:?}", duration);
            info!(
                "Nodes: {}, Edges: {}",
                graph.node_count(),
                graph.edge_count()
            );

            // Calculate confidence statistics
            let total_edges = graph.edge_count();
            let high_confidence_edges = graph
                .edges
                .iter()
                .filter(|e| e.provenance.confidence == Confidence::High)
                .count();
            let confidence_percentage = if total_edges > 0 {
                (high_confidence_edges as f32 / total_edges as f32) * 100.0
            } else {
                100.0
            };

            info!(
                "High confidence edges: {}/{} ({:.1}%)",
                high_confidence_edges, total_edges, confidence_percentage
            );

            info!(
                "Call provenance coverage: LSP={} Heuristic={} (total {})",
                stats.lsp_edges,
                stats.heuristic_edges,
                stats.total_call_edges()
            );
            if !stats.lsp_failures.is_empty() {
                warn!(
                    "{} LSP call-resolution failures; sample: {:?}",
                    stats.lsp_failures.len(),
                    &stats.lsp_failures.iter().take(3).collect::<Vec<_>>()
                );
            }
            if !stats.heuristic_failures.is_empty() {
                warn!(
                    "{} heuristic call-resolution failures; sample: {:?}",
                    stats.heuristic_failures.len(),
                    &stats.heuristic_failures.iter().take(3).collect::<Vec<_>>()
                );
            }

            println!(
                "Call resolution: LSP {} | Heuristic {} | Failures (LSP {} / Heuristic {})",
                stats.lsp_edges,
                stats.heuristic_edges,
                stats.lsp_failures.len(),
                stats.heuristic_failures.len()
            );

            // Output fallback telemetry
            if stats.total_fallbacks() > 0 {
                println!(
                    "Fallback telemetry: Calls {} | Imports {} | Types {}",
                    stats.heuristic_call_fallbacks,
                    stats.heuristic_import_fallbacks,
                    stats.heuristic_type_fallbacks
                );
            }

            // Export if requested
            if output == "json" {
                emit(95, "export", "start", "Exporting graph JSON")?;
                let json_path = root.join(format!("{}.json", lang));
                let json_content = serde_json::to_string_pretty(graph)?;
                std::fs::write(&json_path, json_content)?;
                info!("Exported graph to {}", json_path.display());
                emit(
                    98,
                    "export",
                    "done",
                    &format!("Exported to {}", json_path.display()),
                )?;
            }

            // --- Sprint D.4: optional end-of-scan LSP telemetry JSON ---
            //
            // Written *after* logging/progress so a write failure here
            // does not mask a successful parse. We still surface the
            // error, because the caller explicitly asked for it and a
            // silent swallow would hide monitoring gaps.
            if let Some(telemetry_path) = lsp_telemetry.as_ref() {
                use graphengine_parsing::infrastructure::lsp::telemetry_export::LspTelemetryReport;
                let mut report = LspTelemetryReport::build(
                    stats,
                    lang.clone(),
                    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
                );
                if let Some(session_metrics) = resolved_graph.session_metrics() {
                    report = report.with_session_metrics(session_metrics);
                } else {
                    // An explicit no-op branch so readers can see that
                    // the `None` case was considered, not forgotten.
                    // Happens for non-LSP resolvers (mock / heuristic-only
                    // dispatchers when apex-jorje is absent).
                    info!(
                        "LSP telemetry: resolver reported no session metrics \
                         (non-LSP backend or heuristic-only fallback)"
                    );
                }
                match report.to_json() {
                    Ok(json) => {
                        if let Some(parent) = telemetry_path.parent() {
                            if !parent.as_os_str().is_empty() {
                                if let Err(e) = std::fs::create_dir_all(parent) {
                                    warn!(
                                        "Failed to create parent directories for \
                                         LSP telemetry at {}: {}",
                                        telemetry_path.display(),
                                        e
                                    );
                                }
                            }
                        }
                        match std::fs::write(telemetry_path, json) {
                            Ok(()) => {
                                info!("Wrote LSP telemetry to {}", telemetry_path.display());
                            }
                            Err(e) => {
                                error!(
                                    "Failed to write LSP telemetry to {}: {}",
                                    telemetry_path.display(),
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to serialize LSP telemetry: {}", e);
                    }
                }
            }

            emit(100, "parse", "done", "Done")?;
            Ok(())
        }
        Err(e) => {
            let _ = emit(100, "parse", "error", &format!("{e}"));
            error!("Parse failed: {}", e);
            Err(e.into())
        }
    }
}

async fn doctor_command(language: Option<String>) -> anyhow::Result<()> {
    let languages = if let Some(lang) = language {
        vec![lang]
    } else {
        get_available_languages()?
    };

    if languages.is_empty() {
        println!("No language configurations found in configs/ directory.");
        return Ok(());
    }

    println!("LSP Doctor\n===========");
    let mut all_ok = true;

    for lang in languages {
        match load_config(&lang) {
            Ok(config) => match resolve_lsp_command(&config) {
                Ok(resolved) => {
                    println!(
                        "✅ {}: {} (source: {:?})",
                        lang,
                        resolved.executable.display(),
                        resolved.source
                    );
                }
                Err(err) => {
                    all_ok = false;
                    println!("❌ {lang}: {err}");
                }
            },
            Err(err) => {
                all_ok = false;
                println!("❌ {lang}: failed to load config ({err})");
            }
        }
    }

    if all_ok {
        println!("\nAll configured LSP servers are available.");
    } else {
        println!(
            "\nSome LSP servers are missing. Install them or set GRAPHENGINE_LSP_<LANG> overrides."
        );
    }

    Ok(())
}

async fn query_command(db: PathBuf, query_type: String, params: Vec<String>) -> anyhow::Result<()> {
    info!("Querying database: {}", db.display());

    let repo = SqliteRepository::new(&db.to_string_lossy())?;

    match query_type.as_str() {
        "calls-from" => {
            if params.is_empty() {
                error!("calls-from query requires a node ID parameter");
                return Ok(());
            }
            let node_id = &params[0];
            if let Some(graph) = repo.get(node_id).await? {
                let call_edges: Vec<_> = graph
                    .edges
                    .iter()
                    .filter(|e| {
                        matches!(e.kind, graphengine_parsing::domain::EdgeKind::Call)
                            && e.from_id == *node_id
                    })
                    .collect();
                println!("Found {} call edges from {}", call_edges.len(), node_id);
                for edge in call_edges {
                    println!(
                        "  -> {} (confidence: {:?})",
                        edge.to_id, edge.provenance.confidence
                    );
                }
            } else {
                println!("Node {} not found", node_id);
            }
        }
        "list-nodes" => {
            let node_ids = repo.list().await?;
            println!("Found {} nodes:", node_ids.len());
            for node_id in node_ids {
                println!("  {}", node_id);
            }
        }
        _ => {
            error!("Unknown query type: {}", query_type);
            println!("Available query types: calls-from, list-nodes");
        }
    }

    Ok(())
}

/// Machine-readable descriptor of one language config, derived 1:1 from the
/// YAML files in `configs/`. Desktop's onboarding detector parses this JSON to
/// stay in lock-step with whatever languages the engine actually supports
/// (including Apex, which is a YAML config rather than an LSP-family default).
#[derive(serde::Serialize)]
struct LanguageDescriptor {
    language: String,
    file_extensions: Vec<String>,
    lsp_command: Option<String>,
    /// Mirrors `infrastructure::config::LanguageDescriptor::discovery_only`.
    /// Included in the JSON so the desktop shell + UI can distinguish
    /// languages that are recognized for discovery but not parseable on
    /// their own (e.g. Visualforce, which the Apex pipeline absorbs via
    /// `vf_page_reader`).
    discovery_only: bool,
}

fn collect_language_descriptors() -> anyhow::Result<Vec<LanguageDescriptor>> {
    // Deliberately use the minimal descriptor loader rather than `load_config`:
    // this command only reports "what does the engine know about?", and that
    // must include discovery-only configs (e.g. Visualforce, which is
    // XML-read, not tree-sitter-parsed). Running the full tree-sitter
    // validation here also blew up the subcommand with stderr noise that had
    // nothing to do with listing. Real parse/analyze paths still use
    // `load_config` where the full validation is warranted.
    let mut out = Vec::new();
    for lang in get_available_languages()? {
        match load_language_descriptor(&lang) {
            Ok(d) => out.push(LanguageDescriptor {
                language: d.language,
                file_extensions: d.file_extensions,
                lsp_command: d.lsp_command,
                discovery_only: d.discovery_only,
            }),
            Err(e) => {
                // Don't fail the whole listing because one YAML is
                // unreadable: surface it as a warning so operators still see
                // the broken file, but keep the supported list populated so
                // the UI doesn't fall back to "no supported languages."
                warn!("skipping config '{}' in languages listing: {}", lang, e);
            }
        }
    }
    out.sort_by(|a, b| a.language.cmp(&b.language));
    Ok(out)
}

async fn languages_command(json: bool) -> anyhow::Result<()> {
    let descriptors = collect_language_descriptors()?;

    if json {
        // Pretty-print so a human can `| jq` this without an extra flag, and
        // so the desktop's captured stdout stays readable in error logs.
        let payload = serde_json::to_string_pretty(&descriptors)?;
        println!("{payload}");
        return Ok(());
    }

    println!("Supported programming languages (from configs/*.yaml):");
    if descriptors.is_empty() {
        println!("  (none — configs directory empty or unresolved)");
    }
    for d in &descriptors {
        let lsp = d.lsp_command.as_deref().unwrap_or("(no LSP configured)");
        let exts = d.file_extensions.join(", ");
        println!("  {:<10} - extensions: {}  lsp: {}", d.language, exts, lsp);
    }
    println!();
    println!("Note: Language server availability depends on system installation.");
    println!("Run `graphengine-parsing doctor` to check LSP availability.");
    Ok(())
}

async fn stats_command(db: PathBuf) -> anyhow::Result<()> {
    info!("Getting database statistics: {}", db.display());

    let repo = SqliteRepository::new(&db.to_string_lossy())?;
    let node_ids = repo.list().await?;

    println!("Database Statistics:");
    println!("  Total nodes: {}", node_ids.len());
    println!("  Database path: {}", db.display());

    Ok(())
}
