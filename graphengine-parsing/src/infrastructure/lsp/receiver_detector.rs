//! Receiver Type Detection for Method Calls
//!
//! This module detects whether a method call is on a trait object/interface
//! or a concrete type by querying LSP for type information at the receiver position.
//!
//! This is a language-agnostic solution that works for all languages by:
//! 1. Using LSP hover to get receiver type information
//! 2. Matching against language-specific patterns (configured in YAML)
//! 3. Gracefully degrading when LSP is unavailable

use crate::application::ports::CallSite;
use crate::domain::Range;
use crate::infrastructure::config::{
    HoverExtractionConfig, LanguageConfig, ReceiverTypeDetectionConfig,
};
use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
use crate::infrastructure::lsp::errors::LspError;
use crate::syntax::language::{load_extractor, LanguageSpecificExtractor};
use std::sync::Arc;
use tracing::{debug, warn};

/// Language-agnostic receiver type detector
///
/// Uses LSP hover to get receiver type information and matches against
/// language-specific patterns configured in LanguageConfig. Falls back to
/// [`LanguageSpecificExtractor::is_trait_object_type`] when no YAML patterns
/// are configured.
pub struct ReceiverTypeDetector {
    definition_provider: Arc<dyn DefinitionProvider>,
    #[allow(dead_code)]
    config: Arc<LanguageConfig>,
    language_extractor: Arc<dyn LanguageSpecificExtractor>,
    type_patterns: Option<ReceiverTypeDetectionConfig>,
}

impl ReceiverTypeDetector {
    /// Create a new receiver type detector
    ///
    /// Automatically loads receiver type detection configuration from LanguageConfig.
    /// If no configuration is provided, the detector will still work but with reduced accuracy.
    pub fn new(
        definition_provider: Arc<dyn DefinitionProvider>,
        config: Arc<LanguageConfig>,
    ) -> Self {
        let type_patterns = config.get_receiver_type_detection().cloned();
        let language_extractor = load_extractor(&config);
        Self {
            definition_provider,
            config,
            language_extractor,
            type_patterns,
        }
    }

    /// Detect if a method call is on a trait object/interface
    ///
    /// Returns true if the receiver is a trait object (e.g., `&dyn Trait`, `Box<dyn Trait>`)
    /// Returns false if the receiver is a concrete type
    /// Returns None if type information cannot be determined
    pub async fn is_trait_object_call(
        &self,
        call_site: &CallSite,
        receiver_range: Option<&Range>,
    ) -> Result<Option<bool>, LspError> {
        // If we don't have receiver range, we can't determine the type
        let receiver_range = match receiver_range {
            Some(range) => range,
            None => {
                debug!("No receiver range provided for call site, cannot detect trait object call");
                return Ok(None);
            }
        };

        // Query LSP for type information at receiver position
        let hover_info = match self.definition_provider.hover(receiver_range).await {
            Ok(Some(info)) => info,
            Ok(None) => {
                debug!("LSP hover returned no information for receiver");
                return Ok(None);
            }
            Err(e) => {
                warn!("LSP hover failed for receiver: {}", e);
                return Ok(None);
            }
        };

        // Extract type information from hover response
        let type_string = self.extract_type_from_hover(&hover_info);

        // Check if type matches trait object patterns
        let is_trait_object = self.check_trait_object_patterns(&type_string);

        debug!(
            "Receiver type for call '{}': '{}' -> trait_object: {}",
            call_site.function_name, type_string, is_trait_object
        );

        Ok(Some(is_trait_object))
    }

    /// Get the receiver type string
    pub async fn get_receiver_type(
        &self,
        receiver_range: &Range,
    ) -> Result<Option<String>, LspError> {
        match self.definition_provider.hover(receiver_range).await {
            Ok(Some(info)) => {
                let type_string = self.extract_type_from_hover(&info);
                Ok(Some(type_string))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                warn!("LSP hover failed: {}", e);
                Err(e)
            }
        }
    }

