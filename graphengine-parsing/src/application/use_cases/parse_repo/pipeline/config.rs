//! Configuration loading for language-specific settings

use super::super::super::super::errors::ParsingError;
use crate::infrastructure::load_config;
use tracing::{info, warn};

/// Configuration loader
pub struct ConfigLoader;

impl ConfigLoader {
    /// Load configuration for the given language
    ///
    /// # Arguments
    /// * `language` - Programming language identifier
    ///
    /// # Returns
    /// * `()` - Success
    /// * `ParsingError` - If loading fails
    pub fn load_language_config(language: &str) -> Result<(), ParsingError> {
        info!("Loading configuration for language: {}", language);
        // Load YAML configuration for the language
        let result = load_config(language);
        match &result {
            Ok(_) => {
                info!(
                    "Configuration loaded successfully for language: {}",
                    language
                );
            }
            Err(e) => {
                warn!("Failed to load config for {}: {}", language, e);
            }
        }
        result.map_err(|e| {
            ParsingError::config(format!("Failed to load config for {}: {}", language, e))
        })?;

        Ok(())
    }
}
