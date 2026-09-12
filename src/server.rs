// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::error::Error;

use crate::{
    analysis, builtins, formatting, inlay_hints, locals, parser, remote_documents,
    remote_navigation, semantic_tokens,
};
use lsp_server::{Connection, ErrorCode, Message, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, LogMessage, Notification,
    PublishDiagnostics,
};
use lsp_types::request::{
    DocumentHighlightRequest, DocumentSymbolRequest, FoldingRangeRequest, Formatting,
    GotoDefinition, HoverRequest, InlayHintRequest, Request as LspRequest,
    SemanticTokensFullRequest, SignatureHelpRequest,
};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentHighlight, DocumentHighlightParams, DocumentSymbolParams,
    DocumentSymbolResponse, FoldingRange, FoldingRangeParams, FoldingRangeProviderCapability,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, HoverProviderCapability,
    InlayHint, InlayHintParams, LogMessageParams, MessageType, OneOf, Position,
    PublishDiagnosticsParams, Range, SemanticTokens, SemanticTokensFullOptions,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, SignatureHelp, SignatureHelpOptions,
    SignatureHelpParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri,
};

type ServerResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub fn run(connection: Connection) -> ServerResult<()> {
    let mut session = Session::new();
    for message in &connection.receiver {
        let exit = matches!(&message, Message::Notification(notification) if notification.method == "exit");
        for output in session.handle(message) {
            connection.sender.send(output)?;
        }
        if exit {
            break;
        }
    }
    Ok(())
}

/// A transport-independent LSP session.
///
/// Each input is one complete, headerless JSON-RPC message. Callers deliver the
/// messages returned by [`Session::handle`] to the client in order.
#[derive(Default)]
pub struct Session {
    server: Server,
    initialized: bool,
    shutdown_requested: bool,
    remote_documents: bool,
    next_request_id: i32,
    pending: HashMap<RequestId, PendingRemoteRead>,
}

const READ_DOCUMENT_METHOD: &str = "lambdamoo/readDocument";
const CANONICALIZE_DOCUMENT_METHOD: &str = "lambdamoo/canonicalizeDocument";

