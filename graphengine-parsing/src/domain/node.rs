//! Node representation for parsed code elements
//!
//! Adapted from the old core system's UniversalNode with stable hashing.
//! Represents semantic code entities with deterministic IDs.

use super::node_id::compute_stable_id;
use super::provenance::Provenance;
use std::collections::HashMap;

/// Type of code element represented by a node
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NodeKind {
    /// Function or method
    Function,
    /// Struct or class
    Struct,
    /// Module or namespace
    Module,
    /// Interface or trait
    Interface,
    /// Enum type
    Enum,
    /// Variable or field
    Variable,
    /// Type alias or typedef
    Type,
    /// Import or use statement
    Import,
    /// Project or repository root (top-level container)
    Project,
    /// Crate (compilation unit)
    Crate,
    /// Source file (.rs file)
    File,
    /// Directory or folder
    Folder,
}

/// Location range within a file
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Range {
    /// Starting line number (1-based)
    pub start_line: u32,
    /// Starting character position (0-based)
    pub start_char: u32,
    /// Ending line number (1-based)
    pub end_line: u32,
    /// Ending character position (0-based)
    pub end_char: u32,
    /// File path for this range (absolute or project-relative)
    pub file: String,
}

impl Range {
    /// Create a new range with a default placeholder file path
    pub fn new(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Self {
        Self::with_file(
            start_line,
            start_char,
            end_line,
            end_char,
            "<unknown>".to_string(),
        )
    }

    /// Create a new range with an explicit file path
    pub fn with_file<S: Into<String>>(
        start_line: u32,
        start_char: u32,
        end_line: u32,
        end_char: u32,
        file: S,
    ) -> Self {
        Self {
            start_line,
            start_char,
            end_line,
            end_char,
            file: file.into(),
        }
    }

    /// Create a range using a placeholder file path for tests
    pub fn test(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Self {
        Self::with_file(
            start_line,
            start_char,
            end_line,
            end_char,
            "test.rs".to_string(),
        )
    }
}

/// A node representing a semantic code element
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Node {
    /// Stable, deterministic ID based on content and location
    pub id: String,
    /// Type of code element
    pub kind: NodeKind,
    /// Fully qualified name (e.g., "crate::module::function")
    pub fqn: String,
    /// Location within the source file
    pub location: Range,
    /// Provenance information
    pub provenance: Provenance,
    /// Extra stable properties for templates/clients (classification, paths, etc.)
    ///
    /// This is intentionally schemaless JSON to avoid repeated DB migrations for new keys.
    /// Keys used for cross-client contracts should be treated as stable once published.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, serde_json::Value>,
    /// Trait metadata (for functions that are trait methods)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trait_metadata: Option<TraitMetadata>,
}

/// Metadata about trait method relationships
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TraitMetadata {
    /// Name of the trait this method belongs to
    pub trait_name: String,
    /// Whether this is a default implementation in the trait definition
    pub is_trait_default: bool,
    /// The type implementing this trait (if this is an implementation, not a default)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementing_type: Option<String>,
}

impl Node {
    /// Create a new node with FQN-only stable ID.
    ///
    /// Use this constructor for container nodes (file, folder, module,
    /// project, crate) and synthetic scaffolding where "body" is not a
    /// meaningful concept. For real source-code symbols where the body is
    /// available, prefer [`Node::with_body`] — it produces IDs that remain
    /// stable across formatter passes, blank-line insertions, and comment
    /// edits that leave the body semantically unchanged.
    pub fn new(kind: NodeKind, fqn: String, location: Range, provenance: Provenance) -> Self {
        let id = compute_stable_id(&fqn, None, None);
        Self {
            id,
            kind,
            fqn,
            location,
            provenance,
            properties: HashMap::new(),
            trait_metadata: None,
        }
    }

    /// Create a new node whose ID incorporates its normalized body text.
    ///
    /// `body` is the raw AST-level body text (including braces, signature,
    /// etc.). `language` is a comment-syntax hint — the string the parsing
    /// pipeline uses today (e.g. `"rust"`, `"apex"`, `"python"`). Pass
    /// `None` when the language is unknown; the normalizer will fall back
    /// to a safe C-family-only comment-stripping mode.
    ///
    /// See `domain::node_id` for the full normalization contract and its
    /// limitations.
    pub fn with_body(
        kind: NodeKind,
        fqn: String,
        location: Range,
        provenance: Provenance,
        body: &str,
        language: Option<&str>,
    ) -> Self {
        let id = compute_stable_id(&fqn, Some(body), language);
        Self {
            id,
            kind,
            fqn,
            location,
            provenance,
            properties: HashMap::new(),
            trait_metadata: None,
        }
    }

    /// Create a new node with trait metadata (FQN-only stable ID).
    ///
    /// Prefer [`Node::with_trait_metadata_and_body`] when the body is
    /// available; this variant is retained for sites that synthesize trait
    /// metadata without direct access to the AST body (e.g. cross-file
    /// dispatch resolution).
    pub fn with_trait_metadata(
        kind: NodeKind,
        fqn: String,
        location: Range,
        provenance: Provenance,
        trait_metadata: TraitMetadata,
    ) -> Self {
        let id = compute_stable_id(&fqn, None, None);
        Self {
            id,
            kind,
            fqn,
            location,
            provenance,
            properties: HashMap::new(),
            trait_metadata: Some(trait_metadata),
        }
    }

    /// Create a new node with trait metadata and a body-aware stable ID.
    pub fn with_trait_metadata_and_body(
        kind: NodeKind,
        fqn: String,
        location: Range,
        provenance: Provenance,
        trait_metadata: TraitMetadata,
        body: &str,
        language: Option<&str>,
    ) -> Self {
        let id = compute_stable_id(&fqn, Some(body), language);
        Self {
            id,
            kind,
            fqn,
            location,
            provenance,
            properties: HashMap::new(),
            trait_metadata: Some(trait_metadata),
        }
    }

    /// Set a JSON property key/value on the node.
    pub fn set_property<V: Into<serde_json::Value>>(&mut self, key: &str, value: V) {
        self.properties.insert(key.to_string(), value.into());
    }

    /// Create a function node with Tree-sitter provenance (FQN-only ID).
    pub fn function(fqn: String, location: Range) -> Self {
        Self::new(NodeKind::Function, fqn, location, Provenance::tree_sitter())
    }

    /// Create a struct node with Tree-sitter provenance (FQN-only ID).
    pub fn struct_(fqn: String, location: Range) -> Self {
        Self::new(NodeKind::Struct, fqn, location, Provenance::tree_sitter())
    }

    /// Create a module node with Tree-sitter provenance (FQN-only ID).
    pub fn module(fqn: String, location: Range) -> Self {
        Self::new(NodeKind::Module, fqn, location, Provenance::tree_sitter())
    }
}
