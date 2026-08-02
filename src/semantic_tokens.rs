// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType, SemanticTokensLegend};
use tree_sitter::Node;

const VARIABLE: u32 = 0;
const PROPERTY: u32 = 1;
const FUNCTION: u32 = 2;
const METHOD: u32 = 3;
const KEYWORD: u32 = 4;
const COMMENT: u32 = 5;
const STRING: u32 = 6;
const NUMBER: u32 = 7;
const OPERATOR: u32 = 8;
const ENUM_MEMBER: u32 = 9;

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::VARIABLE,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::METHOD,
            SemanticTokenType::KEYWORD,
            SemanticTokenType::COMMENT,
            SemanticTokenType::STRING,
            SemanticTokenType::NUMBER,
            SemanticTokenType::OPERATOR,
            SemanticTokenType::ENUM_MEMBER,
        ],
        token_modifiers: Vec::new(),
    }
}

pub fn collect(text: &str) -> Vec<SemanticToken> {
    let Some(tree) = crate::parser::parse(text) else {
        return Vec::new();
    };

    let mut absolute = Vec::new();
    collect_node(tree.root_node(), text, &mut absolute);
    absolute.sort_unstable_by_key(|token| (token.line, token.start));

    let mut previous_line = 0;
    let mut previous_start = 0;
    absolute
        .into_iter()
        .map(|token| {
            let delta_line = token.line - previous_line;
            let delta_start = if delta_line == 0 {
                token.start - previous_start
            } else {
                token.start
            };
            previous_line = token.line;
            previous_start = token.start;
            SemanticToken {
                delta_line,
                delta_start,
                length: token.length,
                token_type: token.token_type,
                token_modifiers_bitset: 0,
            }
        })
        .collect()
}

#[derive(Debug)]
struct AbsoluteToken {
    line: u32,
    start: u32,
    length: u32,
    token_type: u32,
}

fn collect_node(node: Node<'_>, text: &str, tokens: &mut Vec<AbsoluteToken>) {
    if let Some(token_type) = classify(node, text) {
        push_range(text, node.start_byte(), node.end_byte(), token_type, tokens);
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node(child, text, tokens);
    }
}

fn classify(node: Node<'_>, text: &str) -> Option<u32> {
    match node.kind() {
        "identifier" => Some(classify_identifier(node, text)),
        "string" if is_standalone_expression(node) => Some(COMMENT),
        "string" => Some(STRING),
        "number" | "object" => Some(NUMBER),
        "error" => Some(ENUM_MEMBER),
        "if" | "elseif" | "else" | "endif" | "for" | "in" | "endfor" | "while" | "endwhile"
        | "fork" | "endfork" | "break" | "continue" | "return" | "try" | "except" | "finally"
        | "endtry" | "ANY" => Some(KEYWORD),
        "=" | "+" | "-" | "*" | "/" | "%" | "^" | "&&" | "||" | "==" | "!=" | "<" | "<=" | ">"
        | ">=" | "|." | "^." | "&." | "<<" | ">>" | ">>>" | "!" | "~" | "?" | "|" | "@" | "$"
        | ".." | "=>" => Some(OPERATOR),
        _ => None,
    }
}

fn is_standalone_expression(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    while let Some(current) = parent {
        match current.kind() {
            "expression_statement" => return true,
            "expression" => parent = current.parent(),
            _ => return false,
        }
    }
    false
}

fn classify_identifier(node: Node<'_>, text: &str) -> u32 {
    const TYPE_VALUES: &[&str] = &[
        "INT", "NUM", "OBJ", "STR", "ERR", "LIST", "FLOAT", "MAP", "BOOL", "WAIF",
    ];
    let identifier = &text[node.byte_range()];
    if TYPE_VALUES
        .iter()
        .any(|type_value| identifier.eq_ignore_ascii_case(type_value))
    {
        return ENUM_MEMBER;
    }

    let Some(parent) = node.parent() else {
        return VARIABLE;
    };
    match parent.kind() {
        "call_expression" => FUNCTION,
        "verb_call" if preceded_by(node, text, ':') || preceded_by(node, text, '$') => METHOD,
        "prop_access" if preceded_by(node, text, '.') || preceded_by(node, text, '$') => PROPERTY,
        _ => VARIABLE,
    }
}

fn preceded_by(node: Node<'_>, text: &str, marker: char) -> bool {
    text[..node.start_byte()]
        .chars()
        .rev()
        .find(|c| !c.is_whitespace())
        == Some(marker)
}

fn push_range(
    text: &str,
    start: usize,
    end: usize,
    token_type: u32,
    tokens: &mut Vec<AbsoluteToken>,
) {
    let mut offset = start;
    while offset < end {
        let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
        let line_end = text[offset..end]
            .find('\n')
            .map_or(end, |index| offset + index);
        let content_end = if line_end > offset && text.as_bytes()[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };

        if content_end > offset {
            tokens.push(AbsoluteToken {
                line: text[..line_start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count() as u32,
                start: text[line_start..offset].encode_utf16().count() as u32,
                length: text[offset..content_end].encode_utf16().count() as u32,
                token_type,
            });
        }
        offset = line_end.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_language_constructs() {
        let tokens = collect("if (x) notify(player.name); thing:move(); endif");
        let types: Vec<u32> = tokens.iter().map(|token| token.token_type).collect();
        assert_eq!(
            types,
            vec![
                KEYWORD, VARIABLE, FUNCTION, VARIABLE, PROPERTY, VARIABLE, METHOD, KEYWORD
            ]
        );
    }

    #[test]
    fn uses_utf16_columns_and_recognizes_comment_strings() {
        let tokens = collect("\"😀\"; notify(\"text\");");
        assert_eq!(tokens[0].length, 4);
        assert_eq!(tokens[0].token_type, COMMENT);
        assert_eq!(tokens[1].token_type, FUNCTION);
        assert_eq!(tokens[2].token_type, STRING);
    }

    #[test]
    fn classifies_builtin_type_values_as_enum_members() {
        let tokens = collect("types = {int, Num, OBJ, str, Err, LIST, float, Map, BOOL, waif};");
        assert_eq!(tokens[0].token_type, VARIABLE);
        assert_eq!(tokens[1].token_type, OPERATOR);
        assert!(
            tokens[2..]
                .iter()
                .all(|token| token.token_type == ENUM_MEMBER)
        );
    }
}
