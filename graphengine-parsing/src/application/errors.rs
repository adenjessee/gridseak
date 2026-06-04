//! Application layer error types
//!
//! Extends domain errors with orchestration-specific failures.
//! Provides context-aware error handling for the parsing pipeline.

use crate::domain::{Range, ValidationError};
use thiserror::Error;

/// Errors that can occur during the parsing orchestration process
#[derive(Error, Debug)]
pub enum ParsingError {
    /// Configuration is invalid or missing
    #[error("Config error: {0}")]
    Config(String),

    /// File discovery failed
    #[error("Discovery failed: {0}")]
    Discovery(#[from] std::io::Error),

    /// Syntax extraction failed
    #[error("Extraction failed: {0}")]
    Extraction(String),

    /// Semantic resolution failed
    #[error("Resolution failed: {0}")]
    Resolution(String),

    /// Graph validation failed (from domain layer)
    #[error("Validation failed: {0}")]
    Validation(#[from] ValidationError),

    /// Repository persistence failed
    #[error("Repository error: {0}")]
    Repository(String),

    /// Unresolved call or reference
    #[error("Unresolved call at {0:?}")]
    UnresolvedCall(Range),

    /// Timeout during processing
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Insufficient confidence in results
    #[error("Low confidence: {0} elements below threshold")]
    LowConfidence(usize),
}

impl ParsingError {
    /// Create a config error with context
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Create an extraction error with context
    pub fn extraction(msg: impl Into<String>) -> Self {
        Self::Extraction(msg.into())
    }

    /// Create a resolution error with context
    pub fn resolution(msg: impl Into<String>) -> Self {
        Self::Resolution(msg.into())
    }

    /// Create a repository error with context
    pub fn repository(msg: impl Into<String>) -> Self {
        Self::Repository(msg.into())
    }

    /// Create a timeout error with context
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::Timeout(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Range;

    #[test]
    fn test_parsing_error_display() {
        let config_error = ParsingError::config("Missing language config");
        assert_eq!(
            config_error.to_string(),
            "Config error: Missing language config"
        );

        let extraction_error = ParsingError::extraction("Tree-sitter failed");
        assert_eq!(
            extraction_error.to_string(),
            "Extraction failed: Tree-sitter failed"
        );

        let resolution_error = ParsingError::resolution("LSP server unavailable");
        assert_eq!(
            resolution_error.to_string(),
            "Resolution failed: LSP server unavailable"
        );

        let repository_error = ParsingError::repository("Database connection failed");
        assert_eq!(
            repository_error.to_string(),
            "Repository error: Database connection failed"
        );

        let timeout_error = ParsingError::timeout("Operation took too long");
        assert_eq!(
            timeout_error.to_string(),
            "Timeout: Operation took too long"
        );

        let unresolved_error =
            ParsingError::UnresolvedCall(Range::with_file(10, 5, 15, 20, "test.rs".to_string()));
        assert!(unresolved_error.to_string().contains("Unresolved call at"));

        let low_conf_error = ParsingError::LowConfidence(5);
        assert_eq!(
            low_conf_error.to_string(),
            "Low confidence: 5 elements below threshold"
        );
    }

    #[test]
    fn test_parsing_error_debug() {
        let error = ParsingError::config("test");
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("Config"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_parsing_error_equality() {
        let error1 = ParsingError::config("test");
        let error2 = ParsingError::config("test");
        let error3 = ParsingError::extraction("test");

        // Note: ParsingError doesn't implement PartialEq, so we can't test equality
        // This is intentional as errors are typically compared by type and message
        assert_eq!(error1.to_string(), error2.to_string());
        assert_ne!(error1.to_string(), error3.to_string());
    }

    #[test]
    fn test_validation_error_conversion() {
        let validation_error = ValidationError::DanglingEdge {
            from_id: "node_a".to_string(),
            to_id: "node_b".to_string(),
        };

        let parsing_error: ParsingError = validation_error.into();
        assert!(parsing_error.to_string().contains("Validation failed"));
        assert!(parsing_error.to_string().contains("Dangling edge"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let parsing_error: ParsingError = io_error.into();
        assert!(parsing_error.to_string().contains("Discovery failed"));
    }
}
