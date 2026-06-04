//! Simple LSP Client implementation
//!
//! A focused LSP client that handles communication with LSP servers
//! for definition lookup and symbol resolution. Supports concurrent
//! in-flight requests via per-request oneshot response routing.

use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::command_locator::resolve_lsp_command;
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::notification_sink::{LspNotificationSink, NullSink};
use crate::infrastructure::lsp::protocol::{
    InitializeResult, LspId, LspMessage, LspProtocol, ServerCapabilities, WorkspaceFolder,
};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};
use url::Url;

type PendingRequests = Arc<std::sync::Mutex<HashMap<u32, oneshot::Sender<LspMessage>>>>;

/// Simple LSP client for basic operations.
///
/// Supports concurrent in-flight requests: each request gets a dedicated
/// oneshot channel, and the reader task routes responses by request ID.
/// A semaphore limits the number of concurrent requests.
pub struct SimpleLspClient {
    config: Arc<LanguageConfig>,
    child: Option<Child>,
    message_tx: Option<mpsc::UnboundedSender<LspMessage>>,
    pending_requests: PendingRequests,
    request_id: std::sync::atomic::AtomicU32,
    server_capabilities: Arc<std::sync::Mutex<Option<ServerCapabilities>>>,
    request_semaphore: Arc<Semaphore>,
    default_timeout: Duration,
    timeout_count: std::sync::atomic::AtomicU64,
    success_count: std::sync::atomic::AtomicU64,
    /// Latency histogram buckets (atomic counters): <10ms, <50ms, <100ms, <250ms, <500ms, <1s, <2s, <5s
    latency_buckets: [std::sync::atomic::AtomicU64; 8],
    max_latency_us: std::sync::atomic::AtomicU64,
    /// Set to `true` the instant the reader task observes EOF / read
    /// error from the child stdout — i.e. the server has crashed, was
    /// killed, or closed the stream. Shared with the reader task via
    /// `Arc` so callers and the supervisor can distinguish "LSP
    /// unresponsive" (timeout, possibly transient) from "LSP dead"
    /// (must restart or give up). See `is_alive()` and D.1 in the
    /// Apex plan.
    dead: Arc<AtomicBool>,
    /// Observability hook fired for every notification frame (from
    /// the reader task) and every stderr line (from the stderr
    /// pump). Defaults to `NullSink` so legacy callers that
    /// construct a `SimpleLspClient` directly keep working. The
    /// `SessionSupervisor` swaps this for its own
    /// `NotificationState` before calling `start_server`, which is
    /// how F.1's counters get populated.
    notification_sink: Arc<dyn LspNotificationSink>,
    /// Workspace folders to advertise at `initialize` time. Typically
    /// populated from SFDX `packageDirectories` for Apex, and left
    /// empty for single-root languages where `rootUri` already
    /// carries the right signal.
    workspace_folders: Vec<WorkspaceFolder>,
}

impl std::fmt::Debug for SimpleLspClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-rolled because `Arc<dyn LspNotificationSink>` has no
        // `Debug` impl (trait-object dispatch would require adding
        // `Debug` as a supertrait of the sink, which leaks an
        // implementation detail into every implementor).
        f.debug_struct("SimpleLspClient")
            .field("language", &self.config.language)
            .field("alive", &self.is_alive())
            .field("default_timeout", &self.default_timeout)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct DefinitionLocation {
    pub uri: String,
    pub range: Option<TextRange>,
}

#[derive(Debug, Clone)]
pub struct HoverInfo {
    pub contents: String,
    pub range: Option<TextRange>,
}

#[derive(Debug, Clone)]
pub struct TextRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

impl SimpleLspClient {
    /// Create a new simple LSP client
    pub fn new(config: LanguageConfig) -> Self {
        let max_concurrent = config.lsp_max_concurrent_requests.unwrap_or(32) as usize;
        let timeout_ms = config.lsp_request_timeout_ms.unwrap_or(5000) as u64;
        Self {
            config: Arc::new(config),
            child: None,
            message_tx: None,
            pending_requests: Arc::new(std::sync::Mutex::new(HashMap::new())),
            request_id: std::sync::atomic::AtomicU32::new(1),
            server_capabilities: Arc::new(std::sync::Mutex::new(None)),
            request_semaphore: Arc::new(Semaphore::new(max_concurrent)),
            default_timeout: Duration::from_millis(timeout_ms),
            timeout_count: std::sync::atomic::AtomicU64::new(0),
            success_count: std::sync::atomic::AtomicU64::new(0),
            latency_buckets: Default::default(),
            max_latency_us: std::sync::atomic::AtomicU64::new(0),
            dead: Arc::new(AtomicBool::new(false)),
            notification_sink: Arc::new(NullSink) as Arc<dyn LspNotificationSink>,
            workspace_folders: Vec::new(),
        }
    }

