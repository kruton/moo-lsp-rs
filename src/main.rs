// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT


pub mod line_index;
pub mod parser;
pub mod semantic_tokens;

use std::collections::HashMap;
use std::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
    documents: RwLock<HashMap<Url, String>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: semantic_tokens::legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..Default::default()
                        },
                    ),
                ),
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
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.documents
            .write()
            .unwrap()
            .insert(uri.clone(), text.clone());
        self.validate_document(uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "file changed!")
            .await;
        if let Some(content) = params.content_changes.into_iter().next() {
            let uri = params.text_document.uri;
            self.documents
                .write()
                .unwrap()
                .insert(uri.clone(), content.text.clone());
            self.validate_document(uri, content.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .unwrap()
            .remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let tokens = self
            .documents
            .read()
            .unwrap()
            .get(&params.text_document.uri)
            .map(|text| semantic_tokens::collect(text));

        Ok(tokens.map(|data| {
            SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data,
            })
        }))
    }
}

impl Backend {
    async fn validate_document(&self, uri: Url, text: String) {
        self.client
            .log_message(MessageType::INFO, format!("Validating {}", uri))
            .await;

        let line_index = line_index::LineIndex::new(&text);
        let mut diagnostics = Vec::new();

        if let Some(tree) = parser::parse(&text) {
            let root = tree.root_node();
            if root.has_error() {
                self.client
                    .log_message(MessageType::INFO, "Syntax errors detected")
                    .await;
                parser::collect_diagnostics(root, &line_index, &mut diagnostics);
            } else {
                self.client
                    .log_message(MessageType::INFO, "Parse successful")
                    .await;
            }
        }

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: RwLock::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