#[derive(Debug)]
enum PendingRemoteRead {
    Definition {
        response_id: RequestId,
        target: remote_documents::VerbTarget,
    },
    HoverResolution {
        response_id: RequestId,
        target: remote_documents::VerbTarget,
    },
    HoverDocument {
        response_id: RequestId,
        uri: Uri,
        resolved: bool,
    },
    OpenDocument {
        original_uri: Uri,
        target: remote_documents::VerbTarget,
    },
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&mut self, message: Message) -> Vec<Message> {
        let mut output = Vec::new();
        match message {
            Message::Request(request) if request.method == "initialize" => {
                if self.initialized {
                    output.push(error_response(
                        request.id,
                        ErrorCode::InvalidRequest,
                        "Server is already initialized".to_owned(),
                    ));
                } else {
                    self.server.remote_links = supports_show_document(&request.params);
                    self.remote_documents = supports_remote_documents(&request.params);
                    self.initialized = true;
                    output.push(Message::Response(Response::new_ok(
                        request.id,
                        serde_json::json!({ "capabilities": server_capabilities() }),
                    )));
                }
            }
            Message::Request(request) if request.method == "shutdown" => {
                self.shutdown_requested = true;
                output.push(Message::Response(Response::new_ok(request.id, ())));
            }
            Message::Request(request) if !self.initialized || self.shutdown_requested => {
                output.push(error_response(
                    request.id,
                    ErrorCode::InvalidRequest,
                    "Server is not running".to_owned(),
                ));
            }
            Message::Request(request) if request.method == GotoDefinition::METHOD => {
                self.start_definition(&mut output, request);
            }
            Message::Request(request) if request.method == HoverRequest::METHOD => {
                self.start_hover(&mut output, request);
            }
            Message::Request(request) => self.server.handle_request_to(&mut output, request),
            Message::Notification(notification) if notification.method == "initialized" => {
                push_log(&mut output, "server initialized!");
            }
            Message::Notification(notification) if notification.method == "exit" => {}
            Message::Notification(notification) if self.initialized && !self.shutdown_requested => {
                let method = notification.method.clone();
                if method == "$/cancelRequest" {
                    self.cancel_request(&mut output, &notification.params);
                    return output;
                }
                let opened_uri = if notification.method == DidOpenTextDocument::METHOD {
                    serde_json::from_value::<DidOpenTextDocumentParams>(notification.params.clone())
                        .ok()
                        .map(|params| params.text_document.uri)
                } else {
                    None
                };
                if let Err(error) = self
                    .server
                    .handle_notification_to(&mut output, notification)
                {
                    push_log(
                        &mut output,
                        format!("Invalid {method} notification: {error}"),
                    );
                }
                if let Some(uri) = opened_uri {
                    self.start_open_canonicalization(&mut output, uri);
                }
            }
            Message::Response(response) => self.finish_remote_read(&mut output, response),
            Message::Notification(_) => {}
        }
        output
    }

    fn start_definition(&mut self, output: &mut Vec<Message>, request: Request) {
        let params = match serde_json::from_value::<GotoDefinitionParams>(request.params) {
            Ok(params) => params,
            Err(error) => {
                output.push(error_response(
                    request.id,
                    ErrorCode::InvalidParams,
                    error.to_string(),
                ));
                return;
            }
        };
        let result = self.server.definition(&params);
        let Some(GotoDefinitionResponse::Scalar(mut location)) = result else {
            output.push(Message::Response(Response::new_ok(request.id, result)));
            return;
        };
        location.uri = remote_documents::canonical_owned_uri(&location.uri);
        let Some(target) = remote_documents::verb_target(&location.uri) else {
            output.push(Message::Response(Response::new_ok(
                request.id,
                GotoDefinitionResponse::Scalar(location),
            )));
            return;
        };
        if !self.remote_documents {
            output.push(Message::Response(Response::new_ok(
                request.id,
                GotoDefinitionResponse::Scalar(location),
            )));
            return;
        }
        self.read_document(
            output,
            target.resolution_uri.clone(),
            PendingRemoteRead::Definition {
                response_id: request.id,
                target,
            },
        );
    }

    fn start_hover(&mut self, output: &mut Vec<Message>, request: Request) {
        let params = match serde_json::from_value::<HoverParams>(request.params) {
            Ok(params) => params,
            Err(error) => {
                output.push(error_response(
                    request.id,
                    ErrorCode::InvalidParams,
                    error.to_string(),
                ));
                return;
            }
        };
        if let Some(hover) = self.server.hover(&params) {
            output.push(Message::Response(Response::new_ok(request.id, hover)));
            return;
        }
        if !self.remote_documents {
            output.push(Message::Response(Response::new_ok(
                request.id,
                Option::<Hover>::None,
            )));
            return;
        }
        let Some(location) = self.server.hover_definition(&params) else {
            output.push(Message::Response(Response::new_ok(
                request.id,
                Option::<Hover>::None,
            )));
            return;
        };
        let uri = remote_documents::canonical_owned_uri(&location.uri);
        let Some(target) = remote_documents::verb_target(&uri) else {
            output.push(Message::Response(Response::new_ok(
                request.id,
                Option::<Hover>::None,
            )));
            return;
        };
        self.read_document(
            output,
            target.resolution_uri.clone(),
            PendingRemoteRead::HoverResolution {
                response_id: request.id,
                target,
            },
        );
    }

    fn start_open_canonicalization(&mut self, output: &mut Vec<Message>, original_uri: Uri) {
        if !self.remote_documents || !original_uri.as_str().starts_with("moo://") {
            return;
        }
        let owned = remote_documents::canonical_owned_uri(&original_uri);
        let Some(target) = remote_documents::verb_target(&owned) else {
            self.notify_canonical_uri(output, original_uri, owned);
            return;
        };
        self.read_document(
            output,
            target.resolution_uri.clone(),
            PendingRemoteRead::OpenDocument {
                original_uri,
                target,
            },
        );
    }

    fn read_document(&mut self, output: &mut Vec<Message>, uri: Uri, pending: PendingRemoteRead) {
        let id = RequestId::from(format!("lambdamoo-{}", self.next_request_id));
        self.next_request_id += 1;
        self.pending.insert(id.clone(), pending);
        output.push(Message::Request(Request::new(
            id,
            READ_DOCUMENT_METHOD.to_owned(),
            serde_json::json!({ "uri": uri }),
        )));
    }

    fn finish_remote_read(&mut self, output: &mut Vec<Message>, response: Response) {
        let Some(pending) = self.pending.remove(&response.id) else {
            return;
        };
        let text = response
            .response_result
            .as_ref()
            .ok()
            .and_then(|result| result.get("text"))
            .and_then(serde_json::Value::as_str);
        match pending {
            PendingRemoteRead::Definition {
                response_id,
                target,
            } => {
                let uri = text
                    .and_then(|text| remote_documents::resolve_verb_uri(&target, text))
                    .unwrap_or(target.uri);
                output.push(Message::Response(Response::new_ok(
                    response_id,
                    GotoDefinitionResponse::Scalar(lsp_types::Location {
                        uri,
                        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                    }),
                )));
            }
            PendingRemoteRead::HoverResolution {
                response_id,
                target,
            } => {
                let resolved_uri =
                    text.and_then(|text| remote_documents::resolve_verb_uri(&target, text));
                let resolved = resolved_uri.is_some();
                let uri = resolved_uri.unwrap_or(target.uri);
                self.read_document(
                    output,
                    uri.clone(),
                    PendingRemoteRead::HoverDocument {
                        response_id,
                        uri,
                        resolved,
                    },
                );
            }
            PendingRemoteRead::HoverDocument {
                response_id,
                uri,
                resolved,
            } => {
                output.push(Message::Response(Response::new_ok(
                    response_id,
                    remote_documents::hover(&uri, text, resolved),
                )));
            }
            PendingRemoteRead::OpenDocument {
                original_uri,
                target,
            } => {
                let uri = text
                    .and_then(|text| remote_documents::resolve_verb_uri(&target, text))
                    .unwrap_or(target.uri);
                self.notify_canonical_uri(output, original_uri, uri);
            }
        }
    }

    fn notify_canonical_uri(&self, output: &mut Vec<Message>, uri: Uri, canonical_uri: Uri) {
        if uri == canonical_uri {
            return;
        }
        output.push(Message::Notification(lsp_server::Notification::new(
            CANONICALIZE_DOCUMENT_METHOD.to_owned(),
            serde_json::json!({ "uri": uri, "canonicalUri": canonical_uri }),
        )));
    }

    fn cancel_request(&mut self, output: &mut Vec<Message>, params: &serde_json::Value) {
        let Some(cancelled_id) = params.get("id") else {
            return;
        };
        let remote_ids: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(remote_id, pending)| {
                let response_id = match pending {
                    PendingRemoteRead::Definition { response_id, .. }
                    | PendingRemoteRead::HoverResolution { response_id, .. }
                    | PendingRemoteRead::HoverDocument { response_id, .. } => Some(response_id),
                    PendingRemoteRead::OpenDocument { .. } => None,
                }?;
                (serde_json::to_value(response_id).ok().as_ref() == Some(cancelled_id))
                    .then(|| remote_id.clone())
            })
            .collect();
        if remote_ids.is_empty() {
            return;
        }
        for remote_id in remote_ids {
            self.pending.remove(&remote_id);
            output.push(Message::Notification(lsp_server::Notification::new(
                "$/cancelRequest".to_owned(),
                serde_json::json!({ "id": remote_id }),
            )));
        }
        if let Ok(response_id) = serde_json::from_value::<RequestId>(cancelled_id.clone()) {
            output.push(Message::Response(Response::new_err(
                response_id,
                -32800,
                "Request cancelled".to_owned(),
            )));
        }
    }
}

