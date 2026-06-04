use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TEMPLATE_QUERY_CONTRACT_VERSION_V1: &str = "template_query_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    FilteredDump,
    Traversal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateQueryPayload {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub metadata: TemplateQueryMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateQueryMetadata {
    pub contract_version: String,
    pub query_mode: QueryMode,
    pub schema_version: String,
    pub externals: ExternalsMetadata,
    pub capabilities: Capabilities,

    #[serde(default)]
    pub recommended_view_roots: Value,

    #[serde(default)]
    pub warnings: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalsMetadata {
    pub show_externals: bool,
    /// Contract v1 standardizes on: "closed_subgraph" when externals are off, otherwise "stubs".
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub traversal_supported: bool,
    pub externals_modes_supported: Vec<String>,
    pub template_fields: TemplateFields,
    pub operators: Vec<String>,
    pub limits: Limits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateFields {
    pub node: Vec<String>,
    pub edge: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limits {
    pub max_depth: i32,
    pub max_nodes: usize,
    pub max_edges: usize,
}

pub fn default_capabilities(traversal_supported: bool) -> Capabilities {
    Capabilities {
        traversal_supported,
        externals_modes_supported: vec!["closed_subgraph".to_string(), "stubs".to_string()],
        template_fields: TemplateFields {
            node: vec![
                "node_type".to_string(),
                "properties.role".to_string(),
                "properties.is_vendor".to_string(),
                "properties.is_build_output".to_string(),
                "properties.is_generated".to_string(),
                "properties.is_test".to_string(),
                "properties.path_repo_rel".to_string(),
            ],
            edge: vec!["rel".to_string()],
        },
        operators: vec![
            "==".to_string(),
            "in".to_string(),
            "starts_with".to_string(),
            "and".to_string(),
        ],
        limits: Limits {
            max_depth: 25,
            max_nodes: 50_000,
            max_edges: 200_000,
        },
    }
}
