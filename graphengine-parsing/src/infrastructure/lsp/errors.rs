//! LSP-specific error types
//!
//! Defines error types for LSP operations including connection failures,
//! timeout errors, and protocol violations.

use thiserror::Error;

/// Errors that can occur during LSP operations
#[derive(Error, Debug)]
pub enum LspError {
    /// LSP server connection failed
    #[error("LSP server connection failed: {0}")]
    ConnectionFailed(String),

    /// LSP server timeout
    #[error("LSP server timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// LSP server crashed or exited unexpectedly
    #[error("LSP server crashed: {0}")]
    ServerCrashed(String),

    /// LSP protocol error
    #[error("LSP protocol error: {0}")]
    ProtocolError(String),

    /// LSP server not available
    #[error("LSP server not available: {0}")]
    ServerNotAvailable(String),

    /// Invalid LSP configuration
    #[error("Invalid LSP configuration: {0}")]
    InvalidConfig(String),

    /// LSP request failed
    #[error("LSP request failed: {0}")]
    RequestFailed(String),

    /// LSP response parsing failed
    #[error("LSP response parsing failed: {0}")]
    ResponseParseFailed(String),

    /// LSP server initialization failed
    #[error("LSP server initialization failed: {0}")]
    InitializationFailed(String),

    /// LSP server health check failed
    #[error("LSP server health check failed: {0}")]
    HealthCheckFailed(String),
}

impl LspError {
    /// Create a connection failed error
    pub fn connection_failed(msg: impl Into<String>) -> Self {
        Self::ConnectionFailed(msg.into())
    }

    /// Create a timeout error
    pub fn timeout(timeout_ms: u64) -> Self {
        Self::Timeout { timeout_ms }
    }

    /// Create a server crashed error
    pub fn server_crashed(msg: impl Into<String>) -> Self {
        Self::ServerCrashed(msg.into())
    }

    /// Create a protocol error
    pub fn protocol_error(msg: impl Into<String>) -> Self {
        Self::ProtocolError(msg.into())
    }

    /// Create a server not available error
    pub fn server_not_available(msg: impl Into<String>) -> Self {
        Self::ServerNotAvailable(msg.into())
    }

    /// Create an invalid config error
    pub fn invalid_config(msg: impl Into<String>) -> Self {
        Self::InvalidConfig(msg.into())
    }

    /// Create a request failed error
    pub fn request_failed(msg: impl Into<String>) -> Self {
        Self::RequestFailed(msg.into())
    }

    /// Create a response parse failed error
    pub fn response_parse_failed(msg: impl Into<String>) -> Self {
        Self::ResponseParseFailed(msg.into())
    }

    /// Create an initialization failed error
    pub fn initialization_failed(msg: impl Into<String>) -> Self {
        Self::InitializationFailed(msg.into())
    }

    /// Create a health check failed error
    pub fn health_check_failed(msg: impl Into<String>) -> Self {
        Self::HealthCheckFailed(msg.into())
    }

    /// Check if this error is recoverable (can be retried)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            LspError::Timeout { .. }
                | LspError::RequestFailed(_)
                | LspError::ResponseParseFailed(_)
                | LspError::HealthCheckFailed(_)
        )
    }

    /// Check if this error indicates the LSP server is not available
    pub fn is_server_unavailable(&self) -> bool {
        matches!(
            self,
            LspError::ConnectionFailed(_)
                | LspError::ServerCrashed(_)
                | LspError::ServerNotAvailable(_)
                | LspError::InitializationFailed(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_error_creation() {
        let conn_error = LspError::connection_failed("Failed to connect");
        assert!(matches!(conn_error, LspError::ConnectionFailed(_)));

        let timeout_error = LspError::timeout(5000);
        assert!(matches!(
            timeout_error,
            LspError::Timeout { timeout_ms: 5000 }
        ));

        let crash_error = LspError::server_crashed("Process exited");
        assert!(matches!(crash_error, LspError::ServerCrashed(_)));
    }

    #[test]
    fn test_lsp_error_display() {
        let error = LspError::connection_failed("Test error");
        let error_str = format!("{}", error);
        assert!(error_str.contains("LSP server connection failed"));
        assert!(error_str.contains("Test error"));
    }
}
