//! Downward polymorphic dispatch fan-out (TR-A.6).
//!
//! The upward parts of the heuristic resolver — [`super::field_type_resolver`]
//! for typed-field dispatch and [`super::containment_walker`] for
//! `parent_class` chain lookup — bind a call site to the method
//! declaration that Apex's *static* name-resolution rules point at. For
//! interface calls that means the interface's own declaration; for
//! abstract / virtual method calls via a parent-typed receiver, that
//! means the parent's declaration.
//!
//! Polymorphism, however, means the *runtime* target of those calls is
//! a concrete subclass / implementer override. The heuristic resolver
//! can't know which one without running the program, so it emits an
//! extra edge to every override the registry knows about. Those edges
//! are Low-confidence (they represent "this is a possible runtime
//! target", not "this is THE target") — users downstream see them as a
//! resolved fan-out rather than `no_callers` dead-code.
//!
//! # What this module does
//!
//! Given a resolved callee [`Node`] (the method the upward resolver
//! bound to), return the set of descendant [`Node`]s that match the
//! same method name + parameter signature under the Apex
//! name-resolution rules:
//!
//! * **Interface call targets.** When the callee is a method on an
//!   interface, every direct implementer + their descendants is a
//!   legal runtime target. Returned as a Low-confidence fan-out.
//! * **Virtual or abstract class-method targets.** When the callee's
//!   method is declared `virtual` or `abstract`, every subclass's
//!   override with the same signature is a legal runtime target.
//!
//! Concrete (non-virtual, non-abstract) class methods are terminal —
//! Apex disallows overriding them — so the module returns an empty
//! slice for those.
//!
//! # What this module deliberately does NOT do
//!
//! * **Emit edges**. It returns `Vec<&Node>`; the caller wraps them in
//!   `Edge::call(..)` with its own provenance. Keeps this module pure.
//! * **Filter self-loops.** The caller already handles the caller →
//!   callee self-loop drop via the `Edge` constructor's validation;
//!   this module would have to know the caller's id to pre-filter,
//!   and threading that through for a one-liner is not worth the
//!   signature churn.
//! * **Rank by signature**. The upward resolver already ran the full
//!   signature-matcher ladder to pick the bound method. Descendants
//!   carry the same signature by definition (an override with a
//!   different signature is not an override, it is a new method); the
//!   only filter here is string equality on the canonical FQN
//!   signature.

use std::collections::HashMap;

use crate::domain::apex::class_symbols::{ApexParameter, ApexTypeRef, CollectionKind};
use crate::domain::Node;

use super::class_registry::{ApexClassRegistry, ApexTypeKind};
use super::type_hierarchy::TypeHierarchy;

/// Return every descendant [`Node`] that is a runtime target for a
/// call resolved to `callee`. See the module-level docs for the set
/// definition.
///
/// Returns an empty vector when no fan-out applies — the callee is
/// already terminal, or the registry has no descendants of the
/// callee's owning class.
pub(super) fn enumerate_overrides<'a>(
    callee: &'a Node,
    registry: &ApexClassRegistry,
    hierarchy: &TypeHierarchy,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
) -> Vec<&'a Node> {
    let Some((owner_api, method_name, param_sig)) = parse_method_fqn(&callee.fqn) else {
        return Vec::new();
    };

    let owner_entry = match registry.lookup(&owner_api) {
        Some(e) => e,
        None => return Vec::new(),
    };

    let descendants = match owner_entry.kind {
        ApexTypeKind::Interface => hierarchy.implementers_rollup(&owner_api),
        ApexTypeKind::Class => {
            if !callee_method_is_polymorphic(owner_entry.symbols.as_ref(), &method_name, &param_sig)
            {
                return Vec::new();
            }
            hierarchy.descendants_of(&owner_api)
        }
        // Enums, triggers, and external types have no runtime
        // subtyping semantics that produce new method implementations.
        _ => return Vec::new(),
    };

    if descendants.is_empty() {
        return Vec::new();
    }

    collect_descendant_nodes(
        &descendants,
        &method_name,
        &param_sig,
        callee,
        functions_by_name_lower,
    )
}