fn error_response(id: RequestId, code: ErrorCode, message: String) -> Message {
    Message::Response(Response::new_err(id, code as i32, message))
}

fn push_log(output: &mut Vec<Message>, message: impl Into<String>) {
    output.push(
        lsp_server::Notification::new(
            LogMessage::METHOD.to_owned(),
            LogMessageParams {
                typ: MessageType::INFO,
                message: message.into(),
            },
        )
        .into(),
    );
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: semantic_tokens::legend(),
                full: Some(SemanticTokensFullOptions::Bool(true)),
                ..Default::default()
            },
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_owned(), ",".to_owned()]),
            retrigger_characters: Some(vec![",".to_owned()]),
            ..Default::default()
        }),
        experimental: Some(serde_json::json!({
            "lambdamoo": { "remoteDocuments": 1 }
        })),
        ..Default::default()
    }
}

#[derive(Default)]
struct Server {
    documents: HashMap<Uri, String>,
    remote_links: bool,
}

impl Server {
    fn handle_request_to(&self, output: &mut Vec<Message>, request: Request) {
        match request.method.as_str() {
            SemanticTokensFullRequest::METHOD => {
                let params =
                    match serde_json::from_value::<SemanticTokensParams>(request.params.clone()) {
                        Ok(params) => params,
                        Err(error) => {
                            output.push(error_response(
                                request.id,
                                ErrorCode::InvalidParams,
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                let result = self.documents.get(&params.text_document.uri).map(|text| {
                    SemanticTokensResult::Tokens(SemanticTokens {
                        result_id: None,
                        data: semantic_tokens::collect(text),
                    })
                });
                output.push(Message::Response(Response::new_ok(request.id, result)));
            }
            Formatting::METHOD => {
                let params = match serde_json::from_value::<DocumentFormattingParams>(
                    request.params.clone(),
                ) {
                    Ok(params) => params,
                    Err(error) => {
                        output.push(error_response(
                            request.id,
                            ErrorCode::InvalidParams,
                            error.to_string(),
                        ));
                        return;
                    }
                };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.formatting(&params),
                )));
            }
            FoldingRangeRequest::METHOD => {
                let params =
                    match serde_json::from_value::<FoldingRangeParams>(request.params.clone()) {
                        Ok(params) => params,
                        Err(error) => {
                            output.push(error_response(
                                request.id,
                                ErrorCode::InvalidParams,
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.folding_range(&params),
                )));
            }
            DocumentSymbolRequest::METHOD => {
                let params =
                    match serde_json::from_value::<DocumentSymbolParams>(request.params.clone()) {
                        Ok(params) => params,
                        Err(error) => {
                            output.push(error_response(
                                request.id,
                                ErrorCode::InvalidParams,
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.document_symbols(&params),
                )));
            }
            GotoDefinition::METHOD => {
                let params =
                    match serde_json::from_value::<GotoDefinitionParams>(request.params.clone()) {
                        Ok(params) => params,
                        Err(error) => {
                            output.push(error_response(
                                request.id,
                                ErrorCode::InvalidParams,
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.definition(&params),
                )));
            }
            DocumentHighlightRequest::METHOD => {
                let params =
                    match serde_json::from_value::<DocumentHighlightParams>(request.params.clone())
                    {
                        Ok(params) => params,
                        Err(error) => {
                            output.push(error_response(
                                request.id,
                                ErrorCode::InvalidParams,
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.document_highlight(&params),
                )));
            }
            InlayHintRequest::METHOD => {
                let params = match serde_json::from_value::<InlayHintParams>(request.params.clone())
                {
                    Ok(params) => params,
                    Err(error) => {
                        output.push(error_response(
                            request.id,
                            ErrorCode::InvalidParams,
                            error.to_string(),
                        ));
                        return;
                    }
                };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.inlay_hints(&params),
                )));
            }
            HoverRequest::METHOD => {
                let params = match serde_json::from_value::<HoverParams>(request.params.clone()) {
                    Ok(params) => params,
                    Err(error) => {
                        output.push(error_response(
                            request.id,
                            ErrorCode::InvalidParams,
                            error.to_string(),
                        ));
                        return;
                    }
                };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.hover(&params),
                )));
            }
            SignatureHelpRequest::METHOD => {
                let params =
                    match serde_json::from_value::<SignatureHelpParams>(request.params.clone()) {
                        Ok(params) => params,
                        Err(error) => {
                            output.push(error_response(
                                request.id,
                                ErrorCode::InvalidParams,
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                output.push(Message::Response(Response::new_ok(
                    request.id,
                    self.signature_help(&params),
                )));
            }
            _ => output.push(error_response(
                request.id,
                ErrorCode::MethodNotFound,
                format!("Unsupported method: {}", request.method),
            )),
        }
    }

    fn handle_notification_to(
        &mut self,
        output: &mut Vec<Message>,
        notification: lsp_server::Notification,
    ) -> Result<(), serde_json::Error> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let params: DidOpenTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                push_log(output, "file opened!");
                let uri = params.text_document.uri;
                let text = params.text_document.text;
                self.documents.insert(uri.clone(), text.clone());
                validate_document_to(output, uri, text);
            }
            DidChangeTextDocument::METHOD => {
                let params: DidChangeTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                push_log(output, "file changed!");
                if let Some(content) = params.content_changes.into_iter().next() {
                    let uri = params.text_document.uri;
                    self.documents.insert(uri.clone(), content.text.clone());
                    validate_document_to(output, uri, content.text);
                }
            }
            DidCloseTextDocument::METHOD => {
                let params: DidCloseTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                self.documents.remove(&params.text_document.uri);
                push_diagnostics(output, params.text_document.uri, Vec::new());
            }
            _ => {}
        }
        Ok(())
    }

