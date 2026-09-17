//! [`SemanticIndex`] adapter over [`graphengine_scip_adapter::ScipIndex`].

use std::path::{Path, PathBuf};

use graphengine_scip_adapter::{ScipIndex, ScipIndexError};

use crate::application::ports::{IndexTarget, SemanticIndex};
use crate::domain::Confidence;

/// SCIP-backed semantic index implementing the parsing-layer port.
pub struct ScipSemanticIndex {
    inner: ScipIndex,
    language: String,
}

impl ScipSemanticIndex {
    pub fn from_file(
        path: impl AsRef<Path>,
        language: impl Into<String>,
    ) -> Result<Self, ScipIndexError> {
        let inner = ScipIndex::from_file(path)?;
        Ok(Self {
            inner,
            language: language.into(),
        })
    }

    pub fn from_file_at_workspace(
        path: impl AsRef<Path>,
        workspace_root: PathBuf,
        language: impl Into<String>,
    ) -> Result<Self, ScipIndexError> {
        let inner = ScipIndex::from_file(path)?.with_project_root(workspace_root);
        Ok(Self {
            inner,
            language: language.into(),
        })
    }

    pub fn project_root(&self) -> &Path {
        self.inner.project_root()
    }
}

impl SemanticIndex for ScipSemanticIndex {
    fn has_file(&self, file: &Path) -> bool {
        self.inner.has_document(file)
    }

    fn definition_at(&self, file: &Path, line: u32, col: u32) -> Option<IndexTarget> {
        let def = self.inner.definition_at(file, line, col)?;
        Some(IndexTarget {
            file: self.inner.project_root().join(&def.relative_path),
            line: def.line.saturating_add(1),
            col: def.character,
            symbol_moniker: def.symbol,
            confidence: Confidence::High,
        })
    }

    fn language(&self) -> &str {
        &self.language
    }

    fn implementations_of(&self, symbol: &str) -> Vec<IndexTarget> {
        let impls = self.inner.implementations_of(symbol);
        let n = impls.len();
        let confidence = if n > 1 {
            Confidence::Medium
        } else {
            Confidence::High
        };
        impls
            .into_iter()
            .map(|moniker| {
                if let Some(def) = self.inner.definition_of(&moniker) {
                    IndexTarget {
                        file: self.inner.project_root().join(&def.relative_path),
                        line: def.line.saturating_add(1),
                        col: def.character,
                        symbol_moniker: moniker,
                        confidence,
                    }
                } else {
                    IndexTarget {
                        file: self.inner.project_root().to_path_buf(),
                        line: 1,
                        col: 0,
                        symbol_moniker: moniker,
                        confidence,
                    }
                }
            })
            .collect()
    }

    fn references_of(&self, symbol: &str) -> Vec<IndexTarget> {
        self.inner
            .references_of(symbol)
            .into_iter()
            .map(|def| IndexTarget {
                file: self.inner.project_root().join(&def.relative_path),
                line: def.line.saturating_add(1),
                col: def.character,
                symbol_moniker: def.symbol,
                confidence: Confidence::High,
            })
            .collect()
    }
}
