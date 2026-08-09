// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use crate::line_index::{LineIndex, safe_slice};
use lsp_types::{DocumentHighlight, DocumentHighlightKind, Location, Position, Range, Uri};
use std::sync::LazyLock;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

static LOCALS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    Query::new(&language, tree_sitter_lambdamoo::LOCALS_QUERY)
        .expect("Failed to compile locals Tree-sitter query")
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolLocation {
    pub name: String,
    pub is_definition: bool,
    pub range: Range,
}

pub fn collect_locals(root: Node, text: &str) -> Vec<SymbolLocation> {
    let query = &*LOCALS_QUERY;
    let line_index = LineIndex::new(text);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, text.as_bytes());
    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_name = match query.capture_names().get(cap.index as usize) {
                Some(name) => *name,
                None => continue,
            };
            let cap_node = cap.node;
            let name = safe_slice(text, cap_node.byte_range()).trim().to_string();

            if name.is_empty() {
                continue;
            }

            let start_pos = cap_node.start_position();
            let end_pos = cap_node.end_position();
            let range = line_index.clamp_range(
                text,
                start_pos.row,
                start_pos.column,
                end_pos.row,
                end_pos.column,
            );

            let is_definition = match cap_name {
                "local.definition" => true,
                "local.reference" => false,
                _ => continue,
            };

            if !is_definition
                && symbols
                    .iter()
                    .any(|s: &SymbolLocation| s.range == range && s.is_definition)
            {
                continue;
            }

            // Avoid duplicate range entries
            if !symbols
                .iter()
                .any(|s: &SymbolLocation| s.range == range && s.is_definition == is_definition)
            {
                symbols.push(SymbolLocation {
                    name,
                    is_definition,
                    range,
                });
            }
        }
    }

    symbols
}

pub fn find_definition(root: Node, text: &str, position: Position, uri: &Uri) -> Option<Location> {
    let locals = collect_locals(root, text);
    let target_symbol = locals
        .iter()
        .find(|s| position_in_range(position, s.range))
        .or_else(|| {
            if position.character == 0 {
                return None;
            }
            let prev = Position {
                line: position.line,
                character: position.character - 1,
            };
            locals.iter().find(|s| position_in_range(prev, s.range))
        })?;

    // Find the definition of this symbol that occurs in the locals
    let def = locals
        .iter()
        .find(|s| s.is_definition && s.name == target_symbol.name)?;
    Some(Location {
        uri: uri.clone(),
        range: def.range,
    })
}

pub fn find_highlights(root: Node, text: &str, position: Position) -> Vec<DocumentHighlight> {
    let delimiter_highlights = find_delimiter_or_keyword_highlights(root, text, position);
    if !delimiter_highlights.is_empty() {
        return delimiter_highlights;
    }

    let locals = collect_locals(root, text);
    let Some(target_symbol) = locals.iter().find(|s| position_in_range(position, s.range)) else {
        return Vec::new();
    };
    let target_name = target_symbol.name.clone();

    locals
        .into_iter()
        .filter(|s| s.name == target_name)
        .map(|s| DocumentHighlight {
            range: s.range,
            kind: if s.is_definition {
                Some(DocumentHighlightKind::WRITE)
            } else {
                Some(DocumentHighlightKind::READ)
            },
        })
        .collect()
}

