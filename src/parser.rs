// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use crate::line_index::{LineIndex, safe_slice};
use lsp_types::*;
use tree_sitter::{Node, Parser, Tree};

pub fn create_parser() -> Parser {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    crate::tree_sitter_allocator::install();

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

use std::sync::LazyLock;
use tree_sitter::{Query, QueryCursor, StreamingIterator};

static ERROR_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    Query::new(&language, tree_sitter_lambdamoo::ERRORS_QUERY)
        .expect("Failed to compile diagnostic Tree-sitter query")
});

static FOLD_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    Query::new(&language, tree_sitter_lambdamoo::FOLDS_QUERY)
        .expect("Failed to compile folding Tree-sitter query")
});

static TAGS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    Query::new(&language, tree_sitter_lambdamoo::TAGS_QUERY)
        .expect("Failed to compile tags Tree-sitter query")
});

pub fn collect_document_symbols(node: Node, text: &str) -> Vec<DocumentSymbol> {
    let query = &*TAGS_QUERY;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, node, text.as_bytes());
    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_node = cap.node;
            let start_pos = cap_node.start_position();
            let end_pos = cap_node.end_position();

            let range = Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: end_pos.row as u32,
                    character: end_pos.column as u32,
                },
            };

            let raw_text = safe_slice(text, cap_node.byte_range()).trim();
            if raw_text.is_empty() {
                continue;
            }

            let (name, kind) = match cap_node.kind() {
                "call_expression" => (raw_text.to_string(), SymbolKind::FUNCTION),
                "verb_call" => (raw_text.to_string(), SymbolKind::METHOD),
                "prop_access" => (raw_text.to_string(), SymbolKind::PROPERTY),
                "assignment" => (raw_text.to_string(), SymbolKind::VARIABLE),
                _ => (raw_text.to_string(), SymbolKind::VARIABLE),
            };

            #[allow(deprecated)]
            symbols.push(DocumentSymbol {
                name,
                detail: None,
                kind,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
    }

    symbols
}

pub fn collect_folding_ranges(node: Node, text: &str) -> Vec<FoldingRange> {
    let query = &*FOLD_QUERY;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, node, text.as_bytes());
    let mut ranges = Vec::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let start_pos = cap.node.start_position();
            let end_pos = cap.node.end_position();

            if start_pos.row < end_pos.row {
                ranges.push(FoldingRange {
                    start_line: start_pos.row as u32,
                    start_character: Some(start_pos.column as u32),
                    end_line: end_pos.row as u32,
                    end_character: Some(end_pos.column as u32),
                    kind: Some(FoldingRangeKind::Region),
                    collapsed_text: None,
                });
            }
        }
    }

    ranges
}

