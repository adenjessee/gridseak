//! Apex / Salesforce-specific parsing helpers.
//!
//! Houses logic that only makes sense for Salesforce codebases: SFDX project
//! layout discovery, metadata XML readers, standard-SObject registries,
//! heuristic fallback resolvers, and trigger-framework detectors. Everything
//! in this module is additive — the generic Tree-sitter / LSP pipeline
//! remains unaware of these details and is fed through the standard
//! [`LanguageConfig`](crate::infrastructure::config::LanguageConfig) path.
//!
//! See `docs/workstreams/apex/INTEGRATION.md` for the architectural rationale.

pub mod arg_type_inferrer;
pub mod arg_type_narrower;
pub mod class_registry;
pub mod class_symbol_codec;
pub mod class_symbols_extractor;
pub mod constructor_resolver;
pub mod containment_walker;
pub mod coverage;
pub mod downward_dispatch;
pub mod entry_points;
pub mod extractor;
pub mod field_body_synth;
pub mod field_type_resolver;
pub mod fqn;
pub mod framework_entry_point_propagation;
pub mod framework_entry_point_stage;
pub mod heuristic_resolver;
pub mod inner_class_resolver;
pub mod local_var_extractor;
pub mod lsp_session;
pub mod managed_package_registry;
pub mod managed_packages;
pub mod metadata_reader;
pub mod resolver_dispatch;
pub mod sfdx_layout;
pub mod sharing;
pub mod signature_matcher;
pub mod soql_sosl;
pub mod trigger_framework;
pub mod trigger_metadata;
pub mod type_hierarchy;
pub mod vf_extraction_stage;
pub mod vf_page_reader;
pub mod vf_page_resolver;

pub use entry_points::{classify as classify_entry_point, is_entry_point, EntryPointKind};
pub use extractor::ApexExtractor;

pub use class_registry::{
    extract_managed_namespace, ApexClassRegistry, ApexTypeCollision, ApexTypeEntry, ApexTypeKind,
    ApexTypeSource,
};
// TR-A Commit 1a: `class_symbols` relocated to `crate::domain::apex::class_symbols`
// so `CallSite.arg_types: Vec<ApexTypeRef>` in `application::ports` obeys the
// application → domain dependency arrow. These re-exports preserve the existing
// `syntax::language::apex::{ApexClassSymbols, ApexTypeRef, …}` import surface for
// in-module callers without re-introducing the layer inversion.
pub use crate::domain::apex::class_symbols::{
    Access as ApexMemberAccess, ApexClassSymbols, ApexConstructor, ApexField, ApexMethod,
    ApexParameter, ApexSymbolsMap, ApexTypeRef, CollectionKind, APEX_CLASS_SYMBOLS_SCHEMA_VERSION,
};
pub use containment_walker::{resolve_dotted_path, walk_methods, DottedPathProvider};
pub use heuristic_resolver::ApexHeuristicResolver;
pub use lsp_session::build_apex_session_options;
pub use managed_package_registry::{
    lookup as lookup_known_managed_package, Category as ManagedPackageCategory, KnownPackage,
    Vendor as ManagedPackageVendor,
};
pub use managed_packages::{
    extract as extract_managed_references,
    synthesize_import_edge as synthesize_managed_package_import_edge,
    synthesize_module_node as synthesize_managed_package_module_node,
    unique_namespaces as unique_managed_namespaces, ManagedReferenceSite,
    ReferenceKind as ManagedReferenceKind, VIRTUAL_MANAGED_MODULE_FILE_SENTINEL,
    VIRTUAL_MANAGED_MODULE_FQN_PREFIX,
};
pub use metadata_reader::{
    read_class_meta, read_object_meta, read_trigger_meta, ApexComponentMeta, SObjectMeta,
};
pub use resolver_dispatch::{
    ApexResolverDispatcher, ResolverOverride, ResolverTier, ENV_APEX_RESOLVER,
};
pub use sfdx_layout::{detect as detect_sfdx_layout, ClassifiedFiles, LayoutKind, SfdxLayout};
pub use sharing::{classify as classify_sharing_model, SharingModel};
pub use soql_sosl::{extract as extract_queries, QueryKind, QueryReference};
pub use trigger_framework::{
    detect as detect_trigger_framework, DetectionResult as TriggerFrameworkDetection,
    TriggerFramework, TriggerFrameworkFacts,
};
