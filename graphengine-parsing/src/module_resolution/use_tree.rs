use std::collections::HashMap;

use crate::application::ports::{ImportSpec, PathRoot, SyntaxResults};

use super::types::{BindingOrigin, ImportRecord};

#[derive(Debug, Default, Clone)]
pub struct UseTree {
    pub(crate) bindings_by_file: HashMap<String, Vec<ImportBinding>>,
}

#[derive(Debug, Clone)]
pub struct ImportBinding {
    pub record: ImportRecord,
    pub origin: BindingOrigin,
}

impl UseTree {
    pub fn build(syntax: &SyntaxResults) -> Self {
        let mut tree = UseTree::default();

        for spec in &syntax.import_specs {
            let binding = build_binding(spec);
            tree.bindings_by_file
                .entry(spec.source_file.clone())
                .or_default()
                .push(binding);
        }

        tree
    }

    pub fn bindings_for_file(&self, file: &str) -> &[ImportBinding] {
        self.bindings_by_file
            .get(file)
            .map(|vec| vec.as_slice())
            .unwrap_or(&[])
    }

    pub fn files(&self) -> impl Iterator<Item = &String> {
        self.bindings_by_file.keys()
    }
}

fn build_binding(spec: &ImportSpec) -> ImportBinding {
    let binding_name = spec
        .alias
        .clone()
        .unwrap_or_else(|| default_binding_name(spec));

    let origin = if spec.is_glob {
        BindingOrigin::Glob
    } else if spec.alias.is_some() {
        BindingOrigin::Alias
    } else {
        match spec.path.root {
            PathRoot::SelfPath => BindingOrigin::SelfPath,
            PathRoot::Super(depth) => BindingOrigin::SuperPath(depth),
            PathRoot::Crate => BindingOrigin::CratePath,
            PathRoot::Absolute => BindingOrigin::CratePath,
            PathRoot::ExternalCrate(_) => BindingOrigin::ExternalCrate,
            PathRoot::Unqualified => BindingOrigin::ExternalCrate,
        }
    };

    ImportBinding {
        record: ImportRecord {
            spec: spec.clone(),
            binding: binding_name,
        },
        origin,
    }
}

fn default_binding_name(spec: &ImportSpec) -> String {
    if spec.is_glob {
        return "*".to_string();
    }

    if let Some(segment) = spec.path.segments.last() {
        return segment.clone();
    }

    match spec.path.root {
        PathRoot::SelfPath => "self".to_string(),
        PathRoot::Super(_) => "super".to_string(),
        PathRoot::Crate => "crate".to_string(),
        PathRoot::Absolute => "crate".to_string(),
        PathRoot::ExternalCrate(ref name) => name.clone(),
        PathRoot::Unqualified => "*".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{
        ImportKind, ImportPath, ImportSpec, ImportVisibility, SyntaxResults,
    };
    use crate::domain::Range;

    #[test]
    fn builds_bindings_with_alias_and_glob() {
        let mut syntax = SyntaxResults::new();
        let file = "src/lib.rs".to_string();

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(1, 0, 1, 20, file.clone()),
            path: ImportPath::new(
                PathRoot::Crate,
                vec!["alpha".into(), "beta".into(), "target".into()],
            ),
            alias: Some("alias_target".into()),
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: file.clone(),
        });

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(2, 0, 2, 15, file.clone()),
            path: ImportPath::new(PathRoot::Crate, vec!["alpha".into(), "beta".into()]),
            alias: None,
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: true,
            source_file: file.clone(),
        });

        syntax.add_import_spec(ImportSpec {
            range: Range::with_file(3, 0, 3, 18, file.clone()),
            path: ImportPath::new(PathRoot::SelfPath, vec!["local".into(), "item".into()]),
            alias: None,
            visibility: ImportVisibility::Private,
            kind: ImportKind::Use,
            is_glob: false,
            source_file: file.clone(),
        });

        let tree = UseTree::build(&syntax);
        let bindings = tree.bindings_for_file(&file);
        assert_eq!(bindings.len(), 3);

        let alias = bindings
            .iter()
            .find(|binding| binding.record.binding == "alias_target")
            .expect("alias binding");
        assert!(matches!(alias.origin, BindingOrigin::Alias));

        let glob = bindings
            .iter()
            .find(|binding| binding.record.binding == "*")
            .expect("glob binding");
        assert!(matches!(glob.origin, BindingOrigin::Glob));

        let self_binding = bindings
            .iter()
            .find(|binding| binding.record.binding == "item")
            .expect("self binding");
        assert!(matches!(self_binding.origin, BindingOrigin::SelfPath));
    }
}
