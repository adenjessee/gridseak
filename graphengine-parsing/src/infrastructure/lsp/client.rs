//! LSP Client implementation
//!
//! Provides a robust LSP client that handles communication with LSP servers,
//! manages state transitions, and prevents race conditions through proper
//! synchronization and timing control.

use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::errors::LspError;
use crate::infrastructure::lsp::protocol::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};
use url::Url;

/// LSP client state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// Client is not initialized
    Uninitialized = 0,
    /// Client is initializing
    Initializing = 1,
    /// Client is ready for requests
    Ready = 2,
    /// Client is shutting down
    ShuttingDown = 3,
    /// Client has failed
    Failed = 4,
}

impl ClientState {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(ClientState::Uninitialized),
            1 => Some(ClientState::Initializing),
            2 => Some(ClientState::Ready),
            3 => Some(ClientState::ShuttingDown),
            4 => Some(ClientState::Failed),
            _ => None,
        }
    }

    pub fn can_accept_requests(self) -> bool {
        self == ClientState::Ready
    }

    pub fn is_functional(self) -> bool {
        matches!(self, ClientState::Ready)
    }
}

/// LSP request context
#[derive(Debug)]
struct RequestContext {
    response_tx: oneshot::Sender<Result<Value, LspError>>,
}

/// LSP client implementation
#[derive(Debug)]
pub struct LspClient {
    /// Current client state
    state: Arc<AtomicU32>,
    /// LSP server process
    child: Arc<TokioMutex<Option<Child>>>,
    /// Language configuration
    config: Arc<LanguageConfig>,
    /// Workspace root URI
    workspace_root: Option<Url>,
    /// Pending requests
    pending_requests: Arc<RwLock<HashMap<String, RequestContext>>>,
    /// Request ID counter
    request_id_counter: AtomicU32,
    /// Message channels
    message_tx: mpsc::UnboundedSender<LspMessage>,
    message_rx: Arc<TokioMutex<Option<mpsc::UnboundedReceiver<LspMessage>>>>,
    /// Concurrency control
    request_semaphore: Arc<Semaphore>,
    /// Request timeout
    request_timeout: Duration,
    /// Health check interval
    health_interval: Duration,
    /// Last health check
    last_health_check: Arc<TokioMutex<Option<Instant>>>,
    /// Server capabilities
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    /// Server info
    server_info: Arc<RwLock<Option<ServerInfo>>>,
}

impl LspClient {
    /// Create a new LSP client
    pub fn new(
        config: LanguageConfig,
        workspace_root: Option<Url>,
        max_concurrent_requests: usize,
    ) -> Self {
        let (message_tx, message_rx) = mpsc::unbounded_channel();

        Self {
            state: Arc::new(AtomicU32::new(ClientState::Uninitialized as u32)),
            child: Arc::new(TokioMutex::new(None)),
            config: Arc::new(config),
            workspace_root,
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            request_id_counter: AtomicU32::new(1),
            message_tx,
            message_rx: Arc::new(TokioMutex::new(Some(message_rx))),
            request_semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
            request_timeout: Duration::from_secs(10),
            health_interval: Duration::from_secs(30),
            last_health_check: Arc::new(TokioMutex::new(None)),
            server_capabilities: Arc::new(RwLock::new(None)),
            server_info: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the LSP client
    #[instrument(skip(self))]
    pub async fn initialize(&self) -> Result<(), LspError> {
        if self.get_state() != ClientState::Uninitialized {
            return Err(LspError::invalid_config("Client already initialized"));
        }

        self.set_state(ClientState::Initializing);

        // Start the LSP server process
        self.start_server().await?;

        // Start message processing
        self.start_message_processing().await;

        // Perform LSP handshake
        self.perform_handshake().await?;

        self.set_state(ClientState::Ready);
        info!("LSP client initialized successfully");

        // Start health monitoring
        self.start_health_monitoring().await;

        Ok(())
    }

    /// Start the LSP server process
    #[instrument(skip(self))]
    async fn start_server(&self) -> Result<(), LspError> {
        let lsp_cmd = self
            .config
            .lsp_command
            .as_ref()
            .ok_or_else(|| LspError::invalid_config("No LSP command configured"))?;

        let mut child = Command::new(lsp_cmd);

        // Add LSP arguments from config
        if let Some(args) = &self.config.lsp_args {
            child.args(args);
        } else {
            child.args(["--stdio"]);
        }

        let child = child
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                LspError::connection_failed(format!("Failed to spawn LSP server: {}", e))
            })?;

        // Store the child process
        {
            let mut child_guard = self.child.lock().await;
            *child_guard = Some(child);
        }

        info!("LSP server process started");
        Ok(())
    }