fn find_delimiter_or_keyword_highlights(
    root: Node,
    text: &str,
    position: Position,
) -> Vec<DocumentHighlight> {
    let point = tree_sitter::Point {
        row: position.line as usize,
        column: position.character as usize,
    };

    let node_at = root.descendant_for_point_range(point, point);
    let mut candidate_nodes_expanded = Vec::new();
    if let Some(n) = node_at {
        candidate_nodes_expanded.push(n);
        if let Some(p) = n.parent() {
            candidate_nodes_expanded.push(p);
        }
    }
    if point.column > 0 {
        let prev_point = tree_sitter::Point {
            row: point.row,
            column: point.column - 1,
        };
        if let Some(prev_node) = root.descendant_for_point_range(prev_point, prev_point) {
            candidate_nodes_expanded.push(prev_node);
            if let Some(p) = prev_node.parent() {
                candidate_nodes_expanded.push(p);
            }
        }
    }

    for node in candidate_nodes_expanded {
        let kind = node.kind();
        let node_slice = safe_slice(text, node.byte_range()).trim();

        if (kind == "(" || kind == ")")
            && let Some(parent) = node.parent()
        {
            let highlights = find_matching_token_children(parent, "(", ")");
            if !highlights.is_empty() {
                return highlights;
            }
        }

        if (kind == "[" || kind == "]")
            && let Some(parent) = node.parent()
        {
            let highlights = find_matching_token_children(parent, "[", "]");
            if !highlights.is_empty() {
                return highlights;
            }
        }

        if (kind == "{" || kind == "}")
            && let Some(parent) = node.parent()
        {
            let highlights = find_matching_token_children(parent, "{", "}");
            if !highlights.is_empty() {
                return highlights;
            }
        }

        match node_slice {
            "if" | "elseif" | "else" | "endif" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "if_statement") {
                    let highlights =
                        find_matching_keywords(stmt_node, text, &["if", "elseif", "else", "endif"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "for" | "endfor" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "for_statement") {
                    let highlights = find_matching_keywords(stmt_node, text, &["for", "endfor"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "while" | "endwhile" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "while_statement") {
                    let highlights =
                        find_matching_keywords(stmt_node, text, &["while", "endwhile"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "fork" | "endfork" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "fork_statement") {
                    let highlights = find_matching_keywords(stmt_node, text, &["fork", "endfork"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "try" | "except" | "finally" | "endtry" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "try_statement") {
                    let highlights = find_matching_keywords(
                        stmt_node,
                        text,
                        &["try", "except", "finally", "endtry"],
                    );
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            _ => {}
        }
    }

    Vec::new()
}

fn find_matching_token_children(
    parent: Node,
    open_kind: &str,
    close_kind: &str,
) -> Vec<DocumentHighlight> {
    let mut open_node = None;
    let mut close_node = None;
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        if child.kind() == open_kind && open_node.is_none() {
            open_node = Some(child);
        } else if child.kind() == close_kind {
            close_node = Some(child);
        }
    }

    let mut result = Vec::new();
    if let Some(o) = open_node {
        let start = o.start_position();
        let end = o.end_position();
        result.push(DocumentHighlight {
            range: Range {
                start: Position {
                    line: start.row as u32,
                    character: start.column as u32,
                },
                end: Position {
                    line: end.row as u32,
                    character: end.column as u32,
                },
            },
            kind: Some(DocumentHighlightKind::TEXT),
        });
    }
    if let Some(c) = close_node {
        let start = c.start_position();
        let end = c.end_position();
        result.push(DocumentHighlight {
            range: Range {
                start: Position {
                    line: start.row as u32,
                    character: start.column as u32,
                },
                end: Position {
                    line: end.row as u32,
                    character: end.column as u32,
                },
            },
            kind: Some(DocumentHighlightKind::TEXT),
        });
    }
    if result.len() == 2 {
        result
    } else {
        Vec::new()
    }
}

fn find_matching_keywords(
    stmt_node: Node,
    text: &str,
    keywords: &[&str],
) -> Vec<DocumentHighlight> {
    let mut result = Vec::new();
    let mut cursor = stmt_node.walk();
    for child in stmt_node.children(&mut cursor) {
        let child_slice = safe_slice(text, child.byte_range())
            .trim()
            .trim_end_matches(';')
            .trim();
        if keywords.contains(&child_slice) {
            let start = child.start_position();
            let end = Position {
                line: start.row as u32,
                character: (start.column + child_slice.len()) as u32,
            };
            result.push(DocumentHighlight {
                range: Range {
                    start: Position {
                        line: start.row as u32,
                        character: start.column as u32,
                    },
                    end,
                },
                kind: Some(DocumentHighlightKind::TEXT),
            });
        }
    }
    result
}

fn find_ancestor_of_kind<'a>(mut node: Node<'a>, target_kind: &str) -> Option<Node<'a>> {
    while let Some(parent) = node.parent() {
        if parent.kind() == target_kind {
            return Some(parent);
        }
        node = parent;
    }
    None
}

fn position_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character >= range.end.character {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::str::FromStr;

    #[test]
    fn test_collect_locals() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let locals = collect_locals(tree.root_node(), code);
        assert!(!locals.is_empty());
        let x_defs: Vec<_> = locals
            .iter()
            .filter(|s| s.name == "x" && s.is_definition)
            .collect();
        assert_eq!(x_defs.len(), 1);
    }

    #[test]
    fn test_find_definition() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();
        // Position on 'x' in line 1: `y = x + 2;`
        let pos = Position {
            line: 1,
            character: 4,
        };
        let def_loc = find_definition(tree.root_node(), code, pos, &uri);
        assert!(def_loc.is_some());
        assert_eq!(def_loc.unwrap().range.start.line, 0);
    }

    #[test]
    fn test_position_in_range_end_is_exclusive() {
        let range = Range {
            start: Position {
                line: 1,
                character: 4,
            },
            end: Position {
                line: 1,
                character: 5,
            },
        };

        assert!(position_in_range(
            Position {
                line: 1,
                character: 4,
            },
            range
        ));
        assert!(!position_in_range(
            Position {
                line: 1,
                character: 5,
            },
            range
        ));
    }

    #[test]
    fn test_find_definition_when_cursor_is_after_symbol() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        // Position one character after `x` in `y = x + 2;`
        let pos = Position {
            line: 1,
            character: 5,
        };

        let def_loc = find_definition(tree.root_node(), code, pos, &uri);
        assert!(def_loc.is_some());
        let def_loc = def_loc.unwrap();
        assert_eq!(def_loc.range.start.line, 0);
        assert_eq!(def_loc.range.start.character, 0);
    }

    #[test]
    fn test_find_definition_uses_utf16_columns() {
        let code = "\"😀\"; x = 1;\n\"😀\"; y = x;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        // The emoji occupies two UTF-16 code units, but four UTF-8 bytes.
        let pos = Position {
            line: 1,
            character: 10,
        };

        let def_loc = find_definition(tree.root_node(), code, pos, &uri).unwrap();
        assert_eq!(
            def_loc.range,
            Range::new(Position::new(0, 6), Position::new(0, 7))
        );
    }

    #[test]
    fn test_find_highlights() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let pos = Position {
            line: 1,
            character: 4,
        };
        let highlights = find_highlights(tree.root_node(), code, pos);
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn test_find_highlights_delimiters() {
        let code = "notify(player);\n";
        let tree = parser::parse(code).unwrap();
        let pos_open = Position {
            line: 0,
            character: 6,
        };
        let highlights = find_highlights(tree.root_node(), code, pos_open);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].range.start.character, 6);
        assert_eq!(highlights[1].range.start.character, 13);
    }

    #[test]
    fn test_find_highlights_block_keywords() {
        let code = "if (x)\n  b = 1;\nendif;\n";
        let tree = parser::parse(code).unwrap();
        let pos_if = Position {
            line: 0,
            character: 0,
        };
        let highlights = find_highlights(tree.root_node(), code, pos_if);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].range.start.line, 0);
        assert_eq!(highlights[1].range.start.line, 2);
    }
}
