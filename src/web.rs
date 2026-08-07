// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use lsp_server::Message;
use wasm_bindgen::prelude::*;

use crate::server::Session;

/// A headerless JSON-RPC language-server session for use in a Web Worker.
#[wasm_bindgen]
pub struct BrowserServer {
    session: Session,
}

#[wasm_bindgen]
impl BrowserServer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            session: Session::new(),
        }
    }

    /// Process one complete JSON-RPC message and return a JSON array containing
    /// every response and server notification produced by it.
    pub fn handle_message(&mut self, message: &str) -> Result<String, JsValue> {
        let message: Message = serde_json::from_str(message)
            .map_err(|error| JsValue::from_str(&format!("invalid JSON-RPC message: {error}")))?;
        serde_json::to_string(&self.session.handle(message))
            .map_err(|error| JsValue::from_str(&format!("could not serialize response: {error}")))
    }
}

impl Default for BrowserServer {
    fn default() -> Self {
        Self::new()
    }
}
