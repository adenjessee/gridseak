//! Configuration system for language-specific parsing
//!
//! Loads YAML configurations that define Tree-sitter queries and mappings
//! for different programming languages. Enables language-agnostic parsing
//! by mapping language-specific syntax to universal domain types.

use crate::domain::NodeKind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{debug, info, instrument, warn};

static CONFIGS_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
static CONFIG_FILE_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Override the configs directory for this process (CLI-friendly).
///
/// This makes config loading deterministic regardless of caller cwd.
pub fn set_configs_dir_override(path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "configs dir override does not exist: {}",
            path.display()
        ));
    }
    CONFIGS_DIR_OVERRIDE
        .set(path)
        .map_err(|_| anyhow::anyhow!("configs dir override already set"))?;
    Ok(())
}

/// Override the exact config file to load (CLI-friendly).
///
/// Intended for debugging/experimentation; the loaded config must match the requested language.
pub fn set_config_file_override(path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "config file override does not exist: {}",
            path.display()
        ));
    }
    CONFIG_FILE_OVERRIDE
        .set(path)
        .map_err(|_| anyhow::anyhow!("config file override already set"))?;
    Ok(())
}

fn resolved_configs_dir() -> PathBuf {
    if let Some(p) = CONFIGS_DIR_OVERRIDE.get() {
        return p.clone();
    }
    if let Ok(p) = std::env::var("GRAPHENGINE_CONFIGS_DIR") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }

    // Dev environment: workspace root via Cargo.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let candidate = PathBuf::from(manifest_dir).join("configs");
        if candidate.exists() {
            return candidate;
        }
    }

    // Runtime: relative to the executable location (works for installed binaries / CI).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for rel in ["configs", "../configs", "../../configs", "../../../configs"] {
                let candidate = exe_dir.join(rel);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    // Fallback: current working directory.
    PathBuf::from("configs")
}

/// Configuration for receiver type detection (trait/interface vs concrete)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReceiverTypeDetectionConfig {
    /// Patterns that indicate trait/interface objects (e.g., "&dyn", "interface")
    #[serde(default)]
    pub trait_object_patterns: Vec<String>,

    /// Patterns that indicate concrete types (optional, for validation)
    #[serde(default)]
    pub concrete_patterns: Vec<String>,

    /// How to extract type from LSP hover response
    #[serde(default)]
    pub hover_extraction: HoverExtractionConfig,
}

/// Configuration for extracting type information from LSP hover responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverExtractionConfig {
    /// Try to extract from markdown code blocks (```language\ntype\n```)
    #[serde(default = "default_true")]
    pub markdown_code_block: bool,

    /// Try to extract from function signature patterns
    #[serde(default = "default_true")]
    pub function_signature: bool,

    /// Fallback to first line if other methods fail
    #[serde(default = "default_true")]
    pub first_line_fallback: bool,
}

