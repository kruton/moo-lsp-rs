// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::error::Error;

use crate::{formatting, line_index::LineIndex, parser, semantic_tokens};
use lsp_server::{Connection, ErrorCode, Message, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, LogMessage, Notification,
    PublishDiagnostics,
};
use lsp_types::request::{
    FoldingRangeRequest, Formatting, Request as LspRequest, SemanticTokensFullRequest,
};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, FoldingRange, FoldingRangeParams, FoldingRangeProviderCapability,
    LogMessageParams, MessageType, OneOf, Position, PublishDiagnosticsParams, Range,
    SemanticTokens, SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri,
};

type ServerResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub fn run(connection: Connection) -> ServerResult<()> {
    let capabilities = serde_json::to_value(server_capabilities())?;
    connection.initialize(capabilities)?;
    log_message(&connection, "server initialized!")?;

    let mut server = Server::default();
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    break;
                }
                server.handle_request(&connection, request)?;
            }
            Message::Notification(notification) => {
                let method = notification.method.clone();
                if let Err(error) = server.handle_notification(&connection, notification) {
                    log_message(
                        &connection,
                        format!("Invalid {method} notification: {error}"),
                    )?;
                }
            }
            Message::Response(_) => {}
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
            Message::Request(request) => self.server.handle_request_to(&mut output, request),
            Message::Notification(notification) if notification.method == "initialized" => {
                push_log(&mut output, "server initialized!");
            }
            Message::Notification(notification) if notification.method == "exit" => {}
            Message::Notification(notification) if self.initialized && !self.shutdown_requested => {
                let method = notification.method.clone();
                if let Err(error) = self
                    .server
                    .handle_notification_to(&mut output, notification)
                {
                    push_log(
                        &mut output,
                        format!("Invalid {method} notification: {error}"),
                    );
                }
            }
            Message::Notification(_) | Message::Response(_) => {}
        }
        output
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
        ..Default::default()
    }
}

#[derive(Default)]
struct Server {
    documents: HashMap<Uri, String>,
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

    fn handle_request(&self, connection: &Connection, request: Request) -> ServerResult<()> {
        match request.method.as_str() {
            SemanticTokensFullRequest::METHOD => {
                let Some(params) = request_params::<SemanticTokensParams>(connection, &request)?
                else {
                    return Ok(());
                };
                let result = self.documents.get(&params.text_document.uri).map(|text| {
                    SemanticTokensResult::Tokens(SemanticTokens {
                        result_id: None,
                        data: semantic_tokens::collect(text),
                    })
                });
                send_ok(connection, request.id, result)?;
            }
            Formatting::METHOD => {
                let Some(params) =
                    request_params::<DocumentFormattingParams>(connection, &request)?
                else {
                    return Ok(());
                };
                let result = self.formatting(&params);
                send_ok(connection, request.id, result)?;
            }
            FoldingRangeRequest::METHOD => {
                let Some(params) = request_params::<FoldingRangeParams>(connection, &request)?
                else {
                    return Ok(());
                };
                let result = self.folding_range(&params);
                send_ok(connection, request.id, result)?;
            }
            _ => {
                send_error(
                    connection,
                    request.id,
                    ErrorCode::MethodNotFound,
                    format!("Unsupported method: {}", request.method),
                )?;
            }
        }
        Ok(())
    }

    fn handle_notification(
        &mut self,
        connection: &Connection,
        notification: lsp_server::Notification,
    ) -> ServerResult<()> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let params: DidOpenTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                log_message(connection, "file opened!")?;
                let uri = params.text_document.uri;
                let text = params.text_document.text;
                self.documents.insert(uri.clone(), text.clone());
                validate_document(connection, uri, text)?;
            }
            DidChangeTextDocument::METHOD => {
                let params: DidChangeTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                log_message(connection, "file changed!")?;
                if let Some(content) = params.content_changes.into_iter().next() {
                    let uri = params.text_document.uri;
                    self.documents.insert(uri.clone(), content.text.clone());
                    validate_document(connection, uri, content.text)?;
                }
            }
            DidCloseTextDocument::METHOD => {
                let params: DidCloseTextDocumentParams =
                    serde_json::from_value(notification.params)?;
                self.documents.remove(&params.text_document.uri);
                publish_diagnostics(connection, params.text_document.uri, Vec::new())?;
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
    let line_index = LineIndex::new(&text);
    let mut diagnostics = Vec::new();
    if let Some(tree) = parser::parse(&text) {
        let root = tree.root_node();
        if root.has_error() {
            push_log(output, "Syntax errors detected");
            parser::collect_diagnostics(root, &line_index, &text, &mut diagnostics);
        } else {
            push_log(output, "Parse successful");
        }
    }
    push_diagnostics(output, uri, diagnostics);
}

