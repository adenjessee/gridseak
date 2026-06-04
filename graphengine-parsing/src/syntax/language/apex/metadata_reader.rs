//! Salesforce metadata XML readers.
//!
//! Salesforce ships Apex with a small family of XML "meta" files that sit
//! next to every source artifact:
//!
//! | Meta file               | Describes                              |
//! | ----------------------- | -------------------------------------- |
//! | `*.cls-meta.xml`        | An `ApexClass` — apiVersion + status   |
//! | `*.trigger-meta.xml`    | An `ApexTrigger` — apiVersion + status |
//! | `*.object-meta.xml`     | A `CustomObject` — label, sharing, etc.|
//! | `*.field-meta.xml`      | A `CustomField` — Phase 2 scope        |
//!
//! These files are schema-stable (Salesforce Metadata API) and tiny (rarely
//! above a few KB). We use a **streaming** reader rather than a DOM crate
//! so:
//!
//! - Parse cost is linear in file size — matters when scanning repos with
//!   thousands of objects/fields.
//! - We don't allocate intermediate trees for fields we throw away.
//! - Unknown/extended elements (Salesforce adds new tags every release)
//!   are silently ignored — the reader is forward-compatible.
//!
//! Every reader is **tolerant of missing data**: absent elements yield
//! `None` on the struct. Only hard parse errors (malformed XML, unreadable
//! file) surface as `Err`. This matches the detector philosophy in
//! [`super::sfdx_layout`]: downstream always gets a usable result.

use std::io::BufRead;
use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

/// Metadata captured from `*.cls-meta.xml` or `*.trigger-meta.xml`. Both
/// files carry the same two fields that matter in Phase 1; separate
/// wrappers exist downstream to avoid mixing them up, but the shape is
/// identical.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexComponentMeta {
    /// `<apiVersion>` element (e.g. `"59.0"`). Absent for hand-authored
    /// files that rely on org defaults.
    pub api_version: Option<String>,
    /// `<status>` element. In practice one of `"Active"`, `"Inactive"`,
    /// or `"Deleted"`. Treated as an opaque string here.
    pub status: Option<String>,
}

impl ApexComponentMeta {
    /// Common-case convenience: trigger / class is considered deployable
    /// if the `status` element is `"Active"` or absent (absence = active
    /// per Salesforce convention).
    pub fn is_active(&self) -> bool {
        match &self.status {
            None => true,
            Some(s) => s.eq_ignore_ascii_case("Active"),
        }
    }
}

/// Metadata captured from a `*.object-meta.xml`. Everything here lands on
/// the SObject node in the graph: labels for report readability, sharing
/// model as a metadata annotation, and the custom-vs-standard flag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SObjectMeta {
    /// The SObject's API name — derived from the filename, NOT the XML
    /// (Salesforce treats the filename as authoritative). For
    /// `Account.object-meta.xml` this is `"Account"`; for
    /// `My_Custom__c.object-meta.xml` it is `"My_Custom__c"`.
    pub api_name: String,

    /// `<label>` — human-readable singular.
    pub label: Option<String>,
    /// `<pluralLabel>`.
    pub plural_label: Option<String>,
    /// `<deploymentStatus>` — e.g. `"Deployed"`.
    pub deployment_status: Option<String>,
    /// `<sharingModel>` — e.g. `"ReadWrite"`, `"Private"`,
    /// `"ControlledByParent"`.
    pub sharing_model: Option<String>,
    /// `<customSettingsType>` — present only on Custom Settings; used to
    /// distinguish them from regular Custom Objects.
    pub custom_settings_type: Option<String>,

    /// `true` when the API name ends with the Salesforce custom-object
    /// suffix (`__c`, `__mdt`, `__e`, `__b`, `__x`). Derived from the
    /// filename, not from XML content.
    pub is_custom: bool,
    /// `true` when the file represents a Custom Metadata Type (`__mdt`
    /// suffix). Implied subset of `is_custom`.
    pub is_metadata_type: bool,
    /// `true` when the file represents a Platform Event (`__e` suffix).
    pub is_platform_event: bool,
}

