//! LSP Protocol implementation
//!
//! Implements the Language Server Protocol (LSP) communication layer using JSON-RPC.
//! Handles message serialization, deserialization, and protocol compliance.

use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// LSP JSON-RPC message types
#[derive(Debug, Clone)]
pub enum LspMessage {
    /// Request message
    Request {
        id: LspId,
        method: String,
        params: Option<Value>,
    },
    /// Response message
    Response {
        id: LspId,
        result: Option<Value>,
        error: Option<LspError>,
    },
    /// Notification message
    Notification {
        method: String,
        params: Option<Value>,
    },
}

/// LSP message ID (can be string, number, or null)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LspId {
    String(String),
    Number(i64),
    Null,
}

/// LSP error structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// LSP initialization parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub process_id: Option<u32>,
    pub client_info: Option<ClientInfo>,
    pub locale: Option<String>,
    pub root_path: Option<String>,
    pub root_uri: Option<String>,
    pub initialization_options: Option<Value>,
    pub capabilities: ClientCapabilities,
    pub trace: Option<String>,
    pub workspace_folders: Option<Vec<WorkspaceFolder>>,
}

/// Client information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// Client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    pub workspace: Option<WorkspaceClientCapabilities>,
    pub text_document: Option<TextDocumentClientCapabilities>,
    pub experimental: Option<Value>,
}

/// Workspace client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceClientCapabilities {
    pub apply_edit: Option<bool>,
    pub workspace_edit: Option<WorkspaceEditClientCapabilities>,
    pub did_change_configuration: Option<DynamicRegistration>,
    pub did_change_watched_files: Option<DynamicRegistration>,
    pub symbol: Option<WorkspaceSymbolClientCapabilities>,
    pub execute_command: Option<DynamicRegistration>,
    pub workspace_folders: Option<bool>,
    pub configuration: Option<bool>,
}

/// Text document client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentClientCapabilities {
    pub synchronization: Option<TextDocumentSyncClientCapabilities>,
    pub completion: Option<CompletionClientCapabilities>,
    pub hover: Option<DynamicRegistration>,
    pub signature_help: Option<SignatureHelpClientCapabilities>,
    pub declaration: Option<DynamicRegistration>,
    pub definition: Option<DynamicRegistration>,
    pub type_definition: Option<DynamicRegistration>,
    pub implementation: Option<DynamicRegistration>,
    pub references: Option<DynamicRegistration>,
    pub document_highlight: Option<DynamicRegistration>,
    pub document_symbol: Option<DocumentSymbolClientCapabilities>,
    pub code_action: Option<CodeActionClientCapabilities>,
    pub code_lens: Option<DynamicRegistration>,
    pub document_link: Option<DynamicRegistration>,
    pub color_provider: Option<DynamicRegistration>,
    pub formatting: Option<DynamicRegistration>,
    pub range_formatting: Option<DynamicRegistration>,
    pub on_type_formatting: Option<DynamicRegistration>,
    pub rename: Option<DynamicRegistration>,
    pub publish_diagnostics: Option<PublishDiagnosticsClientCapabilities>,
    pub folding_range: Option<DynamicRegistration>,
    pub selection_range: Option<DynamicRegistration>,
    pub linked_editing_range: Option<DynamicRegistration>,
    pub call_hierarchy: Option<DynamicRegistration>,
    pub semantic_tokens: Option<SemanticTokensClientCapabilities>,
    pub moniker: Option<DynamicRegistration>,
}

/// Workspace folder
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFolder {
    pub uri: String,
    pub name: String,
}

/// Dynamic registration capability
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicRegistration {
    pub dynamic_registration: Option<bool>,
}

/// Workspace edit client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEditClientCapabilities {
    pub document_changes: Option<bool>,
    pub resource_operations: Option<Vec<String>>,
    pub failure_handling: Option<String>,
    pub normalizes_line_endings: Option<bool>,
    pub change_annotation_support: Option<ChangeAnnotationSupport>,
}

