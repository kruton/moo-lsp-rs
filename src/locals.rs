// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

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
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, text.as_bytes());
    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_name = &query.capture_names()[cap.index as usize];
            let cap_node = cap.node;
            let name = text[cap_node.byte_range()].trim().to_string();

            if name.is_empty() {
                continue;
            }

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

            let is_definition = match *cap_name {
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
        .find(|s| position_in_range(position, s.range))?;

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

fn position_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character > range.end.character {
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
}