pub fn collect_diagnostics(
    node: Node,
    line_index: &LineIndex,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let query = &*ERROR_QUERY;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, node, text.as_bytes());

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_name = match query.capture_names().get(cap.index as usize) {
                Some(name) => *name,
                None => continue,
            };
            let cap_node = cap.node;

            let start_pos = cap_node.start_position();
            let end_pos = cap_node.end_position();

            let range = line_index.clamp_range(
                text,
                start_pos.row,
                start_pos.column,
                end_pos.row,
                end_pos.column,
            );

            let message = match cap_name {
                "missing_endif" => {
                    let parent = cap_node.parent().unwrap_or(cap_node);
                    let open_line = parent.start_position().row + 1;
                    if let Some(mismatched) = find_mismatched_end_token(parent, text, "endif") {
                        format!(
                            "Mismatched block terminator: found '{}', expected 'endif' for 'if' statement on line {}",
                            mismatched, open_line
                        )
                    } else {
                        format!(
                            "Unclosed 'if' statement (opened on line {}); expected matching 'endif'",
                            open_line
                        )
                    }
                }
                "missing_endfor" => {
                    let parent = cap_node.parent().unwrap_or(cap_node);
                    let open_line = parent.start_position().row + 1;
                    if let Some(mismatched) = find_mismatched_end_token(parent, text, "endfor") {
                        format!(
                            "Mismatched block terminator: found '{}', expected 'endfor' for 'for' loop on line {}",
                            mismatched, open_line
                        )
                    } else {
                        format!(
                            "Unclosed 'for' loop (opened on line {}); expected matching 'endfor'",
                            open_line
                        )
                    }
                }
                "missing_endwhile" => {
                    let parent = cap_node.parent().unwrap_or(cap_node);
                    let open_line = parent.start_position().row + 1;
                    if let Some(mismatched) = find_mismatched_end_token(parent, text, "endwhile") {
                        format!(
                            "Mismatched block terminator: found '{}', expected 'endwhile' for 'while' loop on line {}",
                            mismatched, open_line
                        )
                    } else {
                        format!(
                            "Unclosed 'while' loop (opened on line {}); expected matching 'endwhile'",
                            open_line
                        )
                    }
                }
                "missing_endfork" => {
                    let parent = cap_node.parent().unwrap_or(cap_node);
                    let open_line = parent.start_position().row + 1;
                    if let Some(mismatched) = find_mismatched_end_token(parent, text, "endfork") {
                        format!(
                            "Mismatched block terminator: found '{}', expected 'endfork' for 'fork' block on line {}",
                            mismatched, open_line
                        )
                    } else {
                        format!(
                            "Unclosed 'fork' block (opened on line {}); expected matching 'endfork'",
                            open_line
                        )
                    }
                }
                "missing_endtry" => {
                    let parent = cap_node.parent().unwrap_or(cap_node);
                    let open_line = parent.start_position().row + 1;
                    if let Some(mismatched) = find_mismatched_end_token(parent, text, "endtry") {
                        format!(
                            "Mismatched block terminator: found '{}', expected 'endtry' for 'try' block on line {}",
                            mismatched, open_line
                        )
                    } else {
                        format!(
                            "Unclosed 'try' block (opened on line {}); expected matching 'endtry'",
                            open_line
                        )
                    }
                }
                "missing_paren" => "Missing closing parenthesis ')'".to_string(),
                "missing_bracket" => "Missing closing bracket ']'".to_string(),
                "missing_brace" => "Missing closing brace '}'".to_string(),
                "missing_single_quote" => "Missing closing single quote '\''".to_string(),
                "missing_semicolon" => "Missing ';' at end of statement".to_string(),
                "error" => format_error_message(cap_node, text),
                _ => "Syntax error".to_string(),
            };

            // Avoid duplicate range diagnostics if same range and message already pushed
            if !diagnostics
                .iter()
                .any(|d| d.range == range && d.message == message)
            {
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: None,
                    code_description: None,
                    source: Some("moo-lsp-rs".to_string()),
                    message,
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }
    }

    // Also check for orphan control statements parsed as top-level identifiers
    collect_orphan_keywords(node, line_index, text, diagnostics);
}

fn collect_orphan_keywords(
    node: Node,
    line_index: &LineIndex,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if node.kind() == "expression_statement" {
        let stmt_text = safe_slice(text, node.byte_range())
            .trim()
            .trim_end_matches(';')
            .trim();
        match stmt_text {
            "endif" => {
                if find_parent_of_kind(node, &["if_statement"]).is_none() {
                    push_orphan_diagnostic(
                        node,
                        line_index,
                        text,
                        "Unmatched 'endif' without a corresponding 'if' statement",
                        diagnostics,
                    );
                }
            }
            "endfor" => {
                if find_parent_of_kind(node, &["for_statement"]).is_none() {
                    push_orphan_diagnostic(
                        node,
                        line_index,
                        text,
                        "Unmatched 'endfor' without a corresponding 'for' loop",
                        diagnostics,
                    );
                }
            }
            "endwhile" => {
                if find_parent_of_kind(node, &["while_statement"]).is_none() {
                    push_orphan_diagnostic(
                        node,
                        line_index,
                        text,
                        "Unmatched 'endwhile' without a corresponding 'while' loop",
                        diagnostics,
                    );
                }
            }
            "endfork" => {
                if find_parent_of_kind(node, &["fork_statement"]).is_none() {
                    push_orphan_diagnostic(
                        node,
                        line_index,
                        text,
                        "Unmatched 'endfork' without a corresponding 'fork' block",
                        diagnostics,
                    );
                }
            }
            "endtry" if find_parent_of_kind(node, &["try_statement"]).is_none() => {
                push_orphan_diagnostic(
                    node,
                    line_index,
                    text,
                    "Unmatched 'endtry' without a corresponding 'try' block",
                    diagnostics,
                );
            }
            _ => {}
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_orphan_keywords(child, line_index, text, diagnostics);
    }
}

fn push_orphan_diagnostic(
    node: Node,
    line_index: &LineIndex,
    text: &str,
    message: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let start_pos = node.start_position();
    let end_pos = node.end_position();

    let range = line_index.clamp_range(
        text,
        start_pos.row,
        start_pos.column,
        end_pos.row,
        end_pos.column,
    );

    if !diagnostics
        .iter()
        .any(|d| d.range == range && d.message == message)
    {
        diagnostics.push(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: None,
            code_description: None,
            source: Some("moo-lsp-rs".to_string()),
            message: message.to_string(),
            related_information: None,
            tags: None,
            data: None,
        });
    }
}