impl SObjectMeta {
    /// Convenience: the label to render in reports, falling back to the
    /// API name when no human-facing label was declared.
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(self.api_name.as_str())
    }
}

/// Parse a `*.cls-meta.xml` file. The root element is expected to be
/// `<ApexClass>` but the reader is tolerant — any root with the two known
/// children in scope will parse. Returns an [`ApexComponentMeta`] on any
/// successful XML read; malformed XML / I/O failures bubble up.
pub fn read_class_meta(path: &Path) -> Result<ApexComponentMeta> {
    read_component_meta(path).with_context(|| format!("reading {}", path.display()))
}

/// Parse a `*.trigger-meta.xml` file. Same shape and tolerance as
/// [`read_class_meta`]; separate function exists so callers can't mix up
/// class and trigger metadata by accident at call sites.
pub fn read_trigger_meta(path: &Path) -> Result<ApexComponentMeta> {
    read_component_meta(path).with_context(|| format!("reading {}", path.display()))
}

/// Parse a `*.object-meta.xml` file. The API name and custom-flag are
/// derived from the filename per Salesforce convention.
pub fn read_object_meta(path: &Path) -> Result<SObjectMeta> {
    let api_name = derive_object_api_name(path).with_context(|| {
        format!(
            "cannot derive SObject API name from filename: {}",
            path.display()
        )
    })?;

    let mut meta = SObjectMeta {
        api_name: api_name.clone(),
        is_custom: is_custom_api_name(&api_name),
        is_metadata_type: api_name.ends_with("__mdt"),
        is_platform_event: api_name.ends_with("__e"),
        ..Default::default()
    };

    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut reader = Reader::from_str(&raw);
    configure_reader(&mut reader);

    // Depth-1 text extraction: we only care about children of the root
    // element, not their descendants (none of our target fields nest).
    let mut depth: i32 = 0;
    let mut current_leaf: Option<LeafKind> = None;
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .with_context(|| format!("parsing {}", path.display()))?
        {
            Event::Start(e) => {
                depth += 1;
                if depth == 2 {
                    current_leaf = LeafKind::match_object_tag(local_name(&e));
                }
            }
            Event::End(_) => {
                depth -= 1;
                current_leaf = None;
            }
            Event::Text(t) if current_leaf.is_some() => {
                let text = t
                    .unescape()
                    .with_context(|| format!("unescaping text in {}", path.display()))?
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }
                match current_leaf {
                    Some(LeafKind::Label) => meta.label = Some(text),
                    Some(LeafKind::PluralLabel) => meta.plural_label = Some(text),
                    Some(LeafKind::DeploymentStatus) => meta.deployment_status = Some(text),
                    Some(LeafKind::SharingModel) => meta.sharing_model = Some(text),
                    Some(LeafKind::CustomSettingsType) => meta.custom_settings_type = Some(text),
                    None | Some(LeafKind::ApiVersion) | Some(LeafKind::Status) => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(meta)
}

// -----------------------------------------------------------------------------
// Shared component-meta parser (class + trigger)
// -----------------------------------------------------------------------------

fn read_component_meta(path: &Path) -> Result<ApexComponentMeta> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut reader = Reader::from_str(&raw);
    configure_reader(&mut reader);
    parse_component_meta_from(&mut reader)
}

/// Low-level parser so tests can drive the reader from an in-memory
/// string without touching the filesystem.
fn parse_component_meta_from<B: BufRead>(reader: &mut Reader<B>) -> Result<ApexComponentMeta> {
    let mut meta = ApexComponentMeta::default();
    let mut depth: i32 = 0;
    let mut current_leaf: Option<LeafKind> = None;
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .context("parsing ApexClass/ApexTrigger metadata XML")?
        {
            Event::Start(e) => {
                depth += 1;
                if depth == 2 {
                    current_leaf = LeafKind::match_component_tag(local_name(&e));
                }
            }
            Event::End(_) => {
                depth -= 1;
                current_leaf = None;
            }
            Event::Text(t) if current_leaf.is_some() => {
                let text = t
                    .unescape()
                    .context("unescaping text in Apex component metadata")?
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }
                match current_leaf {
                    Some(LeafKind::ApiVersion) => meta.api_version = Some(text),
                    Some(LeafKind::Status) => meta.status = Some(text),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(meta)
}

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

/// Tags we care about, across both component-meta and object-meta schemas.
/// Keeping them as a single enum avoids string comparisons inside the hot
/// event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeafKind {
    ApiVersion,
    Status,
    Label,
    PluralLabel,
    DeploymentStatus,
    SharingModel,
    CustomSettingsType,
}

impl LeafKind {
    fn match_component_tag(name: &str) -> Option<Self> {
        match name {
            "apiVersion" => Some(LeafKind::ApiVersion),
            "status" => Some(LeafKind::Status),
            _ => None,
        }
    }

    fn match_object_tag(name: &str) -> Option<Self> {
        match name {
            "label" => Some(LeafKind::Label),
            "pluralLabel" => Some(LeafKind::PluralLabel),
            "deploymentStatus" => Some(LeafKind::DeploymentStatus),
            "sharingModel" => Some(LeafKind::SharingModel),
            "customSettingsType" => Some(LeafKind::CustomSettingsType),
            _ => None,
        }
    }
}

fn configure_reader<B: BufRead>(reader: &mut Reader<B>) {
    let cfg = reader.config_mut();
    cfg.trim_text(true);
    cfg.expand_empty_elements = false;
    // Salesforce CLI + MDAPI ship well-formed XML. We keep end-name
    // checking on so that truncated / corrupt metadata surfaces as a
    // hard error rather than silently producing garbage. Schema
    // tolerance (unknown elements) is handled by our leaf-matching
    // enum, not by loosening well-formedness checks.
    cfg.check_end_names = true;
}

/// Extract the local name of a BytesStart tag, stripping any `ns:` prefix.
/// The Salesforce meta XML uses a default namespace (no prefix on element
/// names), so this is almost always a no-op — but we do the strip for
/// robustness against MDAPI exports that occasionally include a prefix.
fn local_name<'a>(e: &'a quick_xml::events::BytesStart<'a>) -> &'a str {
    let bytes = e.name().0;
    let s = std::str::from_utf8(bytes).unwrap_or("");
    match s.rfind(':') {
        Some(idx) => &s[idx + 1..],
        None => s,
    }
}

/// Derive an SObject API name from an `*.object-meta.xml` path.
///
/// Salesforce's contract: the file's name, minus the `.object-meta.xml`
/// suffix, IS the API name. We never read this value from the XML body —
/// the filename is authoritative. Returns `None` when the path doesn't
/// end in the expected suffix.
fn derive_object_api_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    const SUFFIX: &str = ".object-meta.xml";
    if name.len() <= SUFFIX.len() {
        return None;
    }
    // Case-insensitive suffix match so macOS case-preserving filesystems
    // don't trip us up (`.Object-Meta.xml` etc.).
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(SUFFIX) {
        return None;
    }
    let cut = name.len() - SUFFIX.len();
    Some(name[..cut].to_string())
}

/// Salesforce custom-object suffixes. `__c` is the common case; others
/// cover metadata types, platform events, external objects, and big
/// objects. Standard SObjects (Account, Contact, Case, User, ...) have
/// no suffix and return `false`.
fn is_custom_api_name(api_name: &str) -> bool {
    api_name.ends_with("__c")
        || api_name.ends_with("__mdt")
        || api_name.ends_with("__e")
        || api_name.ends_with("__b")
        || api_name.ends_with("__x")
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CLASS_META_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ApexClass xmlns="http://soap.sforce.com/2006/04/metadata">
    <apiVersion>59.0</apiVersion>
    <status>Active</status>
</ApexClass>
"#;

    const TRIGGER_META_INACTIVE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ApexTrigger xmlns="http://soap.sforce.com/2006/04/metadata">
    <apiVersion>55.0</apiVersion>
    <status>Inactive</status>
</ApexTrigger>
"#;

    const TRIGGER_META_MISSING_STATUS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ApexTrigger xmlns="http://soap.sforce.com/2006/04/metadata">
    <apiVersion>58.0</apiVersion>
</ApexTrigger>
"#;

    const OBJECT_META_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CustomObject xmlns="http://soap.sforce.com/2006/04/metadata">
    <label>My Custom Object</label>
    <pluralLabel>My Custom Objects</pluralLabel>
    <deploymentStatus>Deployed</deploymentStatus>
    <sharingModel>ReadWrite</sharingModel>
    <nameField>
        <label>Name</label>
        <type>Text</type>
    </nameField>
</CustomObject>
"#;

    #[test]
    fn reads_class_meta_active() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Foo.cls-meta.xml");
        fs::write(&path, CLASS_META_XML).unwrap();

        let meta = read_class_meta(&path).unwrap();
        assert_eq!(meta.api_version.as_deref(), Some("59.0"));
        assert_eq!(meta.status.as_deref(), Some("Active"));
        assert!(meta.is_active());
    }

    #[test]
    fn reads_trigger_meta_inactive_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("OnContact.trigger-meta.xml");
        fs::write(&path, TRIGGER_META_INACTIVE).unwrap();

        let meta = read_trigger_meta(&path).unwrap();
        assert_eq!(meta.api_version.as_deref(), Some("55.0"));
        assert_eq!(meta.status.as_deref(), Some("Inactive"));
        assert!(!meta.is_active());
    }

    #[test]
    fn missing_status_is_treated_as_active_per_salesforce_convention() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("OnAccount.trigger-meta.xml");
        fs::write(&path, TRIGGER_META_MISSING_STATUS).unwrap();

        let meta = read_trigger_meta(&path).unwrap();
        assert!(meta.status.is_none());
        assert!(
            meta.is_active(),
            "absent <status> element must default to active"
        );
    }

    #[test]
    fn reads_object_meta_standard_sobject() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Account.object-meta.xml");
        fs::write(&path, OBJECT_META_XML).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.api_name, "Account");
        assert_eq!(meta.label.as_deref(), Some("My Custom Object"));
        assert_eq!(meta.plural_label.as_deref(), Some("My Custom Objects"));
        assert_eq!(meta.deployment_status.as_deref(), Some("Deployed"));
        assert_eq!(meta.sharing_model.as_deref(), Some("ReadWrite"));
        assert!(
            !meta.is_custom,
            "Account has no __c suffix — must be standard"
        );
        assert!(!meta.is_metadata_type);
        assert!(!meta.is_platform_event);
    }

    #[test]
    fn reads_object_meta_custom_sobject() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("My_Thing__c.object-meta.xml");
        fs::write(&path, OBJECT_META_XML).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.api_name, "My_Thing__c");
        assert!(meta.is_custom);
        assert!(!meta.is_metadata_type);
        assert!(!meta.is_platform_event);
    }

    #[test]
    fn reads_object_meta_metadata_type() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("My_Config__mdt.object-meta.xml");
        fs::write(&path, OBJECT_META_XML).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.api_name, "My_Config__mdt");
        assert!(meta.is_custom);
        assert!(meta.is_metadata_type);
        assert!(!meta.is_platform_event);
    }

    #[test]
    fn reads_object_meta_platform_event() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Order_Placed__e.object-meta.xml");
        fs::write(&path, OBJECT_META_XML).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.api_name, "Order_Placed__e");
        assert!(meta.is_custom);
        assert!(meta.is_platform_event);
        assert!(!meta.is_metadata_type);
    }

    #[test]
    fn display_name_falls_back_to_api_name_when_label_missing() {
        let minimal = r#"<?xml version="1.0"?><CustomObject/>"#;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("UnlabeledObj.object-meta.xml");
        fs::write(&path, minimal).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.display_name(), "UnlabeledObj");
    }

    #[test]
    fn tolerates_unknown_extension_elements() {
        // Salesforce adds new tags every release. We must survive seeing
        // one we've never indexed without producing garbage.
        let with_extra = r#"<?xml version="1.0"?>
<CustomObject xmlns="http://soap.sforce.com/2006/04/metadata">
    <label>Hello</label>
    <brandNewFeatureSalesforceAdded>Opaque</brandNewFeatureSalesforceAdded>
    <sharingModel>Private</sharingModel>
</CustomObject>"#;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Thing.object-meta.xml");
        fs::write(&path, with_extra).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.label.as_deref(), Some("Hello"));
        assert_eq!(meta.sharing_model.as_deref(), Some("Private"));
    }

    #[test]
    fn mismatched_close_tag_returns_clear_error() {
        // A deliberately corrupted close tag — the sort of thing a
        // truncated-then-hand-edited metadata file would carry. With
        // `check_end_names = true` quick-xml MUST reject this rather
        // than silently giving us partial data that looks clean.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Broken.cls-meta.xml");
        fs::write(
            &path,
            "<ApexClass><apiVersion>59.0</apiVersion><status>Active</WRONG></ApexClass>",
        )
        .unwrap();

        let err = read_class_meta(&path).expect_err("mismatched close tag must surface as error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Broken.cls-meta.xml")
                || msg.to_ascii_lowercase().contains("xml")
                || msg.to_ascii_lowercase().contains("end"),
            "error must be contextualized with the failing path or quick-xml message, got: {msg}"
        );
    }

    #[test]
    fn nested_elements_do_not_leak_into_outer_leaves() {
        // Ensures the depth check prevents `<label>` inside `<nameField>`
        // from overwriting the object-level `<label>`.
        let with_nested_label = r#"<?xml version="1.0"?>
<CustomObject xmlns="http://soap.sforce.com/2006/04/metadata">
    <label>Outer Label</label>
    <nameField>
        <label>Inner Label Should Be Ignored</label>
        <type>Text</type>
    </nameField>
</CustomObject>"#;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Nested__c.object-meta.xml");
        fs::write(&path, with_nested_label).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(
            meta.label.as_deref(),
            Some("Outer Label"),
            "depth-1 restriction must prevent nested <label> leak"
        );
    }

    #[test]
    fn custom_api_name_suffixes_all_detected() {
        assert!(is_custom_api_name("Foo__c"));
        assert!(is_custom_api_name("Foo__mdt"));
        assert!(is_custom_api_name("Foo__e"));
        assert!(is_custom_api_name("Foo__b"));
        assert!(is_custom_api_name("Foo__x"));
        assert!(!is_custom_api_name("Account"));
        assert!(!is_custom_api_name("Case"));
    }

    #[test]
    fn derive_api_name_rejects_non_object_meta_files() {
        assert!(derive_object_api_name(Path::new("Foo.cls-meta.xml")).is_none());
        assert!(derive_object_api_name(Path::new("Foo.trigger-meta.xml")).is_none());
        assert!(derive_object_api_name(Path::new("Foo.cls")).is_none());
        assert!(derive_object_api_name(Path::new(".object-meta.xml")).is_none());

        assert_eq!(
            derive_object_api_name(Path::new("Account.object-meta.xml")).as_deref(),
            Some("Account")
        );
        assert_eq!(
            derive_object_api_name(Path::new("/abs/path/My__c.object-meta.xml")).as_deref(),
            Some("My__c")
        );
    }

    #[test]
    fn self_closing_elements_do_not_crash_the_parser() {
        let self_closing = r#"<?xml version="1.0"?>
<CustomObject xmlns="http://soap.sforce.com/2006/04/metadata">
    <label>Hello</label>
    <enableReports/>
    <sharingModel>Private</sharingModel>
</CustomObject>"#;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("SelfClose__c.object-meta.xml");
        fs::write(&path, self_closing).unwrap();

        let meta = read_object_meta(&path).unwrap();
        assert_eq!(meta.label.as_deref(), Some("Hello"));
        assert_eq!(meta.sharing_model.as_deref(), Some("Private"));
    }
}