fn request_params<T>(connection: &Connection, request: &Request) -> ServerResult<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    match serde_json::from_value(request.params.clone()) {
        Ok(params) => Ok(Some(params)),
        Err(error) => {
            send_error(
                connection,
                request.id.clone(),
                ErrorCode::InvalidParams,
                error.to_string(),
            )?;
            Ok(None)
        }
    }
}

fn send_ok(
    connection: &Connection,
    id: RequestId,
    result: impl serde::Serialize,
) -> ServerResult<()> {
    connection
        .sender
        .send(Response::new_ok(id, result).into())?;
    Ok(())
}

fn send_error(
    connection: &Connection,
    id: RequestId,
    code: ErrorCode,
    message: String,
) -> ServerResult<()> {
    connection
        .sender
        .send(Response::new_err(id, code as i32, message).into())?;
    Ok(())
}

fn log_message(connection: &Connection, message: impl Into<String>) -> ServerResult<()> {
    send_notification::<LogMessage>(
        connection,
        LogMessageParams {
            typ: MessageType::INFO,
            message: message.into(),
        },
    )
}

fn publish_diagnostics(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<lsp_types::Diagnostic>,
) -> ServerResult<()> {
    send_notification::<PublishDiagnostics>(
        connection,
        PublishDiagnosticsParams::new(uri, diagnostics, None),
    )
}

fn send_notification<N>(connection: &Connection, params: N::Params) -> ServerResult<()>
where
    N: Notification,
{
    connection
        .sender
        .send(lsp_server::Notification::new(N::METHOD.to_owned(), params).into())?;
    Ok(())
}

fn validate_document(connection: &Connection, uri: Uri, text: String) -> ServerResult<()> {
    log_message(connection, format!("Validating {}", uri.as_str()))?;

    let line_index = LineIndex::new(&text);
    let mut diagnostics = Vec::new();
    if let Some(tree) = parser::parse(&text) {
        let root = tree.root_node();
        if root.has_error() {
            log_message(connection, "Syntax errors detected")?;
            parser::collect_diagnostics(root, &line_index, &text, &mut diagnostics);
        } else {
            log_message(connection, "Parse successful")?;
        }
    }

    publish_diagnostics(connection, uri, diagnostics)
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
        FoldingRangeRequest, Formatting, Request as _, SemanticTokensFullRequest, Shutdown,
    };
    use lsp_types::{
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, FoldingRangeParams, FormattingOptions, InitializeParams,
        InitializedParams, Position, PublishDiagnosticsParams, SemanticTokensParams,
        TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem, Uri,
        VersionedTextDocumentIdentifier, WorkDoneProgressParams,
    };

    use super::{Session, run};

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

    struct TestServer {
        client: lsp_server::Connection,
        thread: thread::JoinHandle<()>,
        next_id: i32,
    }

    impl TestServer {
        fn start() -> Self {
            let (server, client) = lsp_server::Connection::memory();
            let thread = thread::spawn(move || run(server).unwrap());
            let mut test_server = Self {
                client,
                thread,
                next_id: 1,
            };

            let initialize = test_server.request("initialize", InitializeParams::default());
            let result = initialize.response_result.unwrap();
            assert_eq!(result["capabilities"]["textDocumentSync"], 1);
            assert_eq!(result["capabilities"]["documentFormattingProvider"], true);
            assert!(result["capabilities"]["semanticTokensProvider"].is_object());
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
                text: "if (x)\nnotify(player);\nendif\n".to_owned(),
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
                text: "if (x)\nnotify(player);\nendif\n".to_owned(),
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

        server.notify::<DidCloseTextDocument>(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri() },
        });
        assert!(server.next_diagnostics().diagnostics.is_empty());
        server.stop();
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
}
