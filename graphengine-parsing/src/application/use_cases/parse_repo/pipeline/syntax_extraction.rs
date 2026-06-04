//! Syntax extraction from source files

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::{SyntaxExtractor, SyntaxResults};
use tracing::{info, warn};

/// Syntax extraction service
pub struct SyntaxExtraction;

impl SyntaxExtraction {
    /// Extract syntax information from source files
    ///
    /// # Arguments
    /// * `files` - List of source files to extract from
    /// * `syntax_extractor` - Extractor implementation
    ///
    /// # Returns
    /// * `SyntaxResults` - Extracted syntax information
    /// * `ParsingError` - If extraction fails
    pub async fn extract_from_files(
        files: &[std::path::PathBuf],
        syntax_extractor: &dyn SyntaxExtractor,
    ) -> Result<SyntaxResults, ParsingError> {
        info!("Starting syntax extraction for {} files", files.len());
        let result = syntax_extractor.extract(files).await;
        match result {
            Ok(mut syntax_results) => {
                let source_files = files
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect();
                syntax_results.set_source_files(source_files);
                info!(
                    "Syntax extraction succeeded with {} symbols",
                    syntax_results.symbols.len()
                );
                Ok(syntax_results)
            }
            Err(e) => {
                warn!("Syntax extraction failed: {}", e);
                Err(ParsingError::extraction(format!(
                    "Syntax extraction failed: {}",
                    e
                )))
            }
        }
    }
}
