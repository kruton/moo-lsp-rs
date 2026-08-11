// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use crate::{line_index::LineIndex, parser};
use lsp_types::Diagnostic;

/// Analyze one complete LambdaMOO document.
///
/// This is the shared validation entry point for the language server and the
/// command-line checker.
pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(tree) = parser::parse(text) {
        parser::collect_diagnostics(
            tree.root_node(),
            &LineIndex::new(text),
            text,
            &mut diagnostics,
        );
    }
    diagnostics
}
