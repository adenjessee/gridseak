//! Tree-sitter language grammar loader
//!
//! Loads Tree-sitter language grammars for different programming languages
//! based on configuration, plus the matching
//! [`LanguageSpecificExtractor`](super::LanguageSpecificExtractor) instance.

use std::sync::Arc;

use crate::infrastructure::config::LanguageConfig;
use crate::syntax::language::extractor::{GenericExtractor, LanguageSpecificExtractor};
use crate::syntax::language::extractors::{
    csharp::CSharpExtractor, go::GoExtractor, java::JavaExtractor, javascript::JavaScriptExtractor,
    python::PythonExtractor, rust::RustExtractor, typescript::TypeScriptExtractor, ApexExtractor,
};
use anyhow::Result;

/// Load the Tree-sitter language for the given configuration
///
/// # Arguments
/// * `config` - The language configuration
///
/// # Returns
/// The Tree-sitter language for the configured language
///
/// # Errors
/// Returns an error if the language is not supported
pub fn load_language(config: &LanguageConfig) -> Result<tree_sitter::Language> {
    match config.language.as_str() {
        "rust" => {
            // Load the real Tree-sitter Rust grammar
            Ok(tree_sitter_rust::language())
        }
        "python" => Ok(tree_sitter_python::language()),
        "javascript" => Ok(tree_sitter_javascript::language()),
        // TypeScript uses its own grammar - TSX is a superset that handles both .ts and .tsx
        "typescript" => Ok(tree_sitter_typescript::language_tsx()),
        "go" => Ok(tree_sitter_go::language()),
        "java" => Ok(tree_sitter_java::language()),
        "csharp" => Ok(tree_sitter_c_sharp::language()),
        "apex" => Ok(tree_sitter_sfapex_vendored::apex::language()),
        _ => Err(anyhow::anyhow!(
            "Unsupported language for Tree-sitter: {}. Supported: rust, python, javascript, typescript, go, java, csharp, apex",
            config.language
        )),
    }
}

/// Load the per-language [`LanguageSpecificExtractor`] for the given config.
///
/// Unknown languages receive a [`GenericExtractor`] whose predicates all
/// return `false` — identical behaviour to the pre-refactor `_ => false`
/// match arm.
pub fn load_extractor(config: &LanguageConfig) -> Arc<dyn LanguageSpecificExtractor> {
    load_extractor_for_language(&config.language)
}

/// Variant of [`load_extractor`] that keys directly off a language
/// identifier. Introduced for the `UF-FU-004` follow-up: the shape
/// we want at the pipeline boundary is
/// `fn language_extractor_for_file(path) -> Arc<dyn LanguageSpecificExtractor>`
/// so multi-language scans (a repo with both Rust and TypeScript, for
/// example) can dispatch per-file rather than one extractor per scan.
///
/// The registry itself lives here so
/// [`super::language_extractor_for_file`] can reuse the same mapping
/// without re-implementing the "extension → language → extractor"
/// chain. Until the pipeline refactor lands, the function is still
/// driven by the single-language `LanguageConfig` path; keeping the
/// mapping in one place means UF-FU-004 can be closed with a
/// pipeline-side change alone.
pub fn load_extractor_for_language(language: &str) -> Arc<dyn LanguageSpecificExtractor> {
    match language {
        "rust" => Arc::new(RustExtractor),
        "typescript" => Arc::new(TypeScriptExtractor),
        "javascript" => Arc::new(JavaScriptExtractor::default()),
        "python" => Arc::new(PythonExtractor),
        "go" => Arc::new(GoExtractor),
        "java" => Arc::new(JavaExtractor),
        "csharp" => Arc::new(CSharpExtractor),
        "apex" => Arc::new(ApexExtractor),
        other => Arc::new(GenericExtractor::new(other)),
    }
}

/// Map a source-file path to the `LanguageSpecificExtractor` that
/// handles its language, by file-extension lookup. Returns `None`
/// when the extension is absent or unknown so callers can decide
/// whether to skip the file or fall back to a generic extractor.
///
/// This is the target shape of the `UF-FU-004` registry spike. The
/// extension-to-language table below mirrors the discovery rules
/// hardcoded today in `FileDiscovery::discover_source_files`; once
/// the pipeline consumes this registry directly we can delete the
/// duplicated list there (the deletion is out of scope for Gate 1.2
/// — this PR only introduces the registry; the pipeline refactor
/// that consumes it is deferred to the UF-FU-004 follow-on).
pub fn language_extractor_for_file(
    path: &std::path::Path,
) -> Option<Arc<dyn LanguageSpecificExtractor>> {
    let language = language_for_extension(path)?;
    Some(load_extractor_for_language(language))
}

/// File-extension → language-identifier mapping. The language
/// identifiers are the canonical keys that `LanguageConfig::language`
/// uses today. Returns `None` for extensions we don't know about so
/// callers can decide whether to skip.
pub fn language_for_extension(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "cs" => Some("csharp"),
        "cls" | "trigger" | "apex" => Some("apex"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::create_default_rust_config;

    #[test]
    fn test_load_rust_language() {
        let config = create_default_rust_config();
        let result = load_language(&config);
        assert!(result.is_ok(), "Should load Rust language");
    }

    fn cfg_for(language: &str) -> LanguageConfig {
        LanguageConfig::new(
            language.to_string(),
            vec![],
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        )
    }

    #[test]
    fn load_extractor_maps_every_known_language() {
        let names = [
            "rust",
            "typescript",
            "javascript",
            "python",
            "go",
            "java",
            "csharp",
            "apex",
        ];
        for name in names {
            let extractor = load_extractor(&cfg_for(name));
            assert_eq!(extractor.language(), name, "wrong extractor for {name}");
        }
    }

    #[test]
    fn load_extractor_returns_generic_for_unknown_language() {
        let extractor = load_extractor(&cfg_for("klingon"));
        assert_eq!(extractor.language(), "klingon");
        assert!(!extractor.is_function_definition("function_item"));
    }

    #[test]
    fn language_for_extension_matches_expected_ids() {
        use std::path::Path;
        let cases: &[(&str, Option<&str>)] = &[
            ("lib.rs", Some("rust")),
            ("app.ts", Some("typescript")),
            ("app.tsx", Some("typescript")),
            ("util.js", Some("javascript")),
            ("script.py", Some("python")),
            ("main.go", Some("go")),
            ("App.java", Some("java")),
            ("Service.cs", Some("csharp")),
            ("MyApexClass.cls", Some("apex")),
            ("MyTrigger.trigger", Some("apex")),
            ("notes.md", None),
            ("Makefile", None),
        ];
        for (name, expected) in cases {
            assert_eq!(language_for_extension(Path::new(name)), *expected, "{name}");
        }
    }

    #[test]
    fn language_extractor_for_file_rust() {
        use std::path::Path;
        let extractor = language_extractor_for_file(Path::new("src/foo.rs")).expect("rs → rust");
        assert_eq!(extractor.language(), "rust");
    }

    #[test]
    fn language_extractor_for_file_unknown_returns_none() {
        use std::path::Path;
        assert!(language_extractor_for_file(Path::new("README.md")).is_none());
    }
}
