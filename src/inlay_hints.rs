// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

//! Inlay hints derived from syntax and callable metadata.

use crate::{
    builtins,
    line_index::{LineIndex, safe_slice},
};
use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range};
use tree_sitter::Node;

fn position_in_range(position: Position, range: Range) -> bool {
    (position.line, position.character) >= (range.start.line, range.start.character)
        && (position.line, position.character) < (range.end.line, range.end.character)
}

fn argument_nodes(call: Node) -> Vec<Node> {
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut cursor = arguments.walk();
    arguments.named_children(&mut cursor).collect()
}

/// Collects parameter-name hints for calls within an LSP range.
///
/// Built-ins are the only metadata source today. Keeping traversal and LSP
/// construction here lets verb metadata become another source later.
pub fn collect(root: Node, text: &str, range: Range) -> Vec<InlayHint> {
    let index = LineIndex::new(text);
    let mut hints = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        let mut children: Vec<_> = node.named_children(&mut cursor).collect();
        children.reverse();
        stack.extend(children);

        if node.kind() != "call_expression" || node.has_error() {
            continue;
        }
        let Some(function) = node.child_by_field_name("function") else {
            continue;
        };
        let Some(builtin) = builtins::find(safe_slice(text, function.byte_range())) else {
            continue;
        };

        for (argument_index, argument) in argument_nodes(node).into_iter().enumerate() {
            if safe_slice(text, argument.byte_range())
                .trim_start()
                .starts_with('@')
            {
                break;
            }
            let Some(name) = builtin.argument_name(argument_index) else {
                continue;
            };
            let position = index.clamp_point(
                text,
                argument.start_position().row,
                argument.start_position().column,
            );
            if !position_in_range(position, range) {
                continue;
            }
            hints.push(InlayHint {
                position,
                label: InlayHintLabel::String(format!("{name}:")),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip: None,
                padding_left: None,
                padding_right: Some(true),
                data: None,
            });
        }
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn full_range() -> Range {
        Range::new(Position::new(0, 0), Position::new(u32::MAX, 0))
    }

    fn labels(hints: &[InlayHint]) -> Vec<&str> {
        hints
            .iter()
            .map(|hint| match &hint.label {
                InlayHintLabel::String(label) => label.as_str(),
                InlayHintLabel::LabelParts(_) => panic!("expected string label"),
            })
            .collect()
    }

    #[test]
    fn collects_only_curated_names_in_source_order() {
        let text = "x = length(is_member(player, args)); notify(player, \"hi\");";
        let tree = parser::parse(text).unwrap();
        let hints = collect(tree.root_node(), text, full_range());

        assert_eq!(labels(&hints), ["value:", "value:", "list:"]);
        assert!(
            hints
                .windows(2)
                .all(|pair| pair[0].position <= pair[1].position)
        );
        assert!(
            hints
                .iter()
                .all(|hint| hint.kind == Some(InlayHintKind::PARAMETER))
        );
    }

    #[test]
    fn repeats_variadic_names_and_stops_at_a_splice() {
        let text = concat!(
            "call_function(\"f\", 1, 2, 3); ",
            "call_function(\"g\", @values, 4);"
        );
        let tree = parser::parse(text).unwrap();
        let hints = collect(tree.root_node(), text, full_range());

        assert_eq!(
            labels(&hints),
            ["func-name:", "value:", "value:", "value:", "func-name:"]
        );
    }

    #[test]
    fn filters_by_utf16_request_range() {
        let text = "note = \"🦋\";\nlength(args);\nlength(player);";
        let tree = parser::parse(text).unwrap();
        let hints = collect(
            tree.root_node(),
            text,
            Range::new(Position::new(1, 7), Position::new(2, 0)),
        );

        assert_eq!(labels(&hints), ["value:"]);
        assert_eq!(hints[0].position, Position::new(1, 7));
    }
}