    /// Extract type information from LSP hover response
    ///
    /// Hover responses can be in various formats:
    /// - Plain string: "&dyn LayoutAlgorithm"
    /// - Markdown code block: "```rust\n&dyn LayoutAlgorithm\n```"
    /// - Multiple parts: Array of strings
    ///
    /// Uses configuration to determine extraction strategy.
    fn extract_type_from_hover(&self, hover_info: &str) -> String {
        let default_extraction = HoverExtractionConfig::default();
        let extraction_config = self
            .type_patterns
            .as_ref()
            .map(|p| &p.hover_extraction)
            .unwrap_or(&default_extraction);

        // Try to extract type from markdown code blocks (if enabled)
        if extraction_config.markdown_code_block {
            if let Some(start) = hover_info.find("```") {
                if let Some(end) = hover_info[start + 3..].find("```") {
                    let code_block = &hover_info[start + 3..start + 3 + end];
                    // Skip language identifier (first line)
                    if let Some(newline) = code_block.find('\n') {
                        return code_block[newline + 1..].trim().to_string();
                    }
                    return code_block.trim().to_string();
                }
            }
        }

        // Try to extract from function signature patterns (if enabled)
        if extraction_config.function_signature
            && (hover_info.contains("&dyn")
                || hover_info.contains("Box<dyn")
                || hover_info.contains("Arc<dyn"))
        {
            // Extract the type part
            for line in hover_info.lines() {
                if line.contains("&dyn") || line.contains("Box<dyn") || line.contains("Arc<dyn") {
                    // Extract type from line
                    if let Some(start) = line.find("&dyn") {
                        let rest = &line[start..];
                        if let Some(end) = rest.find([' ', '\n', '`']) {
                            return rest[..end].trim().to_string();
                        }
                        return rest.trim().to_string();
                    }
                    if let Some(start) = line.find("Box<dyn") {
                        // Extract until matching >
                        let mut depth = 0;
                        let mut end = start;
                        for (i, c) in line[start..].char_indices() {
                            match c {
                                '<' => depth += 1,
                                '>' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        end = start + i + 1;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        return line[start..end].trim().to_string();
                    }
                    if let Some(start) = line.find("Arc<dyn") {
                        // Similar to Box<dyn
                        let mut depth = 0;
                        let mut end = start;
                        for (i, c) in line[start..].char_indices() {
                            match c {
                                '<' => depth += 1,
                                '>' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        end = start + i + 1;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        return line[start..end].trim().to_string();
                    }
                }
            }
        }

        // Fallback: return first line (if enabled)
        if extraction_config.first_line_fallback {
            hover_info.lines().next().unwrap_or("").trim().to_string()
        } else {
            String::new()
        }
    }

    /// Check if type string matches trait object patterns for the language
    ///
    /// Uses language-specific patterns from configuration. If no patterns are configured,
    /// delegates to [`LanguageSpecificExtractor::is_trait_object_type`].
    fn check_trait_object_patterns(&self, type_string: &str) -> bool {
        // First, try configured patterns (most accurate)
        if let Some(ref patterns) = self.type_patterns {
            if !patterns.trait_object_patterns.is_empty() {
                return patterns
                    .trait_object_patterns
                    .iter()
                    .any(|pattern| type_string.contains(pattern));
            }
        }

        // Fallback to language-specific defaults via the per-language extractor.
        self.language_extractor.is_trait_object_type(type_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::LanguageConfig;

    #[test]
    fn test_extract_type_from_hover_rust() {
        use std::collections::HashMap;
        let _config = Arc::new(LanguageConfig {
            language: "rust".to_string(),
            file_extensions: vec![".rs".to_string()],
            lsp_command: Some("rust-analyzer".to_string()),
            version: "1.0".to_string(),
            queries: HashMap::new(),
            kind_mappings: HashMap::new(),
            grammar_path: None,
            lsp_args: None,
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        });

        // Mock detector (we can't easily test without LSP, but we can test extraction)
        let _hover_markdown = "```rust\n&dyn LayoutAlgorithm\n```";
        // This would be called internally, but for unit test we'd need to refactor
        // to make extract_type_from_hover public or test via integration tests
    }

    #[test]
    fn test_check_trait_object_patterns_rust() {
        use std::collections::HashMap;
        let _config = Arc::new(LanguageConfig {
            language: "rust".to_string(),
            file_extensions: vec![".rs".to_string()],
            lsp_command: Some("rust-analyzer".to_string()),
            version: "1.0".to_string(),
            queries: HashMap::new(),
            kind_mappings: HashMap::new(),
            grammar_path: None,
            lsp_args: None,
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        });

        // We need a mock DefinitionProvider to test this properly
        // For now, this is tested via integration tests
    }
}
