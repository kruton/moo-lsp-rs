// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT


use tower_lsp::lsp_types::*;
use tree_sitter::{Node, Parser, Tree};
use crate::line_index::LineIndex;

pub fn create_parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_lambdamoo::LANGUAGE.into())
        .expect("Error loading LambdaMOO parser");
    parser
}

pub fn parse(text: &str) -> Option<Tree> {
    let mut parser = create_parser();
    parser.parse(text, None)
}

pub fn collect_diagnostics(node: Node, line_index: &LineIndex, diagnostics: &mut Vec<Diagnostic>) {
    if node.is_error() || node.is_missing() {
        let start_byte = node.start_byte();
        let end_byte = node.end_byte();

        let (start_line, start_col) = line_index.line_col(start_byte);
        let (end_line, end_col) = if start_byte == end_byte {
            line_index.line_col(start_byte + 1)
        } else {
            line_index.line_col(end_byte)
        };

        let message = if node.is_missing() {
            format!("Missing expected syntax: {}", node.kind())
        } else {
            "Syntax error".to_string()
        };

        diagnostics.push(Diagnostic {
            range: Range {
                start: Position {
                    line: start_line as u32,
                    character: start_col as u32,
                },
                end: Position {
                    line: end_line as u32,
                    character: end_col as u32,
                },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            code: None,
            code_description: None,
            source: Some("moo-lsp-rs".to_string()),
            message,
            related_information: None,
            tags: None,
            data: None,
        });
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.has_error() {
            collect_diagnostics(child, line_index, diagnostics);
        }
    }
}