/// Change annotation support
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeAnnotationSupport {
    pub groups_on_label: Option<bool>,
}

/// Workspace symbol client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbolClientCapabilities {
    pub dynamic_registration: Option<bool>,
    pub symbol_kind: Option<SymbolKindClientCapabilities>,
    pub tag_support: Option<TagSupport>,
}

/// Symbol kind client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolKindClientCapabilities {
    pub value_set: Option<Vec<i32>>,
}

/// Tag support
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSupport {
    pub value_set: Vec<i32>,
}

/// Text document sync client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentSyncClientCapabilities {
    pub dynamic_registration: Option<bool>,
    pub will_save: Option<bool>,
    pub will_save_wait_until: Option<bool>,
    pub did_save: Option<bool>,
}

/// Completion client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionClientCapabilities {
    pub dynamic_registration: Option<bool>,
    pub completion_item: Option<CompletionItemClientCapabilities>,
    pub completion_item_kind: Option<CompletionItemKindClientCapabilities>,
    pub context_support: Option<bool>,
}

/// Completion item client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemClientCapabilities {
    pub snippet_support: Option<bool>,
    pub commit_characters_support: Option<bool>,
    pub documentation_format: Option<Vec<String>>,
    pub deprecated_support: Option<bool>,
    pub preselect_support: Option<bool>,
    pub tag_support: Option<TagSupport>,
    pub insert_replace_support: Option<bool>,
    pub resolve_support: Option<ResolveSupport>,
    pub insert_text_mode_support: Option<InsertTextModeSupport>,
}

/// Resolve support
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveSupport {
    pub properties: Vec<String>,
}

/// Insert text mode support
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertTextModeSupport {
    pub value_set: Vec<i32>,
}

/// Completion item kind client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemKindClientCapabilities {
    pub value_set: Option<Vec<i32>>,
}

/// Signature help client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureHelpClientCapabilities {
    pub dynamic_registration: Option<bool>,
    pub signature_information: Option<SignatureInformationClientCapabilities>,
    pub context_support: Option<bool>,
}

/// Signature information client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInformationClientCapabilities {
    pub documentation_format: Option<Vec<String>>,
    pub parameter_information: Option<ParameterInformationClientCapabilities>,
    pub active_parameter_support: Option<bool>,
}

/// Parameter information client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInformationClientCapabilities {
    pub label_offset_support: Option<bool>,
}

/// Document symbol client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbolClientCapabilities {
    pub dynamic_registration: Option<bool>,
    pub symbol_kind: Option<SymbolKindClientCapabilities>,
    pub hierarchical_document_symbol_support: Option<bool>,
    pub tag_support: Option<TagSupport>,
    pub label_support: Option<bool>,
}

/// Code action client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionClientCapabilities {
    pub dynamic_registration: Option<bool>,
    pub code_action_literal_support: Option<CodeActionLiteralSupport>,
    pub is_preferred_support: Option<bool>,
    pub disabled_support: Option<bool>,
    pub data_support: Option<bool>,
    pub resolve_support: Option<ResolveSupport>,
    pub honors_change_annotations: Option<bool>,
}

/// Code action literal support
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionLiteralSupport {
    pub code_action_kind: CodeActionKindSupport,
}

/// Code action kind support
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionKindSupport {
    pub value_set: Vec<String>,
}

/// Publish diagnostics client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDiagnosticsClientCapabilities {
    pub related_information: Option<bool>,
    pub tag_support: Option<TagSupport>,
    pub version_support: Option<bool>,
    pub code_description_support: Option<bool>,
    pub data_support: Option<bool>,
}

/// Semantic tokens client capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensClientCapabilities {
    pub requests: SemanticTokensRequests,
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
    pub formats: Vec<String>,
    pub overlapping_token_support: Option<bool>,
    pub multiline_token_support: Option<bool>,
    pub server_cancel_support: Option<bool>,
    pub augments_syntax_tokens: Option<bool>,
}

