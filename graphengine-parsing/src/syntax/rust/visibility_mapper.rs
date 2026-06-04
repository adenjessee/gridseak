//! Visibility mapping utilities for Rust
//!
//! Maps syn crate Visibility types to our domain ImportVisibility types.

use crate::application::ports::ImportVisibility;
use syn::Visibility;

/// Extract path segments from a syn::Path
pub fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

/// Map syn Visibility to ImportVisibility
pub fn map_visibility(visibility: &Visibility) -> ImportVisibility {
    match visibility {
        Visibility::Inherited => ImportVisibility::Private,
        Visibility::Public(_) => ImportVisibility::Pub,
        Visibility::Restricted(restricted) => {
            let segments = path_segments(&restricted.path);
            if restricted.in_token.is_some() {
                ImportVisibility::PubIn(segments.join("::"))
            } else if segments.iter().all(|segment| segment == "super") {
                let depth = segments.len();
                ImportVisibility::PubSuper(depth as u8)
            } else if segments.len() == 1 && segments.first().map(|s| s == "crate").unwrap_or(false)
            {
                ImportVisibility::PubCrate
            } else {
                ImportVisibility::PubIn(segments.join("::"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_path_segments() {
        // This would need actual syn::Path construction in practice
    }
}
