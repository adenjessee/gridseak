//! Bearer token auth middleware for HTTP transport.
//!
//! Reads `GRAPHENGINE_API_KEY` env var at startup. If set, every request must
//! include `Authorization: Bearer <key>` matching that value, or get a 401.
//! If the env var is unset, all requests are allowed (local dev mode).

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

/// Cached API key read once from the environment.
static API_KEY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn get_api_key() -> &'static Option<String> {
    API_KEY.get_or_init(|| {
        std::env::var("GRAPHENGINE_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
    })
}

pub fn is_auth_enabled() -> bool {
    get_api_key().is_some()
}

/// Axum middleware that validates bearer tokens against `GRAPHENGINE_API_KEY`.
pub async fn require_bearer(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let expected = match get_api_key() {
        Some(key) => key,
        None => return Ok(next.run(req).await),
    };

    let header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match header {
        Some(val) if val.strip_prefix("Bearer ").is_some_and(|t| t == expected) => {
            Ok(next.run(req).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