    fn formatting(&self, params: &DocumentFormattingParams) -> Option<Vec<TextEdit>> {
        let text = self.documents.get(&params.text_document.uri)?;
        let new_text = formatting::format(text)?;
        if new_text == *text {
            return Some(Vec::new());
        }
        Some(vec![TextEdit {
            range: Range {
                start: Position::new(0, 0),
                end: document_end(text),
            },
            new_text,
        }])
    }

    fn folding_range(&self, params: &FoldingRangeParams) -> Option<Vec<FoldingRange>> {
        let text = self.documents.get(&params.text_document.uri)?;
        let tree = parser::parse(text)?;
        Some(parser::collect_folding_ranges(tree.root_node(), text))
    }

    fn document_symbols(&self, params: &DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
        let text = self.documents.get(&params.text_document.uri)?;
        let tree = parser::parse(text)?;
        let symbols = parser::collect_document_symbols(tree.root_node(), text);
        Some(DocumentSymbolResponse::Nested(symbols))
    }

    fn definition(&self, params: &GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let text = self.documents.get(uri)?;
        let tree = parser::parse(text)?;
        let locations = locals::find_definitions(
            tree.root_node(),
            text,
            params.text_document_position_params.position,
            uri,
        );
        if locations.is_empty()
            && self.remote_links
            && let Some(location) = remote_navigation::find_definition(
                tree.root_node(),
                text,
                params.text_document_position_params.position,
                uri,
            )
        {
            return Some(GotoDefinitionResponse::Scalar(location));
        }
        match locations.as_slice() {
            [] => None,
            [location] => Some(GotoDefinitionResponse::Scalar(location.clone())),
            _ => Some(GotoDefinitionResponse::Array(locations)),
        }
    }

    fn document_highlight(
        &self,
        params: &DocumentHighlightParams,
    ) -> Option<Vec<DocumentHighlight>> {
        let text = self
            .documents
            .get(&params.text_document_position_params.text_document.uri)?;
        let tree = parser::parse(text)?;
        Some(locals::find_highlights(
            tree.root_node(),
            text,
            params.text_document_position_params.position,
        ))
    }

