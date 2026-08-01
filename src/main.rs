// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT


pub mod line_index;
pub mod parser;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "file opened!")
            .await;
        self.validate_document(params.text_document.uri, params.text_document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "file changed!")
            .await;
        if let Some(content) = params.content_changes.into_iter().next() {
            self.validate_document(params.text_document.uri, content.text).await;
        }
    }
}

impl Backend {
    async fn validate_document(&self, uri: Url, text: String) {
        self.client.log_message(MessageType::INFO, format!("Validating {}", uri)).await;

        let line_index = line_index::LineIndex::new(&text);
        let mut diagnostics = Vec::new();

        if let Some(tree) = parser::parse(&text) {
            let root = tree.root_node();
            if root.has_error() {
                self.client.log_message(MessageType::INFO, "Syntax errors detected").await;
                parser::collect_diagnostics(root, &line_index, &mut diagnostics);
            } else {
                self.client.log_message(MessageType::INFO, "Parse successful").await;
            }
        }

        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend { client });
    Server::new(stdin, stdout, socket).serve(service).await;
}