    /// Install an observability sink. Called by `SessionSupervisor`
    /// immediately after `new()` so the first notifications and
    /// stderr lines after spawn are captured. Idempotent — swapping
    /// the sink on a live client is safe because the reader tasks
    /// clone the `Arc` at spawn time; once spawned they keep using
    /// the original sink for the life of that reader.
    pub fn set_notification_sink(&mut self, sink: Arc<dyn LspNotificationSink>) {
        self.notification_sink = sink;
    }

    /// Install workspace folders to advertise at `initialize` time.
    /// Empty slice clears any previously set folders. Must be called
    /// before `start_server` — the initialize payload is built once
    /// at the start of the spawn and never resent.
    ///
    /// F.2: we pass the SFDX `packageDirectories` here so jorje
    /// indexes every declared source root. Prior to this, Apex scans
    /// that live in a multi-package sfdx-project.json silently
    /// indexed only the primary root, which is one of the reasons
    /// dreamhouse-lwc (single package) worked while NPSP / large
    /// corpora saw empty definition results.
    pub fn set_workspace_folders(&mut self, folders: Vec<WorkspaceFolder>) {
        self.workspace_folders = folders;
    }

    /// True as long as the reader task is still observing the child's
    /// stdout. Transitions to `false` permanently the moment EOF or a
    /// read error is seen (the LSP server has crashed, was killed, or
    /// closed the stream). Checked by `SessionSupervisor` before every
    /// request so a dead client can be surfaced as
    /// [`LspError::ServerCrashed`] instead of silently timing out each
    /// in-flight request.
    pub fn is_alive(&self) -> bool {
        !self.dead.load(Ordering::Acquire)
    }

    fn record_latency(&self, elapsed: Duration) {
        let us = elapsed.as_micros() as u64;
        let bucket = match us {
            0..=9_999 => 0,             // <10ms
            10_000..=49_999 => 1,       // <50ms
            50_000..=99_999 => 2,       // <100ms
            100_000..=249_999 => 3,     // <250ms
            250_000..=499_999 => 4,     // <500ms
            500_000..=999_999 => 5,     // <1s
            1_000_000..=1_999_999 => 6, // <2s
            _ => 7,                     // <5s (up to timeout)
        };
        self.latency_buckets[bucket].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.max_latency_us
            .fetch_max(us, std::sync::atomic::Ordering::Relaxed);
    }

