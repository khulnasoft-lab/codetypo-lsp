//! LSP server implementation using tower-lsp for Codetypo.

use matchit::Match;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use codetypo_cli::policy;
use serde_json::{json, to_string};
use tower_lsp::lsp_types::*;
use tower_lsp::*;
use tower_lsp::{Client, LanguageServer};

use crate::state::{url_path_sanitised, BackendState};
/// LSP backend for Codetypo, managing client and workspace state.
pub struct Backend<'s, 'p> {
    client: Client,
    state: Mutex<crate::state::BackendState<'s>>,
    default_policy: policy::Policy<'p, 'p, 'p>,
}

/// Diagnostic data attached to LSP diagnostics, including correction suggestions.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DiagnosticData<'c> {
    corrections: Vec<Cow<'c, str>>,
}

#[tower_lsp::async_trait]
/// Implements the LSP server for Codetypo.
#[tower_lsp::async_trait]
impl LanguageServer for Backend<'static, 'static> {
    /// Handles LSP initialize request.
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        tracing::debug!("initialize: {}", to_string(&params).unwrap_or_default());

        if let Some(TextDocumentClientCapabilities {
            publish_diagnostics:
                Some(PublishDiagnosticsClientCapabilities {
                    data_support: Some(true),
                    ..
                }),
            ..
        }) = params.capabilities.text_document
        {
            tracing::debug!("Client supports diagnostics data")
        } else {
            tracing::warn!(
                "Client does not support diagnostics data. Code actions will not be available"
            )
        }

        let mut state = self.state.lock().unwrap();

        if let Some(ops) = params.initialization_options {
            if let Some(values) = ops.as_object() {
                if let Some(value) = values.get("diagnosticSeverity").cloned() {
                    match value.as_str().unwrap_or("").to_lowercase().as_str() {
                        "error" => {
                            state.severity = Some(DiagnosticSeverity::ERROR);
                        }
                        "warning" => {
                            state.severity = Some(DiagnosticSeverity::WARNING);
                        }
                        "information" | "info" => {
                            state.severity = Some(DiagnosticSeverity::INFORMATION);
                        }
                        "hint" => {
                            state.severity = Some(DiagnosticSeverity::HINT);
                        }
                        _ => {
                            tracing::warn!("Unknown diagnostic severity: {}", value);
                        }
                    }
                }
                if let Some(value) = values.get("config").cloned() {
                    if let Some(value) = value.as_str() {
                        let expanded_path = PathBuf::from(shellexpand::tilde(value).to_string());
                        state.config = Some(expanded_path);
                    }
                }
            }
        }

        if let Err(e) = state.set_workspace_folders(params.workspace_folders.unwrap_or_default()) {
            tracing::warn!("Falling back to default config: {}", e);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // only support UTF-16 positions for now, which is the default when unspecified
                position_encoding: Some(PositionEncodingKind::UTF16),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    // TODO: should we support incremental?
                    TextDocumentSyncKind::FULL,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: Some(false),
                        },
                        resolve_provider: None,
                    },
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "codetypo".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    /// Called when the server has been initialized.
    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    /// Handles opening of a text document.
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        tracing::debug!("did_open: {:?}", to_string(&params).unwrap_or_default());
        self.report_diagnostics(params.text_document).await;
    }

    /// Handles changes to a text document.
    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        tracing::debug!("did_change: {:?}", to_string(&params).unwrap_or_default());
        self.report_diagnostics(TextDocumentItem {
            language_id: "FOOBAR".to_string(),
            uri: params.text_document.uri,
            text: std::mem::take(&mut params.content_changes[0].text),
            version: params.text_document.version,
        })
        .await;
    }

    /// Handles saving of a text document.
    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        tracing::debug!("did_save: {:?}", to_string(&params).unwrap_or_default());
        // noop to avoid unimplemented warning log line
    }

    /// Handles closing of a text document.
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        tracing::debug!("did_close: {:?}", to_string(&params).unwrap_or_default());
        // clear diagnostics to avoid a stale diagnostics flash on open
        // if the file has codetypo fixed outside of vscode
        // see https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_publishDiagnostics
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    /// Provides code actions (quick fixes) for diagnostics.
    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> jsonrpc::Result<Option<CodeActionResponse>> {
        tracing::debug!("code_action: {:?}", to_string(&params).unwrap_or_default());

        let actions = params
            .context
            .diagnostics
            .iter()
            .filter(|diag| diag.source == Some("codetypo".to_string()))
            .flat_map(|diag| match &diag.data {
                Some(data) => {
                    if let Ok(DiagnosticData { corrections }) =
                        serde_json::from_value::<DiagnosticData>(data.clone())
                    {
                        corrections
                            .iter()
                            .map(|c| {
                                CodeActionOrCommand::CodeAction(CodeAction {
                                    title: c.to_string(),
                                    kind: Some(CodeActionKind::QUICKFIX),
                                    diagnostics: Some(vec![diag.clone()]),
                                    edit: Some(WorkspaceEdit {
                                        changes: Some(HashMap::from([(
                                            params.text_document.uri.clone(),
                                            vec![TextEdit {
                                                range: diag.range,
                                                new_text: c.to_string(),
                                            }],
                                        )])),
                                        ..WorkspaceEdit::default()
                                    }),
                                    is_preferred: if corrections.len() == 1 {
                                        Some(true)
                                    } else {
                                        None
                                    },
                                    ..CodeAction::default()
                                })
                            })
                            .collect()
                    } else {
                        tracing::error!(
                            "Deserialization failed: received {:?} as diagnostic data",
                            data
                        );
                        vec![]
                    }
                }
                None => {
                    tracing::warn!("Client doesn't support diagnostic data");
                    vec![]
                }
            })
            .collect::<Vec<_>>();

        Ok(Some(actions))
    }

    /// Handles workspace folder changes.
    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        tracing::debug!(
            "did_change_workspace_folders: {:?}",
            to_string(&params).unwrap_or_default()
        );

        let mut state = self.state.lock().unwrap();
        if let Err(e) = state.update_workspace_folders(params.event.added, params.event.removed) {
            tracing::warn!("Cannot update workspace folders {}", e);
        }
    }

    /// Handles LSP shutdown request.
    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }
}