    fn inlay_hints(&self, params: &InlayHintParams) -> Option<Vec<InlayHint>> {
        let text = self.documents.get(&params.text_document.uri)?;
        let tree = parser::parse(text)?;
        Some(inlay_hints::collect(tree.root_node(), text, params.range))
    }

    fn hover(&self, params: &HoverParams) -> Option<Hover> {
        let position = &params.text_document_position_params;
        let text = self.documents.get(&position.text_document.uri)?;
        let tree = parser::parse(text)?;
        builtins::hover(tree.root_node(), text, position.position)
    }

    fn hover_definition(&self, params: &HoverParams) -> Option<lsp_types::Location> {
        let position = &params.text_document_position_params;
        let text = self.documents.get(&position.text_document.uri)?;
        let tree = parser::parse(text)?;
        self.remote_links.then(|| {
            remote_navigation::find_hover_definition(
                tree.root_node(),
                text,
                position.position,
                &position.text_document.uri,
            )
        })?
    }

    fn signature_help(&self, params: &SignatureHelpParams) -> Option<SignatureHelp> {
        let position = &params.text_document_position_params;
        let text = self.documents.get(&position.text_document.uri)?;
        let tree = parser::parse(text)?;
        builtins::signature_help(tree.root_node(), text, position.position)
    }
}