impl Default for HoverExtractionConfig {
    fn default() -> Self {
        Self {
            markdown_code_block: true,
            function_signature: true,
            first_line_fallback: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Configuration for a specific programming language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageConfig {
    /// Language name (e.g., "rust", "javascript")
    pub language: String,

    /// File extensions supported by this language
    pub file_extensions: Vec<String>,

    /// Tree-sitter queries for extracting different types of symbols
    pub queries: HashMap<String, String>,

    /// Mapping from Tree-sitter node types to domain NodeKind
    pub kind_mappings: HashMap<String, NodeKind>,

    /// Path to the Tree-sitter grammar (optional, for dynamic loading)
    pub grammar_path: Option<String>,

    /// LSP command for semantic resolution
    pub lsp_command: Option<String>,

    /// LSP command arguments
    pub lsp_args: Option<Vec<String>>,

    /// Version of the configuration schema
    pub version: String,

    /// Receiver type detection configuration (for trait/interface vs concrete calls)
    #[serde(default)]
    pub receiver_type_detection: Option<ReceiverTypeDetectionConfig>,

    /// LSP request timeout in milliseconds (default: 5000)
    #[serde(default)]
    pub lsp_request_timeout_ms: Option<u32>,

    /// Maximum concurrent in-flight LSP requests (default: 32)
    #[serde(default)]
    pub lsp_max_concurrent_requests: Option<u32>,

    /// LSP `initializationOptions` payload sent during the `initialize` request.
    ///
    /// When `None` (default), `SimpleLspClient::initialize` uses the existing
    /// `create_initialize_request_minimal` path — preserving exact byte-for-byte
    /// behavior for languages that do not opt in (rust-analyzer, jdtls, omnisharp,
    /// pyright, gopls, ts-server). When `Some`, the client switches to
    /// `create_initialize_request_with_options` so the LSP receives these options.
    ///
    /// Required for LSPs that need workspace configuration at init time
    /// (e.g. apex-jorje requires `enableSemanticErrors`).
    #[serde(default)]
    pub lsp_initialization_options: Option<serde_json::Value>,
}

impl LanguageConfig {
    /// Create a new language configuration
    pub fn new(
        language: String,
        file_extensions: Vec<String>,
        queries: HashMap<String, String>,
        kind_mappings: HashMap<String, NodeKind>,
    ) -> Self {
        Self {
            language,
            file_extensions,
            queries,
            kind_mappings,
            grammar_path: None,
            lsp_command: None,
            lsp_args: None,
            version: "1.0".to_string(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        }
    }

    /// Validate the configuration for completeness and correctness
    pub fn validate(&self) -> Result<()> {
        // Check required fields
        if self.language.is_empty() {
            return Err(anyhow::anyhow!("Language name cannot be empty"));
        }

        if self.file_extensions.is_empty() {
            return Err(anyhow::anyhow!(
                "At least one file extension must be specified"
            ));
        }

        // Check required queries
        let required_queries = ["functions", "structs", "modules", "call_sites"];
        for query_name in &required_queries {
            if !self.queries.contains_key(*query_name) {
                return Err(anyhow::anyhow!(
                    "Missing required query: {} for language: {}",
                    query_name,
                    self.language
                ));
            }
        }

        // Check that queries are valid Tree-sitter syntax (basic validation)
        for (name, query) in &self.queries {
            if query.trim().is_empty() {
                return Err(anyhow::anyhow!(
                    "Query '{}' cannot be empty for language: {}",
                    name,
                    self.language
                ));
            }

            // Basic syntax validation - should contain capture groups
            if !query.contains("@") {
                warn!(
                    "Query '{}' for language '{}' does not contain capture groups (@)",
                    name, self.language
                );
            }
        }

        // Check kind mappings
        if self.kind_mappings.is_empty() {
            return Err(anyhow::anyhow!(
                "At least one kind mapping must be specified for language: {}",
                self.language
            ));
        }

        // Validate that we have mappings for common node types
        let common_types = ["function_item", "struct_item", "mod_item"];
        for node_type in &common_types {
            if !self.kind_mappings.contains_key(*node_type) {
                warn!(
                    "No kind mapping found for common node type '{}' in language '{}'",
                    node_type, self.language
                );
            }
        }

        info!(
            "Configuration validated successfully for language: {}",
            self.language
        );
        Ok(())
    }

    /// Check if a file extension is supported by this language
    pub fn supports_extension(&self, ext: &str) -> bool {
        self.file_extensions.iter().any(|e| e == ext)
    }

    /// Get the query for a specific symbol type
    pub fn get_query(&self, query_name: &str) -> Option<&String> {
        self.queries.get(query_name)
    }

    /// Get the NodeKind for a Tree-sitter node type
    pub fn get_node_kind(&self, node_type: &str) -> Option<NodeKind> {
        self.kind_mappings.get(node_type).copied()
    }

    /// Get receiver type detection configuration
    pub fn get_receiver_type_detection(&self) -> Option<&ReceiverTypeDetectionConfig> {
        self.receiver_type_detection.as_ref()
    }
}

/// Load a language configuration from a YAML file
#[instrument(skip(language))]
pub fn load_config(language: &str) -> Result<LanguageConfig> {
    if let Some(config_file) = CONFIG_FILE_OVERRIDE.get() {
        return load_config_from_file(language, config_file);
    }

    let configs_dir = resolved_configs_dir();
    let path = configs_dir.join(format!("{}.yaml", language));

    if !path.exists() {
        return Err(anyhow::anyhow!(
            "Configuration file not found: {}. Searched configs dir: {}. Available languages: {:?}",
            path.display(),
            configs_dir.display(),
            get_available_languages()?
        ));
    }

    debug!("Loading configuration from: {}", path.display());

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    debug!("Read {} bytes from config file", content.len());

    let mut config: LanguageConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse YAML config: {}", path.display()))?;

    // Override language name from filename if not set in YAML
    if config.language.is_empty() {
        config.language = language.to_string();
    }

    // Validate the loaded configuration
    config
        .validate()
        .with_context(|| format!("Configuration validation failed for language: {}", language))?;

    if config.language != language {
        warn!(
            "Loaded config language '{}' does not match requested '{}'",
            config.language, language
        );
    }

    info!(
        "Loaded configuration for language: {} ({} queries, {} mappings)",
        language,
        config.queries.len(),
        config.kind_mappings.len()
    );
    Ok(config)
}

/// Lightweight descriptor of a language config — only the fields needed to
/// answer "what languages does this engine know about?" without running the
/// full tree-sitter-aware validation in `load_config`.
///
/// This is the right loader for listing-style commands (`languages --json`,
/// desktop onboarding detection) because it also accepts discovery-only
/// configs that deliberately omit `queries`/`kind_mappings` (e.g. Visualforce,
/// which is XML-read rather than tree-sitter-parsed). Using `load_config` on
/// those would incorrectly reject them and hide them from the UI.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LanguageDescriptor {
    pub language: String,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub lsp_command: Option<String>,
    /// Marks configs that exist purely so the discovery layer can recognize a
    /// file type — they are **not** consumable by the `parse` pipeline on
    /// their own. Visualforce is the canonical case: `.page` files are read
    /// by `syntax::language::apex::vf_page_reader` as part of the Apex scan,
    /// not by a standalone tree-sitter grammar. Telling the UI/shell via this
    /// flag lets them list VF as "recognized" while preventing a spurious
    /// `parse --lang visualforce` that would trip the strict `load_config`
    /// validation (which rightly rejects configs lacking `queries`).
    #[serde(default)]
    pub discovery_only: bool,
}

/// Load just the descriptor fields from `{configs_dir}/{language}.yaml`.
/// Tolerant of discovery-only configs.
pub fn load_language_descriptor(language: &str) -> Result<LanguageDescriptor> {
    let configs_dir = resolved_configs_dir();
    let path = configs_dir.join(format!("{}.yaml", language));

    if !path.exists() {
        return Err(anyhow::anyhow!(
            "Configuration file not found: {}",
            path.display()
        ));
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let mut descriptor: LanguageDescriptor = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse YAML descriptor: {}", path.display()))?;

    if descriptor.language.is_empty() {
        descriptor.language = language.to_string();
    }

    Ok(descriptor)
}

pub fn load_config_from_file(language: &str, path: &Path) -> Result<LanguageConfig> {
    debug!(
        "Loading configuration override for language '{}' from: {}",
        language,
        path.display()
    );
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let mut config: LanguageConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse YAML config: {}", path.display()))?;

    if config.language.is_empty() {
        config.language = language.to_string();
    }
    if config.language != language {
        return Err(anyhow::anyhow!(
            "Config file language '{}' does not match requested '{}'",
            config.language,
            language
        ));
    }

    config.validate().with_context(|| {
        format!(
            "Configuration validation failed for language '{}' (file: {})",
            language,
            path.display()
        )
    })?;
    Ok(config)
}

/// Get list of available language configurations
pub fn get_available_languages() -> Result<Vec<String>> {
    let configs_dir = resolved_configs_dir();

    if !configs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut languages = Vec::new();

    for entry in std::fs::read_dir(&configs_dir).with_context(|| {
        format!(
            "Failed to read configs directory: {}",
            configs_dir.display()
        )
    })? {
        let entry = entry.with_context(|| "Failed to read config directory entry")?;
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "yaml") {
            if let Some(stem) = path.file_stem() {
                if let Some(lang) = stem.to_str() {
                    languages.push(lang.to_string());
                }
            }
        }
    }

    languages.sort();
    Ok(languages)
}

/// Create a default Rust configuration for testing
pub fn create_default_rust_config() -> LanguageConfig {
    let mut queries = HashMap::new();
    queries.insert(
        "functions".to_string(),
        "(function_item name: (identifier) @name) @func".to_string(),
    );
    queries.insert(
        "structs".to_string(),
        "(struct_item name: (identifier) @name) @struct".to_string(),
    );
    queries.insert(
        "modules".to_string(),
        "(mod_item name: (identifier) @name) @module".to_string(),
    );
    queries.insert(
        "call_sites".to_string(),
        "(call_expression function: (identifier) @func) @call".to_string(),
    );
    queries.insert(
        "imports".to_string(),
        "(use_declaration) @import".to_string(),
    );
    queries.insert(
        "type_refs".to_string(),
        "(type_identifier) @type".to_string(),
    );

    let mut kind_mappings = HashMap::new();
    kind_mappings.insert("function_item".to_string(), NodeKind::Function);
    kind_mappings.insert("struct_item".to_string(), NodeKind::Struct);
    kind_mappings.insert("mod_item".to_string(), NodeKind::Module);
    kind_mappings.insert("trait_item".to_string(), NodeKind::Interface);
    kind_mappings.insert("enum_item".to_string(), NodeKind::Enum);
    kind_mappings.insert("const_item".to_string(), NodeKind::Variable);
    kind_mappings.insert("static_item".to_string(), NodeKind::Variable);
    kind_mappings.insert("type_item".to_string(), NodeKind::Type);
    kind_mappings.insert("use_declaration".to_string(), NodeKind::Import);

    LanguageConfig::new(
        "rust".to_string(),
        vec![".rs".to_string()],
        queries,
        kind_mappings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_language_config_creation() {
        let mut queries = HashMap::new();
        queries.insert("functions".to_string(), "(function_item) @func".to_string());

        let mut kind_mappings = HashMap::new();
        kind_mappings.insert("function_item".to_string(), NodeKind::Function);

        let config = LanguageConfig::new(
            "rust".to_string(),
            vec![".rs".to_string()],
            queries,
            kind_mappings,
        );

        assert_eq!(config.language, "rust");
        assert_eq!(config.file_extensions, vec![".rs"]);
        assert!(config.supports_extension(".rs"));
        assert!(!config.supports_extension(".js"));
    }

    #[test]
    fn test_language_config_validation() {
        let mut queries = HashMap::new();
        queries.insert("functions".to_string(), "(function_item) @func".to_string());
        queries.insert("structs".to_string(), "(struct_item) @struct".to_string());
        queries.insert("modules".to_string(), "(mod_item) @module".to_string());
        queries.insert(
            "call_sites".to_string(),
            "(call_expression) @call".to_string(),
        );

        let mut kind_mappings = HashMap::new();
        kind_mappings.insert("function_item".to_string(), NodeKind::Function);

        let config = LanguageConfig::new(
            "rust".to_string(),
            vec![".rs".to_string()],
            queries,
            kind_mappings,
        );

        // Should validate successfully
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_language_config_validation_failures() {
        // Empty language name
        let config = LanguageConfig::new(
            "".to_string(),
            vec![".rs".to_string()],
            HashMap::new(),
            HashMap::new(),
        );
        assert!(config.validate().is_err());

        // No file extensions
        let config =
            LanguageConfig::new("rust".to_string(), vec![], HashMap::new(), HashMap::new());
        assert!(config.validate().is_err());

        // Missing required queries
        let mut queries = HashMap::new();
        queries.insert("functions".to_string(), "(function_item) @func".to_string());
        // Missing structs, modules, call_sites

        let mut kind_mappings = HashMap::new();
        kind_mappings.insert("function_item".to_string(), NodeKind::Function);

        let config = LanguageConfig::new(
            "rust".to_string(),
            vec![".rs".to_string()],
            queries,
            kind_mappings,
        );
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_default_rust_config() {
        let config = create_default_rust_config();

        assert_eq!(config.language, "rust");
        assert!(config.supports_extension(".rs"));
        assert!(config.get_query("functions").is_some());
        assert!(config.get_node_kind("function_item").is_some());
        assert_eq!(
            config.get_node_kind("function_item"),
            Some(NodeKind::Function)
        );

        // Should validate successfully
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_get_available_languages() {
        // This test will depend on what config files exist
        let languages = get_available_languages().unwrap();
        // At minimum, list creation should succeed without panicking.
        assert!(languages.into_iter().all(|lang| !lang.is_empty()));
    }
}