/// Semantic tokens requests
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensRequests {
    pub range: Option<Value>,
    pub full: Option<Value>,
}

/// LSP initialization result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
    pub server_info: Option<ServerInfo>,
}

/// Server capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub text_document_sync: Option<Value>,
    pub hover_provider: Option<Value>,
    pub completion_provider: Option<Value>,
    pub signature_help_provider: Option<Value>,
    pub definition_provider: Option<Value>,
    pub type_definition_provider: Option<Value>,
    pub implementation_provider: Option<Value>,
    pub references_provider: Option<Value>,
    pub document_highlight_provider: Option<Value>,
    pub document_symbol_provider: Option<Value>,
    pub workspace_symbol_provider: Option<Value>,
    pub code_action_provider: Option<Value>,
    pub code_lens_provider: Option<Value>,
    pub document_formatting_provider: Option<Value>,
    pub document_range_formatting_provider: Option<Value>,
    pub document_on_type_formatting_provider: Option<Value>,
    pub rename_provider: Option<Value>,
    pub document_link_provider: Option<Value>,
    pub color_provider: Option<Value>,
    pub folding_range_provider: Option<Value>,
    pub declaration_provider: Option<Value>,
    pub execute_command_provider: Option<Value>,
    pub workspace: Option<WorkspaceServerCapabilities>,
    pub experimental: Option<Value>,
    pub semantic_tokens_provider: Option<Value>,
    pub moniker_provider: Option<Value>,
    pub linked_editing_range_provider: Option<Value>,
    pub call_hierarchy_provider: Option<Value>,
    pub selection_range_provider: Option<Value>,
}

/// Workspace server capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceServerCapabilities {
    pub workspace_folders: Option<WorkspaceFoldersServerCapabilities>,
    pub file_operations: Option<FileOperationsServerCapabilities>,
}

/// Workspace folders server capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFoldersServerCapabilities {
    pub supported: Option<bool>,
    pub change_notifications: Option<Value>,
}

/// File operations server capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationsServerCapabilities {
    pub did_create: Option<Value>,
    pub will_create: Option<Value>,
    pub did_rename: Option<Value>,
    pub will_rename: Option<Value>,
    pub did_delete: Option<Value>,
    pub will_delete: Option<Value>,
}

/// Server information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// LSP protocol implementation
pub struct LspProtocol;

impl LspProtocol {
    /// Create an initialization request
    pub fn create_initialize_request(
        id: LspId,
        root_uri: Option<String>,
        workspace_folders: Option<Vec<WorkspaceFolder>>,
    ) -> LspMessage {
        Self::create_initialize_request_with_options(id, root_uri, workspace_folders, None)
    }