/// Split a method Function FQN into `(owner_api, method_name,
/// canonical_param_sig)`. Shape of an Apex method FQN:
/// `<workspace>::<outer>::<class_api>::<method>(<sig>)`. The
/// `<class_api>` may itself be dotted (`Outer.Inner`).
fn parse_method_fqn(fqn: &str) -> Option<(String, String, String)> {
    let open = fqn.rfind('(')?;
    let close = fqn[open..].find(')')?;
    let sig = fqn[open + 1..open + close].to_string();

    let before_open = &fqn[..open];
    let (class_part, method_short) = before_open.rsplit_once("::")?;
    // Strip the workspace / outer prefix from the class part. The class
    // api-name is the tail after the final `::`.
    let owner_api = class_part
        .rsplit_once("::")
        .map(|(_, tail)| tail)
        .unwrap_or(class_part);

    if owner_api.is_empty() || method_short.is_empty() {
        return None;
    }
    Some((owner_api.to_string(), method_short.to_string(), sig))
}

/// `true` when the callee method on `owner_syms` is declared
/// `virtual` or `abstract`. Falls back to `true` when the symbols
/// aren't attached — we'd rather fan out conservatively than miss a
/// legitimate override; a concrete method with no descendants is a
/// no-op anyway.
fn callee_method_is_polymorphic(
    owner_syms: Option<&crate::domain::apex::class_symbols::ApexClassSymbols>,
    method_name: &str,
    param_sig: &str,
) -> bool {
    let Some(syms) = owner_syms else {
        return true;
    };
    for m in syms.methods_named(method_name) {
        if canonical_param_sig_list(&m.parameters).eq_ignore_ascii_case(param_sig)
            && (m.is_virtual || m.is_abstract)
        {
            return true;
        }
    }
    false
}

/// For every descendant api-name, find the Function node whose short
/// method name matches `method_name` AND whose enclosing class api
/// matches the descendant AND whose canonical parameter signature
/// matches `param_sig`. Excludes the original callee node so we do
/// not emit a redundant edge to the very method we resolved to.
fn collect_descendant_nodes<'a>(
    descendants: &[String],
    method_name: &str,
    param_sig: &str,
    callee: &'a Node,
    functions_by_name_lower: &HashMap<String, Vec<&'a Node>>,
) -> Vec<&'a Node> {
    let key = method_name.to_ascii_lowercase();
    let Some(candidates) = functions_by_name_lower.get(&key) else {
        return Vec::new();
    };
    let sig_lower = param_sig.to_ascii_lowercase();

    let mut out: Vec<&Node> = Vec::new();
    let mut seen_ids: Vec<&str> = Vec::new();
    for desc_api in descendants {
        for cand in candidates {
            if cand.id == callee.id {
                continue;
            }
            if !enclosing_class_api_eq(&cand.fqn, desc_api) {
                continue;
            }
            let Some((_, cand_sig)) = fqn_short_and_sig(&cand.fqn) else {
                continue;
            };
            if !cand_sig.eq_ignore_ascii_case(&sig_lower) {
                continue;
            }
            if seen_ids.contains(&cand.id.as_str()) {
                continue;
            }
            seen_ids.push(cand.id.as_str());
            out.push(*cand);
        }
    }
    out
}

fn enclosing_class_api_eq(fqn: &str, target_api: &str) -> bool {
    let Some(class_fqn) = enclosing_class_fqn(fqn) else {
        return false;
    };
    let api = class_fqn
        .rsplit_once("::")
        .map(|(_, tail)| tail)
        .unwrap_or(class_fqn.as_str());
    api.eq_ignore_ascii_case(target_api)
}

fn enclosing_class_fqn(fqn: &str) -> Option<String> {
    let without_sig = match fqn.find('(') {
        Some(idx) => &fqn[..idx],
        None => fqn,
    };
    if let Some(pos) = without_sig.rfind("::") {
        let prefix = &without_sig[..pos];
        if !prefix.is_empty() {
            return Some(prefix.to_string());
        }
    }
    if let Some((class_part, _)) = without_sig.rsplit_once('.') {
        if !class_part.is_empty() {
            return Some(class_part.to_string());
        }
    }
    None
}

fn fqn_short_and_sig(fqn: &str) -> Option<(&str, &str)> {
    let open = fqn.rfind('(')?;
    let close = fqn[open..].find(')')?;
    let sig = &fqn[open + 1..open + close];
    let before_open = &fqn[..open];
    let short = before_open.rsplit(['.', ':']).next().unwrap_or(before_open);
    Some((short, sig))
}

fn canonical_param_sig_list(params: &[ApexParameter]) -> String {
    params
        .iter()
        .map(|p| canonical_param_sig(&p.ty))
        .collect::<Vec<_>>()
        .join(",")
}

