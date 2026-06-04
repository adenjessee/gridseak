//! Document synchronization utilities for LSP
//!
//! Handles opening and closing documents in the LSP server, reading files from disk,
//! and waiting for the server to index documents.

use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
use std::collections::{HashMap, HashSet};
use tokio::{fs, time::Duration};
use tracing::{debug, info, warn};

/// Manages document synchronization with LSP server
pub struct DocumentSyncManager;

impl DocumentSyncManager {
    /// Synchronize documents with the LSP server
    ///
    /// Opens all specified files in the LSP server, reads their contents,
    /// and optionally waits for the server to index them.
    ///
    /// # Arguments
    /// * `definition_provider` - The LSP definition provider
    /// * `files` - Set of file paths to synchronize
    /// * `wait_for_indexing` - Whether to wait for LSP server to index documents
    /// * `wait_timeout` - Timeout for waiting (if wait_for_indexing is true)
    ///
    /// # Returns
    /// Tuple of (opened_documents, file_cache) where:
    /// - `opened_documents`: List of successfully opened document paths
    /// - `file_cache`: Map from file paths to their contents
    pub async fn sync_documents(
        definition_provider: &dyn DefinitionProvider,
        files: HashSet<String>,
        wait_for_indexing: bool,
        wait_timeout: Duration,
    ) -> (Vec<String>, HashMap<String, String>) {
        let t_sync = std::time::Instant::now();
        let file_count = files.len();

        let mut opened_documents = Vec::new();
        let mut file_cache = HashMap::new();
        let files_vec: Vec<String> = files.into_iter().collect();

        let t_open = std::time::Instant::now();
        for file in files_vec.iter() {
            match fs::read_to_string(file).await {
                Ok(contents) => {
                    let cache_contents = contents.clone();
                    let file_clone = file.clone();
                    match definition_provider.open_document(file, contents).await {
                        Ok(_) => {
                            opened_documents.push(file_clone.clone());
                            file_cache.insert(file_clone, cache_contents);
                        }
                        Err(err) => {
                            warn!(
                                "Failed to synchronize document '{}' with LSP session: {}",
                                file, err
                            );
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "Failed to read source file '{}' for LSP synchronization: {}",
                        file, err
                    );
                }
            }
        }
        info!(
            "[TIMING] Document didOpen loop ({} files, {} opened): {:?}",
            file_count,
            opened_documents.len(),
            t_open.elapsed()
        );

        if wait_for_indexing && !opened_documents.is_empty() {
            let t_index = std::time::Instant::now();
            debug!(
                "Waiting for LSP server to index {} opened documents",
                opened_documents.len()
            );
            if let Err(e) = definition_provider.wait_until_ready(wait_timeout).await {
                warn!(
                    "LSP server did not become ready after opening documents: {}",
                    e
                );
            }
            info!(
                "[TIMING] Document indexing wait ({} docs): {:?}",
                opened_documents.len(),
                t_index.elapsed()
            );
        }

        info!(
            "[TIMING] sync_documents total ({} files): {:?}",
            file_count,
            t_sync.elapsed()
        );
        (opened_documents, file_cache)
    }

    /// Close documents in the LSP server
    ///
    /// # Arguments
    /// * `definition_provider` - The LSP definition provider
    /// * `files` - List of file paths to close
    pub async fn close_documents(definition_provider: &dyn DefinitionProvider, files: &[String]) {
        for file in files {
            if let Err(err) = definition_provider.close_document(file).await {
                warn!(
                    "Failed to close synchronized document '{}' in LSP session: {}",
                    file, err
                );
            }
        }
    }

    /// Synchronize documents and return only the opened documents (no file cache)
    ///
    /// This is a convenience method for cases where file contents aren't needed.
    pub async fn sync_documents_simple(
        definition_provider: &dyn DefinitionProvider,
        files: HashSet<String>,
        wait_for_indexing: bool,
        wait_timeout: Duration,
    ) -> Vec<String> {
        let (opened, _) =
            Self::sync_documents(definition_provider, files, wait_for_indexing, wait_timeout).await;
        opened
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::lsp::definition_provider::DefinitionProvider;
    use crate::infrastructure::lsp::errors::LspError;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::time::Duration;

    struct MockProvider {
        opened: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl DefinitionProvider for MockProvider {
        async fn is_available(&self) -> bool {
            true
        }

        async fn ensure_ready(&self) -> Result<(), LspError> {
            Ok(())
        }

        async fn find_definition(
            &self,
            _call_site: &crate::application::ports::CallSite,
        ) -> Result<Option<crate::domain::Range>, LspError> {
            Ok(None)
        }

        async fn open_document(&self, path: &str, _text: String) -> Result<(), LspError> {
            self.opened.lock().unwrap().push(path.to_string());
            Ok(())
        }

        async fn close_document(&self, _path: &str) -> Result<(), LspError> {
            Ok(())
        }

        async fn wait_until_ready(&self, _timeout: Duration) -> Result<(), LspError> {
            Ok(())
        }

        async fn hover(
            &self,
            _location: &crate::domain::Range,
        ) -> Result<Option<String>, LspError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_sync_documents() {
        let provider = Arc::new(MockProvider {
            opened: Arc::new(std::sync::Mutex::new(Vec::new())),
        });

        let mut files = HashSet::new();
        files.insert("test.rs".to_string());

        let (opened, cache) = DocumentSyncManager::sync_documents(
            provider.as_ref(),
            files,
            false,
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(opened.len(), 0); // File doesn't exist, so won't be opened
        assert!(cache.is_empty());
    }
}