impl<'s> Backend<'s, '_> {
    /// Constructs a new `Backend` with the given LSP client.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Mutex::new(BackendState::default()),
            default_policy: policy::Policy::default(),
        }
    }

    /// Reports diagnostics for the given text document.
    ///
    /// # Parameters
    ///
    /// * `params`: The text document to report diagnostics for.
    pub async fn report_diagnostics(&self, params: TextDocumentItem) {
        let diagnostics = self.check_text(&params.text, &params.uri);
        self.client
            .publish_diagnostics(params.uri, diagnostics, Some(params.version))
            .await;
    }

    /// Checks the given text for typos and returns diagnostics.
    ///
    /// # Parameters
    ///
    /// * `buffer`: The text to check for typos.
    /// * `uri`: The URI of the text document.
    pub fn check_text(&self, buffer: &str, uri: &Url) -> Vec<Diagnostic> {
        let state = self.state.lock().unwrap();

        let Some((tokenizer, dict, ignore)) = self.workspace_policy(uri, &state) else {
            // skip file because it matches extend-exclude
            return Vec::default();
        };

        crate::codetypo::check_str(buffer, tokenizer, dict, ignore)
            .map(|(typo, line_num, line_pos)| {
                Diagnostic {
                    range: Range::new(
                        Position::new(line_num as u32, line_pos as u32),
                        Position::new(line_num as u32, (line_pos + typo.typo.len()) as u32),
                    ),
                    severity: state.severity,
                    source: Some("codetypo".to_string()),
                    message: match &typo.corrections {
                        codetypo::Status::Invalid => format!("`{}` is disallowed", typo.typo),
                        codetypo::Status::Corrections(corrections) => format!(
                            "`{}` should be {}",
                            typo.typo,
                            itertools::join(corrections.iter().map(|s| format!("`{}`", s)), ", ")
                        ),
                        codetypo::Status::Valid => panic!("unexpected codetypo::Status::Valid"),
                    },
                    // store corrections for retrieval during code_action
                    data: match typo.corrections {
                        codetypo::Status::Corrections(corrections) => {
                            Some(json!(DiagnosticData { corrections }))
                        }
                        _ => None,
                    },
                    ..Diagnostic::default()
                }
            })
            .collect()
    }

    /// Determines the workspace policy (tokenizer, dictionary, ignore rules) for a given URI.
    ///
    /// # Parameters
    ///
    /// * `uri`: The URI to determine the workspace policy for.
    /// * `state`: The current state of the backend.
    pub(crate) fn workspace_policy<'a>(
        &'a self,
        uri: &Url,
        state: &'a std::sync::MutexGuard<'a, BackendState<'s>>,
    ) -> Option<(
        &'a codetypo::tokens::Tokenizer,
        &'a dyn codetypo::Dictionary,
        &'a [regex::Regex],
    )> {
        let (tokenizer, dict, ignore) = match uri.to_file_path() {
            Err(_) => {
                // eg: uris like untitled:* or term://*
                tracing::debug!(
                    "workspace_policy: Using default policy because cannot convert uri {} to file path",
                    uri
                );
                (
                    self.default_policy.tokenizer,
                    self.default_policy.dict,
                    self.default_policy.ignore,
                )
            }
            Ok(path) => {
                let uri_path = url_path_sanitised(uri);

                // find relevant tokenizer, and dict for the workspace folder
                let (tokenizer, dict, ignore) = match state.router.at(&uri_path) {
                    Err(_) => {
                        // ie: file:///
                        tracing::debug!(
                            "workspace_policy: Using default policy because no route found for {}",
                            uri_path
                        );
                        (
                            self.default_policy.tokenizer,
                            self.default_policy.dict,
                            self.default_policy.ignore,
                        )
                    }
                    Ok(Match { value, params: _ }) => {
                        tracing::debug!("workspace_policy: path {}", &path.display());
                        // skip file if matches extend-exclude
                        if value.ignores.matched(&path, false).is_ignore() {
                            tracing::debug!(
                                "workspace_policy: Ignoring {} because it matches extend-exclude.",
                                uri
                            );
                            return None;
                        }
                        let policy = value.engine.policy(&path);
                        (policy.tokenizer, policy.dict, policy.ignore)
                    }
                };

                (tokenizer, dict, ignore)
            }
        };
        Some((tokenizer, dict, ignore))
    }
}