fn find_parent_of_kind<'a>(mut node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    while let Some(parent) = node.parent() {
        if kinds.contains(&parent.kind()) {
            return Some(parent);
        }
        node = parent;
    }
    None
}

fn find_mismatched_end_token(node: Node, text: &str, expected: &str) -> Option<&'static str> {
    const END_TOKENS: &[(&str, &str)] = &[
        ("endif", "endif"),
        ("endfor", "endfor"),
        ("endwhile", "endwhile"),
        ("endfork", "endfork"),
        ("endtry", "endtry"),
    ];

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_text = safe_slice(text, child.byte_range())
            .trim()
            .trim_end_matches(';')
            .trim();
        for &(token, name) in END_TOKENS {
            if child_text == token && token != expected {
                return Some(name);
            }
        }
        if let Some(found) = find_mismatched_end_token(child, text, expected) {
            return Some(found);
        }
    }
    None
}

fn format_error_message(node: Node, text: &str) -> String {
    let raw_slice = safe_slice(text, node.byte_range());
    let trimmed = raw_slice.trim();

    if trimmed == "endif" || trimmed.starts_with("endif;") {
        return "Unmatched 'endif' without a corresponding 'if' statement".to_string();
    }
    if trimmed == "endfor" || trimmed.starts_with("endfor;") {
        return "Unmatched 'endfor' without a corresponding 'for' loop".to_string();
    }
    if trimmed == "endwhile" || trimmed.starts_with("endwhile;") {
        return "Unmatched 'endwhile' without a corresponding 'while' loop".to_string();
    }
    if trimmed == "endfork" || trimmed.starts_with("endfork;") {
        return "Unmatched 'endfork' without a corresponding 'fork' block".to_string();
    }
    if trimmed == "endtry" || trimmed.starts_with("endtry;") {
        return "Unmatched 'endtry' without a corresponding 'try' block".to_string();
    }
    if trimmed == "else"
        || trimmed.starts_with("else;")
        || trimmed == "elseif"
        || trimmed.starts_with("elseif")
    {
        return "Unmatched 'else' / 'elseif' without a corresponding 'if' statement".to_string();
    }
    if trimmed == "except"
        || trimmed.starts_with("except")
        || trimmed == "finally"
        || trimmed.starts_with("finally")
    {
        return "Unmatched 'except' / 'finally' without a corresponding 'try' statement"
            .to_string();
    }

    if trimmed.starts_with("try") {
        let open_line = node.start_position().row + 1;
        return format!(
            "Unclosed 'try' block (opened on line {}); expected matching 'endtry'",
            open_line
        );
    }

    if trimmed.starts_with('"') && (!trimmed[1..].contains('"') || trimmed.ends_with('\\')) {
        return "Unclosed string literal".to_string();
    }

    if trimmed == "." || trimmed.starts_with('.') {
        return "Expected property name after '.'".to_string();
    }
    if trimmed == ":" || trimmed.starts_with(':') {
        return "Expected verb name after ':'".to_string();
    }
    if trimmed == "$" || trimmed.starts_with('$') {
        return "Expected identifier after '$'".to_string();
    }

    if let Some(first_char) = trimmed.chars().next()
        && matches!(
            first_char,
            '+' | '-' | '*' | '/' | '%' | '^' | '=' | '<' | '>' | '|' | '&'
        )
    {
        return format!("Expected expression after operator '{}'", first_char);
    }

    let prev_text = safe_slice(text, 0..node.start_byte());
    if let Some(last_char) = prev_text.chars().rev().find(|c| !c.is_whitespace()) {
        match last_char {
            '.' => return "Expected property name after '.'".to_string(),
            ':' => return "Expected verb name after ':'".to_string(),
            '$' => return "Expected identifier after '$'".to_string(),
            '+' | '-' | '*' | '/' | '%' | '^' | '=' | '<' | '>' | '!' | '|' | '&' => {
                return format!("Expected expression after operator '{}'", last_char);
            }
            _ => {}
        }
    }

    if !trimmed.is_empty() {
        if trimmed.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return format!("Missing ';' before '{}'", trimmed);
        }
        if trimmed.len() <= 30 {
            return format!("Syntax error near '{}'", trimmed);
        }
    }

    "Syntax error".to_string()
}
