mod auth;
mod cache;
mod server;

use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "ge-mcp",
    about = "GraphEngine MCP server — exposes parse, validate, analyze, and query tools via stdio or HTTP"
)]
struct Cli {
    /// Directory containing language config YAML files (e.g. graphengine-parsing/configs/).
    /// Required for parse_repo to locate tree-sitter queries and LSP settings.
    #[arg(long)]
    configs_dir: Option<PathBuf>,

    /// Enable verbose logging to stderr (does not affect MCP stdio protocol).
    #[arg(short, long)]
    verbose: bool,

    /// Run as an HTTP server instead of stdio. Exposes MCP over streamable HTTP
    /// at /mcp and a health check at /health.
    #[arg(long)]
    http: bool,

    /// Port to listen on in HTTP mode (default: 8080).
    #[arg(long, default_value = "8080")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        tracing_subscriber::EnvFilter::new("debug")
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    if let Some(dir) = cli.configs_dir {
        graphengine_parsing::infrastructure::config::set_configs_dir_override(dir)?;
    }

    if cli.http {
        serve_http(cli.port).await
    } else {
        serve_stdio().await
    }
}

async fn serve_stdio() -> anyhow::Result<()> {
    let server = server::GraphEngineServer::new();
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

async fn serve_http(port: u16) -> anyhow::Result<()> {
    use axum::{middleware, routing::get, Router};
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    };
    use tower_http::cors::CorsLayer;

    let mcp_service = StreamableHttpService::new(
        || Ok(server::GraphEngineServer::new()),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let mcp_router = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn(auth::require_bearer));

    let app = Router::new()
        .route("/health", get(health_handler))
        .merge(mcp_router)
        .layer(CorsLayer::permissive());

    let bind_addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    if auth::is_auth_enabled() {
        tracing::info!("Auth enabled (GRAPHENGINE_API_KEY is set)");
    } else {
        tracing::warn!(
            "Auth DISABLED — GRAPHENGINE_API_KEY is not set. All requests will be allowed."
        );
    }

    tracing::info!("ge-mcp HTTP server listening on {bind_addr}");
    tracing::info!("  MCP endpoint: http://{bind_addr}/mcp");
    tracing::info!("  Health check: http://{bind_addr}/health");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutting down...");
        })
        .await?;

    Ok(())
}

async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "ge-mcp",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
