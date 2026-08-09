// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use crate::line_index::{LineIndex, safe_slice};
use lsp_types::{DocumentHighlight, DocumentHighlightKind, Location, Position, Range, Uri};
use std::sync::LazyLock;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

static LOCALS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    let query = format!(
        "{}\n(scattering_assignment (scatter_list) @local.scatter)",
        tree_sitter_lambdamoo::LOCALS_QUERY
    );
    Query::new(&language, &query).expect("Failed to compile locals Tree-sitter query")
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

            if cap_name == "local.scatter" {
                collect_scatter_definitions(cap_node, text, &line_index, &mut symbols);
                continue;
            }

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

fn collect_scatter_definitions(
    scatter_list: Node,
    text: &str,
    line_index: &LineIndex,
    symbols: &mut Vec<SymbolLocation>,
) {
    let mut cursor = scatter_list.walk();
    let mut needs_target = true;
    for child in scatter_list.children(&mut cursor) {
        if child.kind() == "," {
            needs_target = true;
        } else if needs_target && child.kind() == "identifier" {
            let range = line_index.clamp_range(
                text,
                child.start_position().row,
                child.start_position().column,
                child.end_position().row,
                child.end_position().column,
            );
            let name = safe_slice(text, child.byte_range()).to_string();
            if !symbols.iter().any(|s| s.range == range && s.is_definition) {
                symbols.push(SymbolLocation {
                    name,
                    is_definition: true,
                    range,
                });
            }
            needs_target = false;
        }
    }
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

    let target_offset = position_to_byte_offset(text, target_symbol.range.start)?;

    // Assignments evaluate their right-hand side before updating their target. Exclude
    // assignment targets that enclose this reference, then use the most recent earlier
    // definition.
    let def = locals
        .iter()
        .filter(|s| {
            s.is_definition
                && s.name == target_symbol.name
                && position_before_or_equal(s.range.end, target_symbol.range.start)
                && !is_enclosing_assignment_target(root, text, s.range, target_offset)
        })
        .max_by_key(|s| (s.range.start.line, s.range.start.character))?;
    Some(Location {
        uri: uri.clone(),
        range: def.range,
    })
}

fn is_enclosing_assignment_target(
    root: Node,
    text: &str,
    definition_range: Range,
    target_offset: usize,
) -> bool {
    let Some(definition_offset) = position_to_byte_offset(text, definition_range.start) else {
        return false;
    };
    let Some(mut node) = root.descendant_for_byte_range(definition_offset, definition_offset)
    else {
        return false;
    };

    loop {
        if node.kind() == "assignment"
            && node.named_child(0).is_some_and(|lhs| {
                lhs.start_byte() <= definition_offset
                    && definition_offset < lhs.end_byte()
                    && lhs.end_byte() <= target_offset
                    && target_offset < node.end_byte()
            })
        {
            return true;
        }
        let Some(parent) = node.parent() else {
            return false;
        };
        node = parent;
    }
}

fn position_to_byte_offset(text: &str, position: Position) -> Option<usize> {
    let line_start = text
        .split_inclusive('\n')
        .take(position.line as usize)
        .map(str::len)
        .sum::<usize>();
    let line = text.get(line_start..)?.split('\n').next()?;
    let mut utf16_col = 0;
    for (byte_col, ch) in line.char_indices() {
        if utf16_col == position.character as usize {
            return Some(line_start + byte_col);
        }
        utf16_col += ch.len_utf16();
        if utf16_col > position.character as usize {
            return None;
        }
    }
    (utf16_col == position.character as usize).then_some(line_start + line.len())
}

fn position_before_or_equal(left: Position, right: Position) -> bool {
    (left.line, left.character) <= (right.line, right.character)
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
    fn test_find_definition_on_rhs_of_reassignment_uses_previous_assignment() {
        let code = "target = 1;\ntarget = target + 1;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        let def_loc = find_definition(tree.root_node(), code, Position::new(1, 9), &uri).unwrap();

        assert_eq!(
            def_loc.range,
            Range::new(Position::new(0, 0), Position::new(0, 6))
        );
    }

    #[test]
    fn test_find_definition_from_scatter_assignment() {
        let code = "{x} = args;\ny = x;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        let def_loc = find_definition(tree.root_node(), code, Position::new(1, 4), &uri).unwrap();

        assert_eq!(
            def_loc.range,
            Range::new(Position::new(0, 1), Position::new(0, 2))
        );
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
