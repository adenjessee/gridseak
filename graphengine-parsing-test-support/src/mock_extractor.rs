//! Config-aware mock syntax extractor that simulates Tree-sitter
//! parsing via line-based pattern matching.
//!
//! Relocated from `graphengine-parsing/src/infrastructure/mock_extractor.rs`
//! by R2 (v0.1.0-rc1 follow-up). Behaviour is identical to the
//! pre-relocation file. This fake is heavier than
//! [`crate::application_mocks::MockSyntaxExtractor`] (which always
//! returns a single hand-crafted symbol): it walks the file content
//! looking for `fn` / `struct` / `mod` / call sites / `use` / type
//! annotations, producing realistic-shaped `SyntaxResults` against
//! which infrastructure-level tests and benches exercise the full
//! application-layer wiring.

use anyhow::Result;
use async_trait::async_trait;
use graphengine_parsing::application::ports::{SyntaxExtractor, SyntaxResults};
use graphengine_parsing::domain::{
    Confidence, Node, NodeKind, Provenance, ProvenanceSource, Range,
};
use graphengine_parsing::infrastructure::config::LanguageConfig;
use rayon::prelude::*;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

/// Mock syntax extractor that simulates Tree-sitter parsing
pub struct MockSyntaxExtractor {
    config: Arc<LanguageConfig>,
}

impl MockSyntaxExtractor {
    /// Create a new mock syntax extractor
    pub fn new(config: LanguageConfig) -> Self {
        info!(
            "Created MockSyntaxExtractor for language: {}",
            config.language
        );
        Self {
            config: Arc::new(config),
        }
    }

    /// Simulate parsing a single file
    fn parse_file(&self, file_path: &Path, content: &str) -> Result<SyntaxResults> {
        let mut results = SyntaxResults::new();

        let lines: Vec<&str> = content.lines().collect();

        for (line_num, line) in lines.iter().enumerate() {
            let line = line.trim();

            if line.starts_with("fn ") && line.contains("(") {
                if let Some(name) = self.extract_function_name(line) {
                    let range = Range::with_file(
                        line_num as u32 + 1,
                        0,
                        line_num as u32 + 1,
                        line.len() as u32,
                        file_path.to_string_lossy().to_string(),
                    );
                    let fqn = format!("{}::{}", self.config.language, name);
                    let provenance =
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium);
                    let node = Node::new(NodeKind::Function, fqn, range, provenance);
                    results.add_symbol(node);
                }
            }

            if line.starts_with("struct ") && line.contains("{") {
                if let Some(name) = self.extract_struct_name(line) {
                    let range = Range::with_file(
                        line_num as u32 + 1,
                        0,
                        line_num as u32 + 1,
                        line.len() as u32,
                        file_path.to_string_lossy().to_string(),
                    );
                    let fqn = format!("{}::{}", self.config.language, name);
                    let provenance =
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium);
                    let node = Node::new(NodeKind::Struct, fqn, range, provenance);
                    results.add_symbol(node);
                }
            }

            if line.starts_with("mod ") {
                if let Some(name) = self.extract_module_name(line) {
                    let range = Range::with_file(
                        line_num as u32 + 1,
                        0,
                        line_num as u32 + 1,
                        line.len() as u32,
                        file_path.to_string_lossy().to_string(),
                    );
                    let fqn = format!("{}::{}", self.config.language, name);
                    let provenance =
                        Provenance::new(ProvenanceSource::TreeSitter, Confidence::Medium);
                    let node = Node::new(NodeKind::Module, fqn, range, provenance);
                    results.add_symbol(node);
                }
            }

            if line.contains("(") && line.contains(")") && !line.starts_with("fn ") {
                let words: Vec<&str> = line.split_whitespace().collect();
                for word in words {
                    if word.contains("(") {
                        let func_name = word.split('(').next().unwrap_or("");
                        if !func_name.is_empty()
                            && func_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        {
                            let range = Range::with_file(
                                line_num as u32 + 1,
                                line.find(func_name).unwrap_or(0) as u32,
                                line_num as u32 + 1,
                                (line.find(func_name).unwrap_or(0) + func_name.len()) as u32,
                                file_path.to_string_lossy().to_string(),
                            );
                            results.add_call_site(range, func_name.to_string());
                        }
                    }
                }
            }

            if line.starts_with("use ") {
                let range = Range::with_file(
                    line_num as u32 + 1,
                    0,
                    line_num as u32 + 1,
                    line.len() as u32,
                    file_path.to_string_lossy().to_string(),
                );
                results.add_import(range);
            }