    pub fn create_initialize_request_minimal(id: LspId, root_uri: Option<String>) -> LspMessage {
        // Use the exact minimal format that worked in manual testing
        let params = serde_json::json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        });

        LspMessage::Request {
            id,
            method: "initialize".to_string(),
            params: Some(params),
        }
    }

    pub fn create_initialize_request_with_options(
        id: LspId,
        root_uri: Option<String>,
        workspace_folders: Option<Vec<WorkspaceFolder>>,
        initialization_options: Option<serde_json::Value>,
    ) -> LspMessage {
        // Simplified initialization for better compatibility with rust-analyzer
        let params = InitializeParams {
            process_id: None, // Use None instead of process ID for better compatibility
            client_info: Some(ClientInfo {
                name: "graphengine-parsing".to_string(),
                version: "1.0.0".to_string(),
            }),
            locale: None,
            root_path: None,
            root_uri,
            initialization_options,
            capabilities: Self::create_minimal_client_capabilities(), // Use minimal capabilities
            trace: Some("verbose".to_string()),
            workspace_folders,
        };

        LspMessage::Request {
            id,
            method: "initialize".to_string(),
            params: Some(serde_json::to_value(params).unwrap()),
        }
    }

    /// Create a shutdown request
    pub fn create_shutdown_request(id: LspId) -> LspMessage {
        LspMessage::Request {
            id,
            method: "shutdown".to_string(),
            params: None,
        }
    }

    /// Create an exit notification
    pub fn create_exit_notification() -> LspMessage {
        LspMessage::Notification {
            method: "exit".to_string(),
            params: None,
        }
    }

    /// Create a text document did open notification
    pub fn create_did_open_notification(
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    ) -> LspMessage {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": version,
                "text": text
            }
        });

        LspMessage::Notification {
            method: "textDocument/didOpen".to_string(),
            params: Some(params),
        }
    }

    /// Create a text document did close notification
    pub fn create_did_close_notification(uri: String) -> LspMessage {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            }
        });

        LspMessage::Notification {
            method: "textDocument/didClose".to_string(),
            params: Some(params),
        }
    }

    /// Create a document symbol request
    pub fn create_document_symbol_request(id: LspId, uri: String) -> LspMessage {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            }
        });

        LspMessage::Request {
            id,
            method: "textDocument/documentSymbol".to_string(),
            params: Some(params),
        }
    }

    /// Create a workspace symbol request
    pub fn create_workspace_symbol_request(id: LspId, query: String) -> LspMessage {
        let params = serde_json::json!({
            "query": query
        });

        LspMessage::Request {
            id,
            method: "workspace/symbol".to_string(),
            params: Some(params),
        }
    }

    /// Create a text document definition request
    pub fn create_definition_request(
        id: LspId,
        uri: String,
        line: u32,
        character: u32,
    ) -> LspMessage {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            },
            "position": {
                "line": line,
                "character": character
            }
        });

        LspMessage::Request {
            id,
            method: "textDocument/definition".to_string(),
            params: Some(params),
        }
    }

    /// Create a text document references request
    pub fn create_references_request(
        id: LspId,
        uri: String,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> LspMessage {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            },
            "position": {
                "line": line,
                "character": character
            },
            "context": {
                "includeDeclaration": include_declaration
            }
        });

        LspMessage::Request {
            id,
            method: "textDocument/references".to_string(),
            params: Some(params),
        }
    }

    /// Create a text document hover request
    pub fn create_hover_request(id: LspId, uri: String, line: u32, character: u32) -> LspMessage {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            },
            "position": {
                "line": line,
                "character": character
            }
        });

        LspMessage::Request {
            id,
            method: "textDocument/hover".to_string(),
            params: Some(params),
        }
    }

    /// Create client capabilities
    fn create_minimal_client_capabilities() -> ClientCapabilities {
        ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities {
                apply_edit: Some(true),
                workspace_edit: None,
                did_change_configuration: None,
                did_change_watched_files: None,
                symbol: None,
                execute_command: None,
                workspace_folders: Some(true),
                configuration: None,
            }),
            text_document: Some(TextDocumentClientCapabilities {
                synchronization: Some(TextDocumentSyncClientCapabilities {
                    dynamic_registration: Some(true),
                    will_save: Some(true),
                    will_save_wait_until: Some(true),
                    did_save: Some(true),
                }),
                completion: None,
                hover: None,
                signature_help: None,
                declaration: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                definition: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                type_definition: None,
                implementation: None,
                references: None,
                document_highlight: None,
                document_symbol: None,
                code_action: None,
                code_lens: None,
                document_link: None,
                color_provider: None,
                formatting: None,
                range_formatting: None,
                on_type_formatting: None,
                rename: None,
                folding_range: None,
                selection_range: None,
                linked_editing_range: None,
                call_hierarchy: None,
                semantic_tokens: None,
                moniker: None,
                publish_diagnostics: None,
            }),
            experimental: None,
        }
    }

    // REMOVED: create_client_capabilities - unused
    fn _removed_create_client_capabilities() -> ClientCapabilities {
        ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities {
                apply_edit: Some(true),
                workspace_edit: Some(WorkspaceEditClientCapabilities {
                    document_changes: Some(true),
                    resource_operations: Some(vec![
                        "create".to_string(),
                        "rename".to_string(),
                        "delete".to_string(),
                    ]),
                    failure_handling: Some("abort".to_string()),
                    normalizes_line_endings: Some(true),
                    change_annotation_support: None,
                }),
                did_change_configuration: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                did_change_watched_files: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                symbol: Some(WorkspaceSymbolClientCapabilities {
                    dynamic_registration: Some(true),
                    symbol_kind: Some(SymbolKindClientCapabilities { value_set: None }),
                    tag_support: None,
                }),
                execute_command: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                workspace_folders: Some(true),
                configuration: Some(true),
            }),
            text_document: Some(TextDocumentClientCapabilities {
                synchronization: Some(TextDocumentSyncClientCapabilities {
                    dynamic_registration: Some(true),
                    will_save: Some(true),
                    will_save_wait_until: Some(true),
                    did_save: Some(true),
                }),
                completion: Some(CompletionClientCapabilities {
                    dynamic_registration: Some(true),
                    completion_item: Some(CompletionItemClientCapabilities {
                        snippet_support: Some(true),
                        commit_characters_support: Some(true),
                        documentation_format: Some(vec![
                            "markdown".to_string(),
                            "plaintext".to_string(),
                        ]),
                        deprecated_support: Some(true),
                        preselect_support: Some(true),
                        tag_support: None,
                        insert_replace_support: Some(true),
                        resolve_support: Some(ResolveSupport {
                            properties: vec![
                                "documentation".to_string(),
                                "detail".to_string(),
                                "additionalTextEdits".to_string(),
                            ],
                        }),
                        insert_text_mode_support: None,
                    }),
                    completion_item_kind: Some(CompletionItemKindClientCapabilities {
                        value_set: None,
                    }),
                    context_support: Some(true),
                }),
                hover: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                signature_help: Some(SignatureHelpClientCapabilities {
                    dynamic_registration: Some(true),
                    signature_information: Some(SignatureInformationClientCapabilities {
                        documentation_format: Some(vec![
                            "markdown".to_string(),
                            "plaintext".to_string(),
                        ]),
                        parameter_information: Some(ParameterInformationClientCapabilities {
                            label_offset_support: Some(true),
                        }),
                        active_parameter_support: Some(true),
                    }),
                    context_support: Some(true),
                }),
                declaration: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                definition: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                type_definition: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                implementation: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                references: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                document_highlight: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                document_symbol: Some(DocumentSymbolClientCapabilities {
                    dynamic_registration: Some(true),
                    symbol_kind: Some(SymbolKindClientCapabilities { value_set: None }),
                    hierarchical_document_symbol_support: Some(true),
                    tag_support: None,
                    label_support: Some(true),
                }),
                code_action: Some(CodeActionClientCapabilities {
                    dynamic_registration: Some(true),
                    code_action_literal_support: Some(CodeActionLiteralSupport {
                        code_action_kind: CodeActionKindSupport {
                            value_set: vec![
                                "quickfix".to_string(),
                                "refactor".to_string(),
                                "source".to_string(),
                            ],
                        },
                    }),
                    is_preferred_support: Some(true),
                    disabled_support: Some(true),
                    data_support: Some(true),
                    resolve_support: Some(ResolveSupport {
                        properties: vec!["edit".to_string()],
                    }),
                    honors_change_annotations: Some(true),
                }),
                code_lens: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                document_link: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                color_provider: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                formatting: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                range_formatting: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                on_type_formatting: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                rename: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                    related_information: Some(true),
                    tag_support: None,
                    version_support: Some(true),
                    code_description_support: Some(true),
                    data_support: Some(true),
                }),
                folding_range: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                selection_range: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                linked_editing_range: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                call_hierarchy: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
                semantic_tokens: Some(SemanticTokensClientCapabilities {
                    requests: SemanticTokensRequests {
                        range: Some(serde_json::json!(true)),
                        full: Some(serde_json::json!(true)),
                    },
                    token_types: vec![
                        "namespace".to_string(),
                        "type".to_string(),
                        "class".to_string(),
                        "enum".to_string(),
                        "interface".to_string(),
                        "struct".to_string(),
                        "typeParameter".to_string(),
                        "parameter".to_string(),
                        "variable".to_string(),
                        "property".to_string(),
                        "enumMember".to_string(),
                        "event".to_string(),
                        "function".to_string(),
                        "method".to_string(),
                        "macro".to_string(),
                        "keyword".to_string(),
                        "modifier".to_string(),
                        "comment".to_string(),
                        "string".to_string(),
                        "number".to_string(),
                        "regexp".to_string(),
                        "operator".to_string(),
                    ],
                    token_modifiers: vec![
                        "declaration".to_string(),
                        "definition".to_string(),
                        "readonly".to_string(),
                        "static".to_string(),
                        "deprecated".to_string(),
                        "abstract".to_string(),
                        "async".to_string(),
                        "modification".to_string(),
                        "documentation".to_string(),
                        "defaultLibrary".to_string(),
                    ],
                    formats: vec!["relative".to_string()],
                    overlapping_token_support: Some(false),
                    multiline_token_support: Some(false),
                    server_cancel_support: Some(false),
                    augments_syntax_tokens: Some(false),
                }),
                moniker: Some(DynamicRegistration {
                    dynamic_registration: Some(true),
                }),
            }),
            experimental: None,
        }
    }

    /// Serialize a message to JSON
    pub fn serialize_message(message: &LspMessage) -> Result<String, serde_json::Error> {
        let mut object = serde_json::map::Map::new();
        object.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));

        match message {
            LspMessage::Request { id, method, params } => {
                object.insert("id".to_string(), serde_json::to_value(id)?);
                object.insert("method".to_string(), Value::String(method.clone()));
                if let Some(p) = params {
                    object.insert("params".to_string(), p.clone());
                }
            }
            LspMessage::Response { id, result, error } => {
                object.insert("id".to_string(), serde_json::to_value(id)?);
                if let Some(res) = result {
                    object.insert("result".to_string(), res.clone());
                }
                if let Some(err) = error {
                    object.insert("error".to_string(), serde_json::to_value(err)?);
                }
            }
            LspMessage::Notification { method, params } => {
                object.insert("method".to_string(), Value::String(method.clone()));
                if let Some(p) = params {
                    object.insert("params".to_string(), p.clone());
                }
            }
        }

        serde_json::to_string(&Value::Object(object))
    }

    /// Deserialize a message from JSON
    pub fn deserialize_message(json: &str) -> Result<LspMessage, serde_json::Error> {
        let value: Value = serde_json::from_str(json)?;
        Self::value_to_message(value)
    }

    /// Parse a message from bytes
    pub fn parse_message(bytes: &[u8]) -> Result<LspMessage, serde_json::Error> {
        let value: Value = serde_json::from_slice(bytes)?;
        Self::value_to_message(value)
    }

    fn value_to_message(value: Value) -> Result<LspMessage, serde_json::Error> {
        let mut map = match value {
            Value::Object(map) => map,
            other => {
                return Err(DeError::custom(format!(
                    "Expected JSON object for LSP message, got {}",
                    other
                )));
            }
        };

        match map.get("jsonrpc") {
            Some(Value::String(version)) if version == "2.0" => {}
            _ => {
                return Err(DeError::custom(
                    "Missing or invalid jsonrpc version, expected '2.0'",
                ));
            }
        }

        let has_method = map.contains_key("method");
        let has_id = map.contains_key("id");

        if has_id && has_method {
            // Request
            let id_value = map.remove("id").unwrap();
            let method_value = map.remove("method").unwrap();
            let params = map.remove("params");

            let id: LspId = serde_json::from_value(id_value)?;
            let method = method_value
                .as_str()
                .ok_or_else(|| DeError::custom("method must be a string"))?
                .to_string();

            Ok(LspMessage::Request { id, method, params })
        } else if has_id {
            // Response
            let id_value = map.remove("id").unwrap();
            let result = map.remove("result");
            let error = map.remove("error");

            let id: LspId = serde_json::from_value(id_value)?;
            let error: Option<LspError> = error.map(serde_json::from_value).transpose()?;

            Ok(LspMessage::Response { id, result, error })
        } else if has_method {
            // Notification
            let method_value = map.remove("method").unwrap();
            let params = map.remove("params");

            let method = method_value
                .as_str()
                .ok_or_else(|| DeError::custom("method must be a string"))?
                .to_string();

            Ok(LspMessage::Notification { method, params })
        } else {
            Err(DeError::custom(
                "Invalid LSP message: missing id/method fields",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_initialize_request() {
        let message = LspProtocol::create_initialize_request(
            LspId::Number(1),
            Some("file:///test".to_string()),
            None,
        );

        match message {
            LspMessage::Request { method, params, .. } => {
                assert_eq!(method, "initialize");
                assert!(params.is_some());
            }
            _ => panic!("Expected request message"),
        }
    }

    #[test]
    fn test_serialize_deserialize() {
        let original = LspProtocol::create_initialize_request(
            LspId::Number(1),
            Some("file:///test".to_string()),
            None,
        );

        let json = LspProtocol::serialize_message(&original).unwrap();
        let deserialized = LspProtocol::deserialize_message(&json).unwrap();

        match (original, deserialized) {
            (LspMessage::Request { method: m1, .. }, LspMessage::Request { method: m2, .. }) => {
                assert_eq!(m1, m2);
            }
            _ => panic!("Expected request messages"),
        }
    }

    /// Proof that `create_initialize_request_with_options` carries the
    /// `initializationOptions` through to the wire JSON. apex-jorje requires
    /// these to be present or it disables its semantic pipeline. If this test
    /// ever regresses, Apex resolution silently degrades.
    #[test]
    fn with_options_emits_initialization_options_on_wire() {
        let options = serde_json::json!({ "enableSemanticErrors": true });
        let msg = LspProtocol::create_initialize_request_with_options(
            LspId::Number(1),
            Some("file:///repo".to_string()),
            None,
            Some(options.clone()),
        );

        let params = match &msg {
            LspMessage::Request { params, .. } => params.clone().expect("params must be present"),
            _ => panic!("expected request"),
        };

        let init_opts = params
            .get("initializationOptions")
            .expect("initializationOptions key must be present on wire");
        assert_eq!(
            init_opts, &options,
            "initializationOptions must round-trip unchanged"
        );

        let root_uri = params
            .get("rootUri")
            .and_then(|v| v.as_str())
            .expect("rootUri must be present");
        assert_eq!(root_uri, "file:///repo");

        let client_info = params
            .get("clientInfo")
            .expect("clientInfo must be present for apex-jorje");
        assert_eq!(
            client_info.get("name").and_then(|v| v.as_str()),
            Some("graphengine-parsing")
        );
    }

    /// Proof that the minimal init path (used by all currently-supported
    /// languages when `lsp_initialization_options` is unset) does NOT emit an
    /// `initializationOptions` key. rust-analyzer and some other servers
    /// behave differently when the field is present but empty; we must keep
    /// the pre-Apex byte-shape for them.
    #[test]
    fn minimal_path_omits_initialization_options() {
        let msg = LspProtocol::create_initialize_request_minimal(
            LspId::Number(1),
            Some("file:///repo".to_string()),
        );

        let params = match &msg {
            LspMessage::Request { params, .. } => params.clone().expect("params must be present"),
            _ => panic!("expected request"),
        };

        assert!(
            params.get("initializationOptions").is_none(),
            "minimal path must not include initializationOptions; got params={}",
            serde_json::to_string(&params).unwrap()
        );
        let obj = params.as_object().expect("params object");
        let keys: std::collections::BTreeSet<&str> = obj.keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            ["capabilities", "processId", "rootUri"]
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            "minimal path keys drifted — other languages may regress"
        );
    }
}