    /// Start the LSP server
    pub async fn start_server(&mut self, workspace_root: Option<Url>) -> Result<(), LspError> {
        info!("Starting LSP server for language: {}", self.config.language);

        let server_command = self.get_server_command()?;

        if !self.is_lsp_server_available(&server_command[0]) {
            return Err(LspError::ProtocolError(format!(
                "LSP server '{}' is not available. Please install it or configure a different LSP server.",
                server_command[0]
            )));
        }

        let resolved_root = Self::resolve_workspace_root(workspace_root.clone());
        if let Some(root) = &resolved_root {
            debug!("Using workspace root: {}", root);
        } else {
            debug!("No workspace root provided; relying on server defaults");
        }

        let mut command = Command::new(&server_command[0]);
        command
            .args(&server_command[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(root_url) = resolved_root.as_ref() {
            if let Ok(path) = root_url.to_file_path() {
                command.current_dir(path);
            }
        }

        let mut child = command
            .spawn()
            .map_err(|e| LspError::ProtocolError(format!("Failed to start LSP server '{}': {}. Please ensure the LSP server is installed and in your PATH.", server_command[0], e)))?;

        let (message_tx, message_rx) = mpsc::unbounded_channel::<LspMessage>();

        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            let stderr_sink = Arc::clone(&self.notification_sink);
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    stderr_sink.record_stderr_line(&line);
                }
                debug!("LSP stderr stream closed");
            });
        } else {
            warn!("LSP server stderr not available for logging");
        }

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::ProtocolError("Failed to capture LSP stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::ProtocolError("Failed to capture LSP stdout".to_string()))?;

        let pending = Arc::clone(&self.pending_requests);
        let dead = Arc::clone(&self.dead);
        let sink = Arc::clone(&self.notification_sink);
        tokio::spawn(async move {
            if let Err(e) =
                Self::process_messages(stdin, stdout, message_rx, pending, dead, sink).await
            {
                error!("LSP message processing error: {}", e);
            }
        });

        self.child = Some(child);
        self.message_tx = Some(message_tx);

        self.initialize(resolved_root.clone()).await?;

        info!("LSP server started and initialized successfully");
        Ok(())
    }

    /// Initialize the LSP session (uses longer timeout for server startup).
    ///
    /// When `LanguageConfig.lsp_initialization_options` is `None`, uses the
    /// existing minimal `initialize` payload — preserves byte-identical behavior
    /// for all currently-supported languages (Rust, Java, C#, TS, JS, Python, Go).
    ///
    /// When `Some`, sends a full `InitializeParams` including the provided
    /// `initializationOptions`. Required for LSPs that mandate workspace
    /// configuration at init (e.g. Apex `apex-jorje` needs `enableSemanticErrors`).
    async fn initialize(&self, workspace_root: Option<Url>) -> Result<(), LspError> {
        let request_id = self.get_next_request_id();
        let root_uri = workspace_root.as_ref().map(|u| u.to_string());

        // F.2: if the supervisor populated `workspace_folders`, send
        // them in the initialize payload. If the list is empty and we
        // have a `rootUri`, synthesize a single folder so multi-root
        // LSPs (jorje, pyright, rust-analyzer with linked projects)
        // always see *something* in `workspaceFolders` and don't fall
        // back to cwd on servers that ignore `rootUri` when the array
        // is absent (notable: jorje).
        let folders: Option<Vec<WorkspaceFolder>> = if !self.workspace_folders.is_empty() {
            Some(self.workspace_folders.clone())
        } else if let Some(root) = workspace_root.as_ref() {
            let name = root
                .to_file_path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "workspace".to_string());
            Some(vec![WorkspaceFolder {
                uri: root.to_string(),
                name,
            }])
        } else {
            None
        };

        let init_request = match self.config.lsp_initialization_options.clone() {
            Some(options) => LspProtocol::create_initialize_request_with_options(
                LspId::Number(request_id as i64),
                root_uri,
                folders,
                Some(options),
            ),
            None => {
                // Only elevate to the "with options" payload if we
                // actually have extra folders to advertise — this
                // preserves the minimal-init contract for every
                // pre-existing language that already works.
                if folders.as_ref().is_some_and(|f| f.len() > 1) {
                    LspProtocol::create_initialize_request_with_options(
                        LspId::Number(request_id as i64),
                        root_uri,
                        folders,
                        None,
                    )
                } else {
                    LspProtocol::create_initialize_request_minimal(
                        LspId::Number(request_id as i64),
                        root_uri,
                    )
                }
            }
        };

        let response = self
            .send_request(init_request, request_id, Duration::from_secs(30))
            .await?;

        match response {
            LspMessage::Response { result, error, .. } => {
                if let Some(err) = error {
                    return Err(LspError::ProtocolError(format!(
                        "Initialize failed: {}",
                        err.message
                    )));
                }
                if let Some(result_value) = result {
                    if let Ok(init_result) =
                        serde_json::from_value::<InitializeResult>(result_value.clone())
                    {
                        let mut caps = self.server_capabilities.lock().unwrap();
                        *caps = Some(init_result.capabilities);
                        info!(
                            "LSP server initialized: {}",
                            init_result
                                .server_info
                                .map(|si| format!("{} v{}", si.name, si.version))
                                .unwrap_or_else(|| "unknown".to_string())
                        );
                    }
                } else {
                    return Err(LspError::ProtocolError(
                        "Initialize response missing result".to_string(),
                    ));
                }
            }
            _ => {
                return Err(LspError::ProtocolError(
                    "Unexpected response type".to_string(),
                ))
            }
        }

        let initialized_notification = LspMessage::Notification {
            method: "initialized".to_string(),
            params: None,
        };
        self.send_message(initialized_notification).await?;

        info!("LSP session initialized successfully");
        Ok(())
    }

    /// Send a request and wait for its response, routed by request ID.
    ///
    /// Acquires a semaphore permit to bound concurrency, registers a oneshot
    /// receiver in `pending_requests`, sends the message, then awaits the
    /// response with the given timeout.
    async fn send_request(
        &self,
        message: LspMessage,
        request_id: u32,
        timeout_duration: Duration,
    ) -> Result<LspMessage, LspError> {
        // Fast-fail: if the reader task has already observed the
        // server dying, there is no point registering a pending
        // request — the response will never come. This turns a
        // post-crash burst of requests into a tight stream of clean
        // `ServerCrashed` errors the supervisor can act on, instead
        // of `default_timeout` × N wasted seconds.
        if !self.is_alive() {
            return Err(LspError::server_crashed(
                "LSP server process exited — request rejected before send",
            ));
        }

        let _permit = self
            .request_semaphore
            .acquire()
            .await
            .map_err(|_| LspError::ProtocolError("Request semaphore closed".to_string()))?;

        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending_requests.lock().map_err(|_| {
                LspError::ProtocolError("Pending requests lock poisoned".to_string())
            })?;
            pending.insert(request_id, tx);
        }

        if let Err(e) = self.send_message(message).await {
            self.pending_requests
                .lock()
                .ok()
                .map(|mut p| p.remove(&request_id));
            return Err(e);
        }

        let req_start = std::time::Instant::now();
        match timeout(timeout_duration, rx).await {
            Ok(Ok(response)) => {
                self.record_latency(req_start.elapsed());
                self.success_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(response)
            }
            Ok(Err(_)) => {
                self.pending_requests
                    .lock()
                    .ok()
                    .map(|mut p| p.remove(&request_id));
                // The oneshot sender was dropped. Two cases:
                //   (a) reader task exited because the server died —
                //       `self.dead` is now true. Return `ServerCrashed`
                //       so the supervisor can trigger a restart cycle.
                //   (b) something else dropped the sender unexpectedly —
                //       unlikely in the current design, but we fall
                //       through to the generic protocol error for
                //       honesty.
                if !self.is_alive() {
                    Err(LspError::server_crashed(
                        "LSP server process exited while request was in flight",
                    ))
                } else {
                    Err(LspError::ProtocolError(
                        "Response channel closed".to_string(),
                    ))
                }
            }
            Err(_) => {
                self.pending_requests
                    .lock()
                    .ok()
                    .map(|mut p| p.remove(&request_id));
                let count = self
                    .timeout_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                let successes = self
                    .success_count
                    .load(std::sync::atomic::Ordering::Relaxed);
                warn!(
                    "[LSP_TIMEOUT] request {} timed out after {:?} (total timeouts: {}, successes: {})",
                    request_id, timeout_duration, count, successes
                );
                Err(LspError::ProtocolError("Request timeout".to_string()))
            }
        }
    }

    /// Find definition of a symbol (concurrent-safe, takes &self)
    pub async fn find_definition(
        &self,
        uri: String,
        line: u32,
        character: u32,
    ) -> Result<Option<DefinitionLocation>, LspError> {
        let request_id = self.get_next_request_id();
        let definition_request = LspProtocol::create_definition_request(
            LspId::Number(request_id as i64),
            uri,
            line,
            character,
        );

        let response = self
            .send_request(definition_request, request_id, self.default_timeout)
            .await?;

        match response {
            LspMessage::Response { result, error, .. } => {
                if let Some(err) = error {
                    warn!("Definition request failed: {}", err.message);
                    return Ok(None);
                }

                if let Some(result) = result {
                    if result.is_null() {
                        return Ok(None);
                    }

                    if let Some(locations) = result.as_array() {
                        if let Some(location) = locations.first() {
                            return Self::location_from_value(location);
                        }
                    }

                    if result.is_object() {
                        return Self::location_from_value(&result);
                    }
                }

                Ok(None)
            }
            _ => {
                warn!("Unexpected response type for definition request");
                Ok(None)
            }
        }
    }

    /// F.2: readiness probe. Sends `textDocument/documentSymbol` and
    /// returns `Ok(Some(symbols))` with the raw JSON array if the
    /// server has any symbols for `uri`. `Ok(None)` when the server
    /// returned null / an error / an empty result. We intentionally
    /// don't shape the return type further — the supervisor treats
    /// this as a binary "has the server indexed this file yet?" probe.
    pub async fn document_symbol(&self, uri: String) -> Result<Option<Vec<Value>>, LspError> {
        let request_id = self.get_next_request_id();
        let req =
            LspProtocol::create_document_symbol_request(LspId::Number(request_id as i64), uri);

        let response = self
            .send_request(req, request_id, self.default_timeout)
            .await?;
        match response {
            LspMessage::Response { result, error, .. } => {
                if error.is_some() {
                    return Ok(None);
                }
                match result {
                    Some(Value::Array(arr)) if !arr.is_empty() => Ok(Some(arr)),
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    fn location_from_value(value: &Value) -> Result<Option<DefinitionLocation>, LspError> {
        if let Some(uri) = value.get("uri").and_then(|u| u.as_str()) {
            let range = value.get("range").and_then(Self::parse_text_range);
            Ok(Some(DefinitionLocation {
                uri: uri.to_string(),
                range,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get hover information (concurrent-safe, takes &self)
    pub async fn hover(
        &self,
        uri: String,
        line: u32,
        character: u32,
    ) -> Result<Option<HoverInfo>, LspError> {
        let request_id = self.get_next_request_id();
        let hover_request = LspProtocol::create_hover_request(
            LspId::Number(request_id as i64),
            uri,
            line,
            character,
        );

        let response = self
            .send_request(hover_request, request_id, self.default_timeout)
            .await?;

        match response {
            LspMessage::Response { result, error, .. } => {
                if let Some(err) = error {
                    warn!("Hover request failed: {}", err.message);
                    return Ok(None);
                }

                if let Some(result) = result {
                    if result.is_null() {
                        return Ok(None);
                    }

                    if let Some(contents) = result.get("contents") {
                        let hover_info = Self::parse_hover_contents(contents)?;
                        let range = result.get("range").and_then(Self::parse_text_range);
                        Ok(Some(HoverInfo {
                            contents: hover_info,
                            range,
                        }))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }
            _ => {
                warn!("Unexpected response type for hover request");
                Ok(None)
            }
        }
    }

    fn parse_hover_contents(value: &Value) -> Result<String, LspError> {
        if let Some(s) = value.as_str() {
            return Ok(s.to_string());
        }

        if let Some(obj) = value.as_object() {
            if let Some(value_str) = obj.get("value").and_then(|v| v.as_str()) {
                if let Some(lang) = obj.get("language").and_then(|l| l.as_str()) {
                    return Ok(format!("```{}\n{}\n```", lang, value_str));
                }
                return Ok(value_str.to_string());
            }
        }

        if let Some(arr) = value.as_array() {
            let mut parts = Vec::new();
            for item in arr {
                if let Ok(parsed) = Self::parse_hover_contents(item) {
                    parts.push(parsed);
                }
            }
            return Ok(parts.join("\n\n"));
        }

        if let Some(obj) = value.as_object() {
            if let Some(value_str) = obj.get("value").and_then(|v| v.as_str()) {
                return Ok(value_str.to_string());
            }
        }

        Ok(String::new())
    }

    fn parse_text_range(value: &Value) -> Option<TextRange> {
        let start = value.get("start")?;
        let end = value.get("end")?;

        let start_line = start.get("line")?.as_u64()? as u32;
        let start_character = start.get("character")?.as_u64()? as u32;
        let end_line = end.get("line")?.as_u64()? as u32;
        let end_character = end.get("character")?.as_u64()? as u32;

        Some(TextRange {
            start_line,
            start_character,
            end_line,
            end_character,
        })
    }

    /// Open a document in the LSP server (notification, no response expected)
    pub async fn document_did_open(
        &self,
        uri: String,
        language_id: String,
        text: String,
    ) -> Result<(), LspError> {
        let notification = LspProtocol::create_did_open_notification(uri, language_id, 1, text);
        self.send_message(notification).await
    }

    /// Close a document in the LSP server (notification, no response expected)
    pub async fn document_did_close(&self, uri: String) -> Result<(), LspError> {
        let notification = LspProtocol::create_did_close_notification(uri);
        self.send_message(notification).await
    }

    /// Find references to a symbol (concurrent-safe, takes &self)
    pub async fn find_references(
        &self,
        uri: String,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<DefinitionLocation>, LspError> {
        let request_id = self.get_next_request_id();
        let refs_request = LspProtocol::create_references_request(
            LspId::Number(request_id as i64),
            uri,
            line,
            character,
            include_declaration,
        );

        let response = self
            .send_request(refs_request, request_id, self.default_timeout)
            .await?;

        match response {
            LspMessage::Response { result, error, .. } => {
                if let Some(err) = error {
                    warn!("References request failed: {}", err.message);
                    return Ok(Vec::new());
                }

                let mut locations = Vec::new();
                if let Some(result) = result {
                    if let Some(locs_array) = result.as_array() {
                        for loc_value in locs_array {
                            if let Ok(Some(loc)) = Self::location_from_value(loc_value) {
                                locations.push(loc);
                            }
                        }
                    }
                }

                Ok(locations)
            }
            _ => {
                warn!("Unexpected response type for references request");
                Ok(Vec::new())
            }
        }
    }

    /// Get document symbols (concurrent-safe, takes &self)
    pub async fn document_symbols(&self, uri: String) -> Result<Vec<serde_json::Value>, LspError> {
        let request_id = self.get_next_request_id();
        let symbols_request =
            LspProtocol::create_document_symbol_request(LspId::Number(request_id as i64), uri);

        let response = self
            .send_request(symbols_request, request_id, self.default_timeout)
            .await?;

        match response {
            LspMessage::Response { result, error, .. } => {
                if let Some(err) = error {
                    warn!("Document symbols request failed: {}", err.message);
                    return Ok(Vec::new());
                }

                if let Some(result) = result {
                    if let Some(symbols_array) = result.as_array() {
                        return Ok(symbols_array.clone());
                    }
                }

                Ok(Vec::new())
            }
            _ => {
                warn!("Unexpected response type for document symbols request");
                Ok(Vec::new())
            }
        }
    }

    /// Get server capabilities
    pub fn server_capabilities(&self) -> Option<ServerCapabilities> {
        self.server_capabilities.lock().ok()?.clone()
    }

    /// Send a message to the LSP server (fire-and-forget for notifications)
    async fn send_message(&self, message: LspMessage) -> Result<(), LspError> {
        if let Some(tx) = &self.message_tx {
            tx.send(message)
                .map_err(|_| LspError::ProtocolError("Failed to send message".to_string()))?;
            Ok(())
        } else {
            Err(LspError::ProtocolError(
                "LSP client not initialized".to_string(),
            ))
        }
    }

    fn get_next_request_id(&self) -> u32 {
        self.request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    fn is_lsp_server_available(&self, command: &str) -> bool {
        match std::process::Command::new(command)
            .arg("--version")
            .output()
        {
            Ok(output) => output.status.success(),
            Err(_) => match std::process::Command::new(command).arg("--help").output() {
                Ok(output) => output.status.success(),
                Err(_) => false,
            },
        }
    }

    fn get_server_command(&self) -> Result<Vec<String>, LspError> {
        let resolved = resolve_lsp_command(&self.config)?;
        Ok(resolved.command)
    }

    fn resolve_workspace_root(workspace_root: Option<Url>) -> Option<Url> {
        if let Some(url) = workspace_root {
            return Some(url);
        }

        env::current_dir()
            .ok()
            .and_then(|path| Url::from_directory_path(path).ok())
    }

    /// Process messages between client and server.
    ///
    /// The reader task routes responses to the correct pending oneshot sender
    /// by matching on the response's request ID.
    async fn process_messages(
        stdin: tokio::process::ChildStdin,
        stdout: tokio::process::ChildStdout,
        mut message_rx: mpsc::UnboundedReceiver<LspMessage>,
        pending_requests: PendingRequests,
        dead: Arc<AtomicBool>,
        notification_sink: Arc<dyn LspNotificationSink>,
    ) -> Result<(), LspError> {
        let mut writer = BufWriter::new(stdin);
        let mut reader = BufReader::new(stdout);

        let reader_pending = Arc::clone(&pending_requests);
        let reader_dead = Arc::clone(&dead);
        let reader_sink = Arc::clone(&notification_sink);
        tokio::spawn(async move {
            let cause: &str = loop {
                match Self::read_lsp_message(&mut reader).await {
                    Ok(Some(message)) => {
                        match &message {
                            LspMessage::Response {
                                id: LspId::Number(id_num),
                                ..
                            } => {
                                let sender = {
                                    reader_pending
                                        .lock()
                                        .ok()
                                        .and_then(|mut p| p.remove(&(*id_num as u32)))
                                };
                                if let Some(sender) = sender {
                                    let _ = sender.send(message);
                                } else {
                                    warn!("Received response for unknown request ID {}", id_num);
                                }
                            }
                            LspMessage::Notification { method, params } => {
                                // F.1: route every notification into
                                // the supervisor's sink. Previously
                                // silently consumed — the single
                                // biggest reason jorje's indexing
                                // progress was invisible.
                                reader_sink.record_notification(method, params.as_ref());
                            }
                            LspMessage::Response { .. } => {
                                // Response with a non-numeric id; not
                                // currently produced by our client.
                            }
                            LspMessage::Request { .. } => {
                                // Server-initiated requests are not
                                // yet supported; ignore rather than
                                // deadlock.
                            }
                        }
                    }
                    Ok(None) => {
                        break "EOF from LSP server stdout (server exited)";
                    }
                    Err(err) => {
                        error!("Failed to read LSP message: {}", err);
                        break "read error from LSP server stdout";
                    }
                }
            };
            // Mark the client dead *before* draining pending, so any
            // caller that races in after we drop the oneshot senders
            // can check `is_alive()` and map its `Canceled` error to
            // the real root cause (`ServerCrashed`) rather than the
            // generic channel-closed message.
            reader_dead.store(true, Ordering::Release);
            warn!(
                pending_on_crash = reader_pending.lock().ok().map(|p| p.len()).unwrap_or(0),
                cause, "LSP reader task exiting — failing all pending requests",
            );
            // Drop every pending oneshot sender. The `send_request`
            // path receives `oneshot::Canceled`, sees `!is_alive()`
            // via the parent client, and returns `ServerCrashed`
            // instead of `ProtocolError("channel closed")`.
            if let Ok(mut pending) = reader_pending.lock() {
                pending.clear();
            }
        });

        while let Some(message) = message_rx.recv().await {
            if let Err(err) = Self::write_lsp_message(&mut writer, &message).await {
                error!("Failed to write LSP message: {}", err);
                return Err(err);
            }
        }

        Ok(())
    }

    /// Shutdown the LSP client
    pub async fn shutdown(&mut self) -> Result<(), LspError> {
        let timeouts = self
            .timeout_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let successes = self
            .success_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let max_us = self
            .max_latency_us
            .load(std::sync::atomic::Ordering::Relaxed);

        let labels = [
            "<10ms", "<50ms", "<100ms", "<250ms", "<500ms", "<1s", "<2s", "<5s",
        ];
        let counts: Vec<u64> = self
            .latency_buckets
            .iter()
            .map(|b| b.load(std::sync::atomic::Ordering::Relaxed))
            .collect();
        let histogram: Vec<String> = labels
            .iter()
            .zip(counts.iter())
            .filter(|(_, &c)| c > 0)
            .map(|(l, c)| format!("{}={}", l, c))
            .collect();

        info!(
            "[LSP_STATS] Session summary: {} successes, {} timeouts (timeout_ms={})",
            successes,
            timeouts,
            self.default_timeout.as_millis()
        );
        info!(
            "[LSP_LATENCY] Histogram: [{}] max={:.1}ms",
            histogram.join(", "),
            max_us as f64 / 1000.0
        );

        if self.child.is_some() {
            let request_id = self.get_next_request_id();
            let shutdown_request =
                LspProtocol::create_shutdown_request(LspId::Number(request_id as i64));

            let _ = self
                .send_request(shutdown_request, request_id, Duration::from_secs(5))
                .await;

            let exit_notification = LspProtocol::create_exit_notification();
            let _ = self.send_message(exit_notification).await;

            if let Some(child) = &mut self.child {
                let _ = child.wait().await;
            }
        }

        self.child = None;
        self.message_tx = None;

        info!("LSP client shutdown complete");
        Ok(())
    }
}

impl Drop for SimpleLspClient {
    fn drop(&mut self) {
        if self.child.is_some() {
            warn!("LSP client dropped without proper shutdown");
        }
    }
}

impl SimpleLspClient {
    async fn write_lsp_message<W>(writer: &mut W, message: &LspMessage) -> Result<(), LspError>
    where
        W: AsyncWrite + Unpin,
    {
        let json = LspProtocol::serialize_message(message)
            .map_err(|e| LspError::ProtocolError(format!("Failed to serialize message: {}", e)))?;
        let header = format!("Content-Length: {}\r\n\r\n", json.len());
        writer
            .write_all(header.as_bytes())
            .await
            .map_err(|e| LspError::ProtocolError(format!("Failed to write header: {}", e)))?;
        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| LspError::ProtocolError(format!("Failed to write payload: {}", e)))?;
        writer
            .flush()
            .await
            .map_err(|e| LspError::ProtocolError(format!("Failed to flush: {}", e)))?;
        Ok(())
    }

    async fn read_lsp_message<R>(reader: &mut R) -> Result<Option<LspMessage>, LspError>
    where
        R: AsyncBufRead + Unpin,
    {
        let mut header_line = String::new();
        let mut content_length: Option<usize> = None;

        loop {
            header_line.clear();
            let bytes_read = reader
                .read_line(&mut header_line)
                .await
                .map_err(|e| LspError::ProtocolError(format!("Failed to read header: {}", e)))?;

            if bytes_read == 0 {
                return Ok(None);
            }

            let trimmed = header_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }

            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }

        let len = content_length
            .ok_or_else(|| LspError::ProtocolError("Missing Content-Length header".to_string()))?;

        let mut buffer = vec![0u8; len];
        reader
            .read_exact(&mut buffer)
            .await
            .map_err(|e| LspError::ProtocolError(format!("Failed to read payload: {}", e)))?;

        let json = String::from_utf8(buffer)
            .map_err(|e| LspError::ProtocolError(format!("Invalid UTF-8 payload: {}", e)))?;

        let message = LspProtocol::deserialize_message(&json).map_err(|e| {
            LspError::ProtocolError(format!("Failed to deserialize message: {}", e))
        })?;
        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_simple_client_creation() {
        let config = LanguageConfig {
            language: "rust".to_string(),
            file_extensions: vec![".rs".to_string()],
            queries: std::collections::HashMap::new(),
            kind_mappings: std::collections::HashMap::new(),
            grammar_path: None,
            lsp_command: None,
            lsp_args: None,
            version: "1.0".to_string(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        };

        let client = SimpleLspClient::new(config);
        assert!(client.child.is_none());
        assert!(client.message_tx.is_none());
    }

    #[tokio::test]
    async fn test_get_next_request_id() {
        let config = LanguageConfig {
            language: "rust".to_string(),
            file_extensions: vec![".rs".to_string()],
            queries: std::collections::HashMap::new(),
            kind_mappings: std::collections::HashMap::new(),
            grammar_path: None,
            lsp_command: None,
            lsp_args: None,
            version: "1.0".to_string(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        };

        let client = SimpleLspClient::new(config);
        let id1 = client.get_next_request_id();
        let id2 = client.get_next_request_id();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test]
    async fn test_message_framing_round_trip() {
        let (mut client_side, server_side) = duplex(1024);
        let message = LspMessage::Notification {
            method: "initialized".to_string(),
            params: None,
        };

        SimpleLspClient::write_lsp_message(&mut client_side, &message)
            .await
            .expect("write message");

        let mut reader = BufReader::new(server_side);
        let received = SimpleLspClient::read_lsp_message(&mut reader)
            .await
            .expect("read message");

        match received {
            Some(LspMessage::Notification { method, .. }) => {
                assert_eq!(method, "initialized");
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test]
    async fn fresh_client_is_alive_until_marked_dead() {
        let config = LanguageConfig {
            language: "rust".to_string(),
            file_extensions: vec![".rs".to_string()],
            queries: std::collections::HashMap::new(),
            kind_mappings: std::collections::HashMap::new(),
            grammar_path: None,
            lsp_command: None,
            lsp_args: None,
            version: "1.0".to_string(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        };
        let client = SimpleLspClient::new(config);
        assert!(
            client.is_alive(),
            "newly constructed client must report alive before any reader task runs"
        );

        // Simulate the reader task observing EOF — this is exactly what
        // `process_messages`'s crash path does. After this the
        // `ServerCrashed` branch of `send_request` must be reachable on
        // the next call.
        client.dead.store(true, Ordering::Release);
        assert!(
            !client.is_alive(),
            "client must report dead once the reader marks it crashed"
        );
    }
}