fn supports_show_document(params: &serde_json::Value) -> bool {
    params
        .pointer("/capabilities/window/showDocument/support")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn supports_remote_documents(params: &serde_json::Value) -> bool {
    params
        .pointer("/initializationOptions/lambdamoo/remoteDocuments")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
}

fn push_diagnostics(output: &mut Vec<Message>, uri: Uri, diagnostics: Vec<lsp_types::Diagnostic>) {
    output.push(
        lsp_server::Notification::new(
            PublishDiagnostics::METHOD.to_owned(),
            PublishDiagnosticsParams::new(uri, diagnostics, None),
        )
        .into(),
    );
}

fn validate_document_to(output: &mut Vec<Message>, uri: Uri, text: String) {
    push_log(output, format!("Validating {}", uri.as_str()));
    let diagnostics = analysis::diagnostics(&text);
    if diagnostics.is_empty() {
        push_log(output, "Parse successful");
    } else {
        push_log(output, "Syntax errors detected");
    }
    push_diagnostics(output, uri, diagnostics);
}

fn document_end(text: &str) -> Position {
    let line = text.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let last_line = text.rsplit('\n').next().unwrap_or_default();
    let character = last_line.encode_utf16().count() as u32;
    Position::new(line, character)
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use lsp_server::{Message, Notification, Request, RequestId};
    use lsp_types::notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Exit, Initialized,
        Notification as _, PublishDiagnostics,
    };
    use lsp_types::request::{
        DocumentHighlightRequest, DocumentSymbolRequest, FoldingRangeRequest, Formatting,
        GotoDefinition, HoverRequest, InlayHintRequest, Request as _, SemanticTokensFullRequest,
        Shutdown, SignatureHelpRequest,
    };
    use lsp_types::{
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, DocumentHighlightParams, DocumentSymbolParams,
        FoldingRangeParams, FormattingOptions, GotoDefinitionParams, InitializeParams,
        InitializedParams, InlayHintParams, NumberOrString, Position, PublishDiagnosticsParams,
        Range, SemanticTokensParams, TextDocumentContentChangeEvent, TextDocumentIdentifier,
        TextDocumentItem, TextDocumentPositionParams, Uri, VersionedTextDocumentIdentifier,
        WorkDoneProgressParams,
    };

    use super::{CANONICALIZE_DOCUMENT_METHOD, READ_DOCUMENT_METHOD, Session, run};
    use lsp_server::Response;

    #[test]
    fn headerless_session_initializes_and_replies() {
        let mut session = Session::new();
        let initialize: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }))
        .unwrap();
        let output = session.handle(initialize);
        assert!(!session.server.remote_links);
        assert_eq!(output.len(), 1);
        let response = serde_json::to_value(&output[0]).unwrap();
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["capabilities"]["textDocumentSync"], 1);

        let unsupported: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "moo/unknown",
            "params": null
        }))
        .unwrap();
        let output = session.handle(unsupported);
        let response = serde_json::to_value(&output[0]).unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn headerless_session_records_show_document_support() {
        let mut session = Session::new();
        let initialize: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {
                    "window": { "showDocument": { "support": true } }
                }
            }
        }))
        .unwrap();
        session.handle(initialize);
        assert!(session.server.remote_links);
    }

    fn remote_session() -> Session {
        let mut session = Session::new();
        let initialize: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {
                    "window": { "showDocument": { "support": true } }
                },
                "initializationOptions": {
                    "lambdamoo": { "remoteDocuments": 1 }
                }
            }
        }))
        .unwrap();
        let response = serde_json::to_value(&session.handle(initialize)[0]).unwrap();
        assert_eq!(
            response["result"]["capabilities"]["experimental"]["lambdamoo"]["remoteDocuments"],
            1
        );
        session
    }

    fn read_response(request: &Message, text: &str) -> Message {
        let Message::Request(request) = request else {
            panic!("expected a remote document request");
        };
        assert_eq!(request.method, READ_DOCUMENT_METHOD);
        Message::Response(Response::new_ok(
            request.id.clone(),
            serde_json::json!({ "text": text }),
        ))
    }

    #[test]
    fn resolves_remote_definitions_and_builds_method_hover() {
        let mut session = remote_session();
        let remote_uri: Uri = "moo://waterpoint/object/42/verb/current".parse().unwrap();
        let opened = Message::Notification(lsp_server::Notification::new(
            DidOpenTextDocument::METHOD.to_owned(),
            DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: remote_uri.clone(),
                    language_id: "lambdamoo".to_owned(),
                    version: 1,
                    text: "$string_utils:explode();".to_owned(),
                },
            },
        ));
        let output = session.handle(opened);
        let open_read = output
            .iter()
            .find(|message| matches!(message, Message::Request(_)))
            .unwrap();
        assert!(session.handle(read_response(open_read, "#42")).is_empty());

        let definition: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": remote_uri },
                "position": { "line": 0, "character": 15 }
            }
        }))
        .unwrap();
        let output = session.handle(definition);
        let resolution = &output[0];
        assert_eq!(
            serde_json::to_value(resolution).unwrap()["params"]["uri"],
            "moo://waterpoint/object/0/property/string_utils/object/resolve/verb/explode/defined-on"
        );
        let output = session.handle(read_response(resolution, "#18\n"));
        let definition = serde_json::to_value(&output[0]).unwrap();
        assert_eq!(
            definition["result"]["uri"],
            "moo://waterpoint/object/18/verb/explode"
        );

        let hover: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": remote_uri },
                "position": { "line": 0, "character": 15 }
            }
        }))
        .unwrap();
        let resolution = session.handle(hover).remove(0);
        let document_request = session.handle(read_response(&resolution, "#18")).remove(0);
        assert_eq!(
            serde_json::to_value(&document_request).unwrap()["params"]["uri"],
            "moo://waterpoint/object/18/verb/explode"
        );
        let output = session.handle(read_response(
            &document_request,
            "\"Split a string.\";\n{subject, delim} = args;\nreturn {};",
        ));
        let hover = serde_json::to_value(&output[0]).unwrap();
        let value = hover["result"]["contents"]["value"].as_str().unwrap();
        assert!(value.contains("{subject, delim} = args;"));
        assert!(value.contains("Split a string."));
        assert!(value.contains("moo://waterpoint/object/18/verb/explode"));
    }

    #[test]
    fn canonicalizes_an_open_owned_verb() {
        let mut session = remote_session();
        let opened: Message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "moo://waterpoint/owned/525/verb/check_authorization",
                    "languageId": "lambdamoo",
                    "version": 1,
                    "text": "return 1;"
                }
            }
        }))
        .unwrap();
        let output = session.handle(opened);
        let request = output
            .iter()
            .find(|message| matches!(message, Message::Request(_)))
            .unwrap();
        assert_eq!(
            serde_json::to_value(request).unwrap()["params"]["uri"],
            "moo://waterpoint/object/525/resolve/verb/check_authorization/defined-on"
        );
        let output = session.handle(read_response(request, "#525"));
        let notification = serde_json::to_value(&output[0]).unwrap();
        assert_eq!(notification["method"], CANONICALIZE_DOCUMENT_METHOD);
        assert_eq!(
            notification["params"]["canonicalUri"],
            "moo://waterpoint/object/525/verb/check_authorization"
        );
    }

    struct TestServer {
        client: lsp_server::Connection,
        thread: thread::JoinHandle<()>,
        next_id: i32,
    }

    impl TestServer {
        fn start() -> Self {
            Self::start_with_show_document(false)
        }

        fn start_with_show_document(support: bool) -> Self {
            let (server, client) = lsp_server::Connection::memory();
            let thread = thread::spawn(move || run(server).unwrap());
            let mut test_server = Self {
                client,
                thread,
                next_id: 1,
            };

            let mut initialize_params = serde_json::to_value(InitializeParams::default()).unwrap();
            initialize_params["capabilities"]["window"]["showDocument"]["support"] =
                serde_json::json!(support);
            let initialize = test_server.request("initialize", initialize_params);
            let result = initialize.response_result.unwrap();
            assert_eq!(result["capabilities"]["textDocumentSync"], 1);
            assert_eq!(result["capabilities"]["documentFormattingProvider"], true);
            assert!(result["capabilities"]["semanticTokensProvider"].is_object());
            assert_eq!(result["capabilities"]["hoverProvider"], true);
            assert_eq!(result["capabilities"]["inlayHintProvider"], true);
            assert_eq!(
                result["capabilities"]["signatureHelpProvider"]["triggerCharacters"],
                serde_json::json!(["(", ","])
            );
            test_server.notify::<Initialized>(InitializedParams {});
            test_server
        }

        fn request(&mut self, method: &str, params: impl serde::Serialize) -> lsp_server::Response {
            let id = RequestId::from(self.next_id);
            self.next_id += 1;
            self.client
                .sender
                .send(Request::new(id.clone(), method.to_owned(), params).into())
                .unwrap();
            loop {
                match self
                    .client
                    .receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                {
                    Message::Response(response) if response.id == id => return response,
                    _ => {}
                }
            }
        }

        fn notify<N>(&self, params: N::Params)
        where
            N: lsp_types::notification::Notification,
        {
            self.client
                .sender
                .send(Notification::new(N::METHOD.to_owned(), params).into())
                .unwrap();
        }

        fn next_diagnostics(&self) -> PublishDiagnosticsParams {
            loop {
                match self
                    .client
                    .receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                {
                    Message::Notification(notification)
                        if notification.method == PublishDiagnostics::METHOD =>
                    {
                        return serde_json::from_value(notification.params).unwrap();
                    }
                    _ => {}
                }
            }
        }

        fn stop(self) {
            let id = RequestId::from(self.next_id);
            self.client
                .sender
                .send(Request::new(id.clone(), Shutdown::METHOD.to_owned(), ()).into())
                .unwrap();
            loop {
                if matches!(
                    self.client.receiver.recv().unwrap(),
                    Message::Response(response) if response.id == id
                ) {
                    break;
                }
            }
            self.notify::<Exit>(());
            self.thread.join().unwrap();
        }
    }

    fn uri() -> Uri {
        "file:///test.moo".parse().unwrap()
    }

    #[test]
    fn serves_diagnostics_tokens_formatting_and_close() {
        let mut server = TestServer::start();
        server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri(),
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "if (player)\nnotify(player, \"hi\");\nendif\n".to_owned(),
            },
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());

        server.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "if (x\n".to_owned(),
            }],
        });
        assert!(!server.next_diagnostics().diagnostics.is_empty());

        server.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri(),
                version: 3,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "if (player)\nnotify(player, \"hi\");\nendif\n".to_owned(),
            }],
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());

        let tokens = server.request(
            SemanticTokensFullRequest::METHOD,
            SemanticTokensParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            },
        );
        assert!(
            tokens.response_result.unwrap()["data"]
                .as_array()
                .is_some_and(|v| !v.is_empty())
        );

        let hover = server.request(
            HoverRequest::METHOD,
            serde_json::json!({
                "textDocument": { "uri": uri() },
                "position": { "line": 1, "character": 2 }
            }),
        );
        assert!(
            hover.response_result.unwrap()["contents"]["value"]
                .as_str()
                .is_some_and(|value| value.contains("notify(arg1: OBJ, arg2: STR"))
        );

        let signature = server.request(
            SignatureHelpRequest::METHOD,
            serde_json::json!({
                "textDocument": { "uri": uri() },
                "position": { "line": 1, "character": 17 }
            }),
        );
        let signature = signature.response_result.unwrap();
        assert_eq!(signature["activeParameter"], 1);
        assert!(
            signature["signatures"][0]["label"]
                .as_str()
                .is_some_and(|value| value.starts_with("notify("))
        );

        let formatting = server.request(
            Formatting::METHOD,
            DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                options: FormattingOptions {
                    tab_size: 2,
                    insert_spaces: true,
                    ..Default::default()
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        );
        let edits = formatting
            .response_result
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0]["range"]["start"],
            serde_json::json!(Position::new(0, 0))
        );

        let folds = server.request(
            FoldingRangeRequest::METHOD,
            FoldingRangeParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            },
        );
        let fold_list = folds.response_result.unwrap().as_array().unwrap().clone();
        assert_eq!(fold_list.len(), 1);

        let doc_symbols = server.request(
            DocumentSymbolRequest::METHOD,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            },
        );
        let symbol_list = doc_symbols
            .response_result
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert!(!symbol_list.is_empty());

        let def_req = server.request(
            GotoDefinition::METHOD,
            GotoDefinitionParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri() },
                    position: Position::new(1, 4),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            },
        );
        assert!(def_req.response_result.is_ok());

        let hl_req = server.request(
            DocumentHighlightRequest::METHOD,
            DocumentHighlightParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri() },
                    position: Position::new(1, 4),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            },
        );
        assert!(hl_req.response_result.is_ok());

        server.notify::<DidCloseTextDocument>(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri() },
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());
        server.stop();
    }

    #[test]
    fn serves_builtin_argument_inlay_hints() {
        let mut server = TestServer::start();
        server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri(),
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "is_member(player, args); notify(player, \"hi\");".to_owned(),
            },
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());

        let response = server.request(
            InlayHintRequest::METHOD,
            InlayHintParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                range: Range::new(Position::new(0, 0), Position::new(1, 0)),
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        );
        let hints = response.response_result.unwrap();
        assert_eq!(hints.as_array().unwrap().len(), 2);
        assert_eq!(hints[0]["label"], "value:");
        assert_eq!(hints[0]["kind"], 2);
        assert_eq!(hints[0]["paddingRight"], true);
        assert_eq!(hints[1]["label"], "list:");

        server.stop();
    }

    #[test]
    fn reports_invalid_identifier_without_tree_sitter_error() {
        let server = TestServer::start();
        server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri(),
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "notify(if, \"hi\"); result = E_NONE;".to_owned(),
            },
        });

        let diagnostics = server.next_diagnostics().diagnostics;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code.as_ref().unwrap(),
            &NumberOrString::String("invalid-identifier".to_owned())
        );
        assert_eq!(
            diagnostics[0].range,
            Range::new(Position::new(0, 7), Position::new(0, 9))
        );

        server.stop();
    }

    #[test]
    fn reports_unbound_locals_and_returns_reaching_definitions() {
        let mut server = TestServer::start();
        server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri(),
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "if (player)\n  value = 1;\nelse\n  value = 2;\nendif\nreturn value;\n"
                    .to_owned(),
            },
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());

        let definitions = server.request(
            GotoDefinition::METHOD,
            GotoDefinitionParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri() },
                    position: Position::new(5, 7),
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            },
        );
        assert_eq!(
            definitions
                .response_result
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );

        server.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "return missing;\n".to_owned(),
            }],
        });
        let diagnostics = server.next_diagnostics().diagnostics;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("unbound-local".to_owned()))
        );
        assert_eq!(
            diagnostics[0].range,
            Range::new(Position::new(0, 7), Position::new(0, 14))
        );
        server.stop();
    }

    #[test]
    fn gates_remote_verb_definitions_on_show_document_support() {
        fn request_remote_definition(server: &mut TestServer, uri: &Uri) -> serde_json::Value {
            server
                .request(
                    GotoDefinition::METHOD,
                    GotoDefinitionParams {
                        text_document_position_params: TextDocumentPositionParams {
                            text_document: TextDocumentIdentifier { uri: uri.clone() },
                            position: Position::new(0, 5),
                        },
                        work_done_progress_params: WorkDoneProgressParams::default(),
                        partial_result_params: Default::default(),
                    },
                )
                .response_result
                .unwrap()
        }

        let remote_uri: Uri = "moo://codepoint/object/42/verb/current".parse().unwrap();
        let document = |uri: Uri| DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "#123:foo();".to_owned(),
            },
        };

        let mut unsupported = TestServer::start();
        unsupported.notify::<DidOpenTextDocument>(document(remote_uri.clone()));
        assert!(unsupported.next_diagnostics().diagnostics.is_empty());
        assert!(request_remote_definition(&mut unsupported, &remote_uri).is_null());
        unsupported.stop();

        let mut supported = TestServer::start_with_show_document(true);
        supported.notify::<DidOpenTextDocument>(document(remote_uri.clone()));
        assert!(supported.next_diagnostics().diagnostics.is_empty());
        let location = request_remote_definition(&mut supported, &remote_uri);
        assert_eq!(location["uri"], "moo://codepoint/object/123/verb/foo");
        assert_eq!(
            location["range"]["start"],
            serde_json::json!(Position::new(0, 0))
        );
        supported.stop();
    }

    #[test]
    fn reports_protocol_errors_without_stopping() {
        let mut server = TestServer::start();
        let invalid = server.request(SemanticTokensFullRequest::METHOD, serde_json::json!({}));
        assert_eq!(invalid.response_result.unwrap_err().code, -32602);

        let unsupported = server.request("moo/unknown", ());
        assert_eq!(unsupported.response_result.unwrap_err().code, -32601);
        server.stop();
    }

    #[test]
    fn handles_formatting_with_invalid_documents_open() {
        let mut server = TestServer::start();
        let invalid_uri: Uri = "file:///invalid.moo".parse().unwrap();
        let valid_uri: Uri = "file:///valid.moo".parse().unwrap();

        server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: invalid_uri.clone(),
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "if (x\n  b = 1;\n".to_owned(),
            },
        });
        let diags = server.next_diagnostics();
        assert!(!diags.diagnostics.is_empty());

        server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: valid_uri.clone(),
                language_id: "lambdamoo".to_owned(),
                version: 1,
                text: "if (player)\n  b = 1;\nendif;\n".to_owned(),
            },
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());

        let format_invalid = server.request(
            Formatting::METHOD,
            DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri: invalid_uri },
                options: FormattingOptions {
                    tab_size: 2,
                    insert_spaces: true,
                    ..Default::default()
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        );
        assert!(format_invalid.response_result.unwrap().is_null());

        let format_valid = server.request(
            Formatting::METHOD,
            DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri: valid_uri },
                options: FormattingOptions {
                    tab_size: 2,
                    insert_spaces: true,
                    ..Default::default()
                },
                work_done_progress_params: WorkDoneProgressParams::default(),
            },
        );
        assert!(format_valid.response_result.unwrap().is_array());

        server.stop();
    }
}
