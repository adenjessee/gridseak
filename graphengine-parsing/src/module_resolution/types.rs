use std::fmt;

use crate::application::ports::{ImportPath, ImportSpec, ImportVisibility, ModDecl};

/// Represents a module path as a sequence of segments (e.g. `crate::foo::bar`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ModulePath(pub Vec<String>);

impl ModulePath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_segments<S: Into<String>>(segments: impl IntoIterator<Item = S>) -> Self {
        Self(segments.into_iter().map(Into::into).collect())
    }

    pub fn push<S: Into<String>>(&mut self, segment: S) {
        self.0.push(segment.into());
    }

    pub fn join(&self, separator: &str) -> String {
        self.0.join(separator)
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }

    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            None
        } else {
            let mut segments = self.0.clone();
            segments.pop();
            Some(Self(segments))
        }
    }

    pub fn shared_prefix_len(&self, other: &ModulePath) -> usize {
        self.0
            .iter()
            .zip(other.0.iter())
            .take_while(|(a, b)| a == b)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for ModulePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.join("::"))
    }
}

/// Origin of a name binding when resolving identifiers in context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingOrigin {
    Local,
    Alias,
    Glob,
    SelfPath,
    SuperPath(u8),
    CratePath,
    ExternalCrate,
    Unresolved,
}

/// Relative confidence weight used when ordering candidate resolutions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfidenceWeight {
    High,
    Medium,
    Low,
}

/// Resolved fully-qualified name candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedName {
    pub fqn: String,
    pub origin: BindingOrigin,
    pub confidence: ConfidenceWeight,
    pub module_path: ModulePath,
}

impl ResolvedName {
    pub fn new(
        fqn: String,
        module_path: ModulePath,
        origin: BindingOrigin,
        confidence: ConfidenceWeight,
    ) -> Self {
        Self {
            fqn,
            module_path,
            origin,
            confidence,
        }
    }
}

/// Wrapper around structured import data for downstream modules.
#[derive(Debug, Clone)]
pub struct ImportRecord {
    pub spec: ImportSpec,
    pub binding: String,
}

/// Module declaration record produced by the syntax layer.
pub type ModuleDeclaration = ModDecl;

/// Convenience alias for import paths captured during syntax extraction.
pub type UsePath = ImportPath;

/// Visibility alias exported for convenience.
pub type UseVisibility = ImportVisibility;