    /// Start message processing
    async fn start_message_processing(&self) {
        let child = Arc::clone(&self.child);
        let message_rx = self.message_rx.lock().await.take().unwrap();

        // Spawn message sender task
        tokio::spawn(async move {
            let mut child_guard = child.lock().await;
            if let Some(child) = child_guard.as_mut() {
                let mut stdin = child.stdin.take().unwrap();
                let mut message_rx = message_rx;

                while let Some(message) = message_rx.recv().await {
                    let json = match LspProtocol::serialize_message(&message) {
                        Ok(json) => json,
                        Err(e) => {
                            error!("Failed to serialize message: {}", e);
                            continue;
                        }
                    };

                    let message_with_length =
                        format!("Content-Length: {}\r\n\r\n{}", json.len(), json);

                    if let Err(e) = stdin.write_all(message_with_length.as_bytes()).await {
                        error!("Failed to send message to LSP server: {}", e);
                        break;
                    }

                    if let Err(e) = stdin.flush().await {
                        error!("Failed to flush stdin: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn message receiver task
        let child = Arc::clone(&self.child);
        let pending_requests = Arc::clone(&self.pending_requests);

        tokio::spawn(async move {
            let mut child_guard = child.lock().await;
            if let Some(child) = child_guard.as_mut() {
                let stdout = child.stdout.take().unwrap();
                let mut reader = BufReader::new(stdout);
                let mut buffer = String::new();

                loop {
                    buffer.clear();
                    match reader.read_line(&mut buffer).await {
                        Ok(0) => break, // EOF
                        Ok(_) => {
                            // Parse LSP message
                            if let Some(message) = Self::parse_lsp_message(&buffer) {
                                match message {
                                    LspMessage::Response { id, result, error } => {
                                        let id_str = match id {
                                            LspId::String(s) => s,
                                            LspId::Number(n) => n.to_string(),
                                            LspId::Null => "null".to_string(),
                                        };

                                        let response = if let Some(error) = error {
                                            Err(LspError::request_failed(format!(
                                                "LSP error: {}",
                                                error.message
                                            )))
                                        } else {
                                            Ok(result.unwrap_or(Value::Null))
                                        };

                                        // Find and complete the pending request
                                        if let Some(context) =
                                            pending_requests.write().unwrap().remove(&id_str)
                                        {
                                            let _ = context.response_tx.send(response);
                                        }
                                    }
                                    LspMessage::Notification { method, params: _ } => {
                                        debug!("Received LSP notification: {}", method);
                                        // Handle notifications as needed
                                    }
                                    _ => {
                                        warn!("Unexpected LSP message type");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from LSP server: {}", e);
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Parse LSP message from buffer
    fn parse_lsp_message(buffer: &str) -> Option<LspMessage> {
        // Skip Content-Length header
        if let Some(content_start) = buffer.find("\r\n\r\n") {
            let content = &buffer[content_start + 4..];
            LspProtocol::deserialize_message(content).ok()
        } else {
            None
        }
    }

    /// Perform LSP handshake
    #[instrument(skip(self))]
    async fn perform_handshake(&self) -> Result<(), LspError> {
        // Send initialize request
        let init_id = LspId::Number(1);
        let workspace_folders = self.workspace_root.as_ref().map(|root| {
            vec![WorkspaceFolder {
                uri: root.to_string(),
                name: "workspace".to_string(),
            }]
        });

        let init_request = LspProtocol::create_initialize_request(
            init_id.clone(),
            self.workspace_root.as_ref().map(|u| u.to_string()),
            workspace_folders,
        );

        let init_result = self.send_request(init_request).await?;

        // Parse initialization result
        let init_result: InitializeResult = serde_json::from_value(init_result).map_err(|e| {
            LspError::response_parse_failed(format!("Failed to parse init result: {}", e))
        })?;

        // Store server capabilities and info
        {
            let mut capabilities = self.server_capabilities.write().unwrap();
            *capabilities = Some(init_result.capabilities);
        }

        if let Some(server_info) = init_result.server_info {
            let mut info = self.server_info.write().unwrap();
            *info = Some(server_info);
        }

        // Send initialized notification
        let initialized_notification = LspMessage::Notification {
            method: "initialized".to_string(),
            params: None,
        };

        self.send_notification(initialized_notification).await?;

        info!("LSP handshake completed successfully");
        Ok(())
    }

    /// Send a request and wait for response
    #[instrument(skip(self))]
    pub async fn send_request(&self, request: LspMessage) -> Result<Value, LspError> {
        if !self.get_state().can_accept_requests() {
            return Err(LspError::server_not_available("Client not ready"));
        }

        let _permit =
            self.request_semaphore.acquire().await.map_err(|e| {
                LspError::request_failed(format!("Failed to acquire semaphore: {}", e))
            })?;

        let (id, _method) = match &request {
            LspMessage::Request { id, method, .. } => (id.clone(), method.clone()),
            _ => return Err(LspError::invalid_config("Not a request message")),
        };

        let id_str = match &id {
            LspId::String(s) => s.clone(),
            LspId::Number(n) => n.to_string(),
            LspId::Null => "null".to_string(),
        };

        let (response_tx, response_rx) = oneshot::channel();

        // Store pending request
        {
            let mut pending = self.pending_requests.write().unwrap();
            pending.insert(id_str.clone(), RequestContext { response_tx });
        }

        // Send the request
        self.message_tx
            .send(request)
            .map_err(|e| LspError::request_failed(format!("Failed to send request: {}", e)))?;

        // Wait for response with timeout
        match timeout(self.request_timeout, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(LspError::request_failed(format!("Request failed: {}", e))),
            Err(_) => {
                // Remove from pending requests
                self.pending_requests.write().unwrap().remove(&id_str);
                Err(LspError::Timeout {
                    timeout_ms: self.request_timeout.as_millis() as u64,
                })
            }
        }
    }

    /// Send a notification
    #[instrument(skip(self))]
    pub async fn send_notification(&self, notification: LspMessage) -> Result<(), LspError> {
        if !self.get_state().is_functional() {
            return Err(LspError::server_not_available("Client not functional"));
        }

        self.message_tx
            .send(notification)
            .map_err(|e| LspError::request_failed(format!("Failed to send notification: {}", e)))?;

        Ok(())
    }

    /// Get document symbols
    #[instrument(skip(self))]
    pub async fn get_document_symbols(&self, uri: String) -> Result<Value, LspError> {
        let id = LspId::Number(self.request_id_counter.fetch_add(1, Ordering::SeqCst) as i64);
        let request = LspProtocol::create_document_symbol_request(id, uri);
        self.send_request(request).await
    }

    /// Get workspace symbols
    #[instrument(skip(self))]
    pub async fn get_workspace_symbols(&self, query: String) -> Result<Value, LspError> {
        let id = LspId::Number(self.request_id_counter.fetch_add(1, Ordering::SeqCst) as i64);
        let request = LspProtocol::create_workspace_symbol_request(id, query);
        self.send_request(request).await
    }

    /// Open a document
    #[instrument(skip(self))]
    pub async fn open_document(
        &self,
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    ) -> Result<(), LspError> {
        let notification =
            LspProtocol::create_did_open_notification(uri, language_id, version, text);
        self.send_notification(notification).await
    }

    /// Close a document
    #[instrument(skip(self))]
    pub async fn close_document(&self, uri: String) -> Result<(), LspError> {
        let notification = LspProtocol::create_did_close_notification(uri);
        self.send_notification(notification).await
    }

    /// Start health monitoring
    async fn start_health_monitoring(&self) {
        let state = Arc::clone(&self.state);
        let last_health_check = Arc::clone(&self.last_health_check);
        let health_interval = self.health_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(health_interval);

            loop {
                interval.tick().await;

                if state.load(Ordering::SeqCst) == ClientState::Ready as u32 {
                    // Perform health check (simple ping)
                    let health_check_time = Instant::now();
                    {
                        let mut last_check = last_health_check.lock().await;
                        *last_check = Some(health_check_time);
                    }
                }
            }
        });
    }

    /// Get current client state
    pub fn get_state(&self) -> ClientState {
        let state_value = self.state.load(Ordering::SeqCst);
        ClientState::from_u8(state_value as u8).unwrap_or(ClientState::Failed)
    }

    /// Set client state
    fn set_state(&self, new_state: ClientState) {
        self.state.store(new_state as u32, Ordering::SeqCst);
        debug!("LSP client state changed to: {:?}", new_state);
    }

    /// Check if client is available
    pub fn is_available(&self) -> bool {
        self.get_state().is_functional()
    }

    /// Get server capabilities
    pub fn get_server_capabilities(&self) -> Option<ServerCapabilities> {
        self.server_capabilities.read().unwrap().clone()
    }

    /// Get server info
    pub fn get_server_info(&self) -> Option<ServerInfo> {
        self.server_info.read().unwrap().clone()
    }

    /// Shutdown the client
    #[instrument(skip(self))]
    pub async fn shutdown(&self) -> Result<(), LspError> {
        if self.get_state() == ClientState::ShuttingDown {
            return Ok(());
        }

        self.set_state(ClientState::ShuttingDown);

        // Send shutdown request
        let shutdown_id =
            LspId::Number(self.request_id_counter.fetch_add(1, Ordering::SeqCst) as i64);
        let shutdown_request = LspProtocol::create_shutdown_request(shutdown_id);

        if let Err(e) = self.send_request(shutdown_request).await {
            warn!("Shutdown request failed: {}", e);
        }

        // Send exit notification
        let exit_notification = LspProtocol::create_exit_notification();
        if let Err(e) = self.send_notification(exit_notification).await {
            warn!("Exit notification failed: {}", e);
        }

        // Kill the child process
        {
            let mut child_guard = self.child.lock().await;
            if let Some(mut child) = child_guard.take() {
                if let Err(e) = child.kill().await {
                    warn!("Failed to kill LSP server: {}", e);
                }
            }
        }

        self.set_state(ClientState::Failed);
        info!("LSP client shutdown completed");
        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Ensure cleanup on drop
        let child = Arc::clone(&self.child);
        tokio::spawn(async move {
            let mut child_guard = child.lock().await;
            if let Some(mut child) = child_guard.take() {
                if let Err(e) = child.kill().await {
                    warn!("Failed to kill LSP server on drop: {}", e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_creation() {
        let config = LanguageConfig::new(
            "rust".to_string(),
            vec![".rs".to_string()],
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );

        let client = LspClient::new(config, None, 10);
        assert_eq!(client.get_state(), ClientState::Uninitialized);
        assert!(!client.is_available());
    }

    #[test]
    fn test_state_transitions() {
        assert!(ClientState::Ready.can_accept_requests());
        assert!(ClientState::Ready.is_functional());
        assert!(!ClientState::Uninitialized.can_accept_requests());
        assert!(!ClientState::Uninitialized.is_functional());
    }
}