            if line.contains(": ") && !line.starts_with("//") {
                let parts: Vec<&str> = line.split(": ").collect();
                if parts.len() >= 2 {
                    let type_part = parts[1].split_whitespace().next().unwrap_or("");
                    if !type_part.is_empty()
                        && type_part.chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        let range = Range::with_file(
                            line_num as u32 + 1,
                            line.find(type_part).unwrap_or(0) as u32,
                            line_num as u32 + 1,
                            (line.find(type_part).unwrap_or(0) + type_part.len()) as u32,
                            file_path.to_string_lossy().to_string(),
                        );
                        results.add_type_ref(range);
                    }
                }
            }
        }

        info!(
            "Mock extracted {} symbols, {} unresolved references, {} imports, {} type refs from {}",
            results.symbols.len(),
            results.references.len(),
            results.imports.len(),
            results.type_refs.len(),
            file_path.display()
        );

        Ok(results)
    }

    fn extract_function_name(&self, line: &str) -> Option<String> {
        if let Some(start) = line.find("fn ") {
            let after_fn = &line[start + 3..];
            if let Some(end) = after_fn.find('(') {
                let name = after_fn[..end].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    fn extract_struct_name(&self, line: &str) -> Option<String> {
        if let Some(start) = line.find("struct ") {
            let after_struct = &line[start + 7..];
            if let Some(end) = after_struct.find(' ') {
                let name = after_struct[..end].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    fn extract_module_name(&self, line: &str) -> Option<String> {
        if let Some(start) = line.find("mod ") {
            let after_mod = &line[start + 4..];
            let name = after_mod.split_whitespace().next().unwrap_or("");
            if !name.is_empty() {
                let clean_name = name.trim_end_matches(';');
                return Some(clean_name.to_string());
            }
        }
        None
    }
}

#[async_trait]
impl SyntaxExtractor for MockSyntaxExtractor {
    async fn extract(&self, files: &[std::path::PathBuf]) -> Result<SyntaxResults> {
        info!("Starting mock syntax extraction for {} files", files.len());

        if files.is_empty() {
            return Ok(SyntaxResults::new());
        }

        let supported_files: Vec<_> = files
            .iter()
            .filter(|file| {
                if let Some(ext) = file.extension() {
                    if let Some(ext_str) = ext.to_str() {
                        return self.config.supports_extension(&format!(".{}", ext_str));
                    }
                }
                false
            })
            .collect();

        info!(
            "Processing {} supported files out of {} total",
            supported_files.len(),
            files.len()
        );

        let parse_results: Result<Vec<_>> = supported_files
            .par_iter()
            .map(|file_path| match std::fs::read_to_string(file_path) {
                Ok(content) => match self.parse_file(file_path, &content) {
                    Ok(results) => Ok(results),
                    Err(e) => {
                        warn!("Failed to parse file {}: {}", file_path.display(), e);
                        Ok(SyntaxResults::new())
                    }
                },
                Err(e) => {
                    warn!("Failed to read file {}: {}", file_path.display(), e);
                    Ok(SyntaxResults::new())
                }
            })
            .collect();

        let parse_results = parse_results?;

        let mut final_results = SyntaxResults::new();
        for mut results in parse_results {
            final_results.symbols.append(&mut results.symbols);
            final_results.references.append(&mut results.references);
            final_results.imports.append(&mut results.imports);
            final_results.type_refs.append(&mut results.type_refs);
            final_results
                .type_references
                .append(&mut results.type_references);
        }

        info!(
            "Mock extraction complete: {} symbols, {} unresolved references, {} imports, {} type refs, {} structured type refs",
            final_results.symbols.len(),
            final_results.references.len(),
            final_results.imports.len(),
            final_results.type_refs.len(),
            final_results.type_references.len()
        );

        Ok(final_results)
    }

    fn supported_language(&self) -> &str {
        &self.config.language
    }

    fn supports_extension(&self, ext: &str) -> bool {
        self.config.supports_extension(ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphengine_parsing::infrastructure::config::create_default_rust_config;
    use std::path::PathBuf;

    #[test]
    fn test_mock_extractor_creation() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        assert_eq!(extractor.supported_language(), "rust");
        assert!(extractor.supports_extension(".rs"));
        assert!(!extractor.supports_extension(".js"));
    }

    #[test]
    fn test_extract_function_name() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        assert_eq!(
            extractor.extract_function_name("fn main() {"),
            Some("main".to_string())
        );
        assert_eq!(
            extractor.extract_function_name("fn test_function() {"),
            Some("test_function".to_string())
        );
        assert_eq!(
            extractor.extract_function_name("fn complex_function(param: i32) {"),
            Some("complex_function".to_string())
        );
        assert_eq!(extractor.extract_function_name("not a function"), None);
    }

    #[test]
    fn test_extract_struct_name() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        assert_eq!(
            extractor.extract_struct_name("struct MyStruct {"),
            Some("MyStruct".to_string())
        );
        assert_eq!(
            extractor.extract_struct_name("struct Point { x: i32, y: i32 }"),
            Some("Point".to_string())
        );
        assert_eq!(extractor.extract_struct_name("not a struct"), None);
    }

    #[test]
    fn test_extract_module_name() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        assert_eq!(
            extractor.extract_module_name("mod my_module"),
            Some("my_module".to_string())
        );
        assert_eq!(
            extractor.extract_module_name("mod utils;"),
            Some("utils".to_string())
        );
        assert_eq!(extractor.extract_module_name("not a module"), None);
    }

    #[test]
    fn test_parse_file() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        let test_code = r#"
fn main() {
    println!("Hello, world!");
}

struct Point {
    x: i32,
    y: i32,
}

mod utils {
    fn helper() {}
}
"#;

        let temp_file = std::env::temp_dir().join("test.rs");
        std::fs::write(&temp_file, test_code).unwrap();

        let results = extractor.parse_file(&temp_file, test_code).unwrap();

        assert!(!results.symbols.is_empty());
        assert!(results.symbols.iter().any(|n| n.kind == NodeKind::Function));
        assert!(results.symbols.iter().any(|n| n.kind == NodeKind::Struct));
        assert!(results.symbols.iter().any(|n| n.kind == NodeKind::Module));
    }

    #[tokio::test]
    async fn test_extract_empty_files() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        let files = vec![];
        let results = extractor.extract(&files).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_extract_unsupported_files() {
        let config = create_default_rust_config();
        let extractor = MockSyntaxExtractor::new(config);

        let files = vec![PathBuf::from("test.js")];
        let results = extractor.extract(&files).await.unwrap();
        assert!(results.is_empty());
    }
}