fn canonical_param_sig(ty: &ApexTypeRef) -> String {
    match ty {
        ApexTypeRef::Primitive { name } => name.clone(),
        ApexTypeRef::Sobject { api_name } | ApexTypeRef::UserDefined { api_name } => {
            api_name.clone()
        }
        ApexTypeRef::Collection { kind, .. } => match kind {
            CollectionKind::List => "List".to_string(),
            CollectionKind::Set => "Set".to_string(),
        },
        ApexTypeRef::Map { .. } => "Map".to_string(),
        ApexTypeRef::Generic { base, .. } => base.clone(),
        ApexTypeRef::Unresolved { raw } => raw.split_whitespace().collect(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apex::class_symbols::{Access, ApexClassSymbols, ApexMethod, ApexParameter};
    use crate::domain::Range;
    use crate::syntax::language::apex::class_registry::ApexTypeKind;
    use std::path::PathBuf;

    fn prim(name: &str) -> ApexTypeRef {
        ApexTypeRef::Primitive {
            name: name.to_string(),
        }
    }

    fn method(
        name: &str,
        param_types: &[ApexTypeRef],
        is_virtual: bool,
        is_abstract: bool,
    ) -> ApexMethod {
        ApexMethod {
            name: name.to_string(),
            parameters: param_types
                .iter()
                .enumerate()
                .map(|(i, ty)| ApexParameter {
                    name: format!("p{i}"),
                    ty: ty.clone(),
                })
                .collect(),
            return_type: None,
            access: Access::Public,
            is_static: false,
            is_virtual,
            is_abstract,
        }
    }

    fn fn_node(fqn: &str) -> Node {
        Node::function(
            fqn.to_string(),
            Range::with_file(1, 0, 10, 0, "test.cls".to_string()),
        )
    }

    fn registry_with(entries: &[(&str, ApexTypeKind, ApexClassSymbols)]) -> ApexClassRegistry {
        let mut r = ApexClassRegistry::with_standard_preload();
        for (api, kind, _) in entries {
            let enclosing = api.rsplit_once('.').map(|(outer, _)| outer.to_string());
            r.insert_user_declared(api, *kind, PathBuf::from(format!("{api}.cls")), enclosing);
        }
        for (api, _, syms) in entries {
            assert!(r.attach_symbols(api, syms.clone()));
        }
        r
    }

    #[test]
    fn parse_method_fqn_splits_owner_method_sig() {
        let got = parse_method_fqn("path::Outer::Outer.Inner::doThing(String,Integer)");
        assert_eq!(
            got,
            Some((
                "Outer.Inner".to_string(),
                "doThing".to_string(),
                "String,Integer".to_string()
            ))
        );
    }

    #[test]
    fn parse_method_fqn_handles_zero_arg_call() {
        let got = parse_method_fqn("path::Foo::Foo::bar()");
        assert_eq!(
            got,
            Some(("Foo".to_string(), "bar".to_string(), "".to_string()))
        );
    }

    #[test]
    fn enumerate_overrides_returns_empty_for_unknown_owner() {
        let r = registry_with(&[]);
        let h = TypeHierarchy::build(&r);
        let callee = fn_node("path::Unknown::Unknown::foo()");
        let fns: HashMap<String, Vec<&Node>> = HashMap::new();
        assert!(enumerate_overrides(&callee, &r, &h, &fns).is_empty());
    }

    #[test]
    fn enumerate_overrides_fans_out_for_interface_call() {
        // Registry: `IFoo` (interface) + `Impl1` / `Impl2` (direct
        // implementers). All carry a matching `run(String)` method.
        let iface_syms = ApexClassSymbols {
            methods: vec![method("run", &[prim("String")], false, false)],
            ..Default::default()
        };
        let impl1_syms = ApexClassSymbols {
            implemented_interfaces: vec!["IFoo".into()],
            methods: vec![method("run", &[prim("String")], false, false)],
            ..Default::default()
        };
        let impl2_syms = ApexClassSymbols {
            implemented_interfaces: vec!["IFoo".into()],
            methods: vec![method("run", &[prim("String")], false, false)],
            ..Default::default()
        };
        let r = registry_with(&[
            ("IFoo", ApexTypeKind::Interface, iface_syms),
            ("Impl1", ApexTypeKind::Class, impl1_syms),
            ("Impl2", ApexTypeKind::Class, impl2_syms),
        ]);
        let h = TypeHierarchy::build(&r);

        let iface_node = fn_node("ws::IFoo::IFoo::run(String)");
        let impl1_node = fn_node("ws::Impl1::Impl1::run(String)");
        let impl2_node = fn_node("ws::Impl2::Impl2::run(String)");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.insert("run".into(), vec![&iface_node, &impl1_node, &impl2_node]);

        let overrides = enumerate_overrides(&iface_node, &r, &h, &fns);
        assert_eq!(overrides.len(), 2);
        assert!(overrides.iter().any(|n| n.id == impl1_node.id));
        assert!(overrides.iter().any(|n| n.id == impl2_node.id));
        // Must exclude the original callee from the fan-out.
        assert!(overrides.iter().all(|n| n.id != iface_node.id));
    }

    #[test]
    fn enumerate_overrides_fans_out_for_virtual_class_method() {
        let parent_syms = ApexClassSymbols {
            methods: vec![method("load", &[prim("Integer")], true, false)],
            ..Default::default()
        };
        let child_syms = ApexClassSymbols {
            parent_class: Some("Parent".into()),
            methods: vec![method("load", &[prim("Integer")], false, false)],
            ..Default::default()
        };
        let r = registry_with(&[
            ("Parent", ApexTypeKind::Class, parent_syms),
            ("Child", ApexTypeKind::Class, child_syms),
        ]);
        let h = TypeHierarchy::build(&r);

        let parent_node = fn_node("ws::Parent::Parent::load(Integer)");
        let child_node = fn_node("ws::Child::Child::load(Integer)");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.insert("load".into(), vec![&parent_node, &child_node]);

        let overrides = enumerate_overrides(&parent_node, &r, &h, &fns);
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].id, child_node.id);
    }

    #[test]
    fn enumerate_overrides_ignores_concrete_terminal_method() {
        // Concrete method on a class that has a subclass which does NOT
        // override it. Apex's override rules would refuse that shape at
        // compile time, but the resolver must also handle cases where
        // the subclass declares an unrelated method with the same name
        // but a different signature — those are not overrides and the
        // fanout must skip them.
        let parent_syms = ApexClassSymbols {
            methods: vec![method("load", &[prim("Integer")], false, false)],
            ..Default::default()
        };
        let child_syms = ApexClassSymbols {
            parent_class: Some("Parent".into()),
            methods: vec![method("load", &[prim("String")], false, false)],
            ..Default::default()
        };
        let r = registry_with(&[
            ("Parent", ApexTypeKind::Class, parent_syms),
            ("Child", ApexTypeKind::Class, child_syms),
        ]);
        let h = TypeHierarchy::build(&r);

        let parent_node = fn_node("ws::Parent::Parent::load(Integer)");
        let child_node = fn_node("ws::Child::Child::load(String)");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.insert("load".into(), vec![&parent_node, &child_node]);

        // Parent's `load(Integer)` is neither virtual nor abstract, so
        // no fanout. Child's `load(String)` also has a different sig
        // anyway.
        let overrides = enumerate_overrides(&parent_node, &r, &h, &fns);
        assert!(overrides.is_empty());
    }

    #[test]
    fn enumerate_overrides_signature_must_match() {
        // Subclass has a same-named method but different signature —
        // not an override. Must be excluded from the fanout.
        let iface_syms = ApexClassSymbols {
            methods: vec![method("run", &[prim("String")], false, false)],
            ..Default::default()
        };
        let impl_syms = ApexClassSymbols {
            implemented_interfaces: vec!["IFoo".into()],
            methods: vec![method("run", &[prim("Integer")], false, false)],
            ..Default::default()
        };
        let r = registry_with(&[
            ("IFoo", ApexTypeKind::Interface, iface_syms),
            ("Impl", ApexTypeKind::Class, impl_syms),
        ]);
        let h = TypeHierarchy::build(&r);

        let iface_node = fn_node("ws::IFoo::IFoo::run(String)");
        let impl_node = fn_node("ws::Impl::Impl::run(Integer)");
        let mut fns: HashMap<String, Vec<&Node>> = HashMap::new();
        fns.insert("run".into(), vec![&iface_node, &impl_node]);

        let overrides = enumerate_overrides(&iface_node, &r, &h, &fns);
        assert!(overrides.is_empty());
    }
}
