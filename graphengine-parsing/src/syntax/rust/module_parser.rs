//! Rust module parsing utilities
//!
//! Parses Rust module declarations using the syn crate and converts them
//! to our domain ModDecl types.

use crate::application::ports::{ModDecl, ModKind};
use crate::domain::Range;
use crate::syntax::utils::path_utils::resolve_external_module_path;
use syn::ItemMod;
use tracing::warn;

/// Build a module declaration from a syn ItemMod
///
/// # Arguments
/// * `item_mod` - The parsed module item
/// * `file_path` - The file path where the module is declared
/// * `range` - The range of the module declaration
///
/// # Returns
/// A ModDecl representing the module
pub fn build_mod_decl(item_mod: &ItemMod, file_path: &str, range: Range) -> ModDecl {
    let name = item_mod.ident.to_string();
    let kind = if item_mod.content.is_some() {
        ModKind::Inline
    } else {
        ModKind::External
    };

    let resolved_file = match kind {
        ModKind::Inline => Some(file_path.to_string()),
        ModKind::External => resolve_external_module_path(file_path, &name),
    };

    ModDecl {
        name,
        source_file: file_path.to_string(),
        range,
        kind,
        resolved_file,
    }
}

/// Parse a module declaration from text
///
/// Attempts to parse module declaration text using syn and build a ModDecl.
///
/// # Arguments
/// * `text` - The text of the module declaration
/// * `file_path` - The file path
/// * `range` - The range of the module declaration
///
/// # Returns
/// `Some(ModDecl)` if parsing succeeds, `None` otherwise
pub fn parse_mod_decl(text: &str, file_path: &str, range: Range) -> Option<ModDecl> {
    match syn::parse_str::<ItemMod>(text) {
        Ok(item_mod) => Some(build_mod_decl(&item_mod, file_path, range)),
        Err(err) => {
            warn!(
                "Failed to parse module declaration '{}' in {}: {}",
                text.trim(),
                file_path,
                err
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_build_mod_decl_inline() {
        // This would need actual syn::ItemMod construction in practice
    }
}
