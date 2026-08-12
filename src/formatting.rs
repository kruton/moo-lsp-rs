// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use std::sync::LazyLock;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

use crate::parser;

const INDENT_WIDTH: usize = 2;

static INDENT_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    Query::new(&language, tree_sitter_lambdamoo::INDENTS_QUERY)
        .expect("Failed to compile indentation Tree-sitter query")
});

#[derive(Clone, Copy, Debug)]
enum IndentEvent {
    Begin,
    Branch,
    End,
}

/// Format a verb in the same two-space block style used by `verb_code()`.
///
/// Leading whitespace is normalized and empty statements are removed. This
/// deliberately leaves expression spelling, comments, and line endings alone.
pub fn format(text: &str) -> Option<String> {
    let tree = parser::parse(text)?;
    if tree.root_node().has_error() {
        return None;
    }

    let mut empty_statement_ranges = Vec::new();
    collect_empty_statements(tree.root_node(), &mut empty_statement_ranges);
    let mut normalized = text.to_owned();
    for range in empty_statement_ranges.into_iter().rev() {
        normalized.replace_range(range, "");
    }

    let tree = parser::parse(&normalized)?;
    let mut statement_boundaries = Vec::new();
    collect_statement_boundaries(tree.root_node(), &mut statement_boundaries);
    insert_line_breaks(&mut normalized, statement_boundaries);

    // Inserted statement lines change the rows used by the indentation query.
    let tree = parser::parse(&normalized)?;
    let line_count = normalized.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let mut line_events = vec![Vec::new(); line_count];
    collect_indent_events(tree.root_node(), normalized.as_bytes(), &mut line_events);

    let mut depth = 0usize;
    let mut formatted = String::with_capacity(text.len());
    for (line_number, line) in normalized.split_inclusive('\n').enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let (content, newline) = match content.strip_suffix('\r') {
            Some(content) if line.ends_with("\r\n") => (content, "\r\n"),
            _ if line.ends_with('\n') => (content, "\n"),
            _ => (content, ""),
        };

        let events = line_events
            .get(line_number)
            .map_or(&[][..], |v| v.as_slice());
        let starts_with_dedent = events
            .first()
            .is_some_and(|ev| matches!(ev, IndentEvent::Branch | IndentEvent::End));
        let line_depth = depth.saturating_sub(usize::from(starts_with_dedent));

        let trimmed = content.trim_start_matches([' ', '\t']);
        if !trimmed.is_empty() {
            formatted.extend(std::iter::repeat_n(' ', line_depth * INDENT_WIDTH));
            formatted.push_str(trimmed);
        }
        formatted.push_str(newline);

        for event in events {
            match event {
                IndentEvent::Begin => depth += 1,
                IndentEvent::Branch => {}
                IndentEvent::End => depth = depth.saturating_sub(1),
            }
        }
    }

    Some(formatted)
}

fn collect_statement_boundaries(node: Node<'_>, boundaries: &mut Vec<usize>) {
    if matches!(
        node.kind(),
        "statement" | "elseif_clause" | "else_clause" | "except_clause"
    ) || matches!(
        node.kind(),
        "endif" | "endfor" | "endwhile" | "endfork" | "finally" | "endtry"
    ) {
        boundaries.push(node.start_byte());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_statement_boundaries(child, boundaries);
    }
}

fn insert_line_breaks(text: &mut String, mut boundaries: Vec<usize>) {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    boundaries.sort_unstable();
    boundaries.dedup();

    for boundary in boundaries.into_iter().rev() {
        let bytes = text.as_bytes();
        let mut whitespace_start = boundary;
        while whitespace_start > 0 && matches!(bytes[whitespace_start - 1], b' ' | b'\t') {
            whitespace_start -= 1;
        }
        if whitespace_start > 0 && bytes[whitespace_start - 1] != b'\n' {
            text.replace_range(whitespace_start..boundary, newline);
        }
    }
}

fn collect_empty_statements(node: Node<'_>, ranges: &mut Vec<std::ops::Range<usize>>) {
    if node.kind() == "statement" && node.named_child_count() == 0 {
        ranges.push(node.byte_range());
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_empty_statements(child, ranges);
    }
}

fn collect_indent_events(node: Node<'_>, source: &[u8], lines: &mut [Vec<IndentEvent>]) {
    let query = &*INDENT_QUERY;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, node, source);

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_name = match query.capture_names().get(cap.index as usize) {
                Some(name) => *name,
                None => continue,
            };
            let row = cap.node.start_position().row;

            let event = match cap_name {
                "indent.begin" => IndentEvent::Begin,
                "indent.branch" => IndentEvent::Branch,
                "indent.end" => IndentEvent::End,
                _ => continue,
            };

            if row < lines.len() {
                lines[row].push(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn indents_nested_blocks_and_branches() {
        let input = "if (ready)\nvalue = 1;\nfor item in (items)\ntry\nitem:run();\nexcept err (ANY)\nreturn err;\nendtry\nendfor\nelse\ntry\nreturn;\nfinally\ncleanup();\nendtry\nendif\n";
        let expected = "if (ready)\n  value = 1;\n  for item in (items)\n    try\n      item:run();\n    except err (ANY)\n      return err;\n    endtry\n  endfor\nelse\n  try\n    return;\n  finally\n    cleanup();\n  endtry\nendif\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn preserves_comments_line_endings_and_expression_text() {
        let input = "if (x)\r\n\t\"endif is not code\";\r\n  y   =   \"if\";\r\nendif\r\n";
        let expected = "if (x)\r\n  \"endif is not code\";\r\n  y   =   \"if\";\r\nendif\r\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn collapses_repeated_statement_semicolons() {
        assert_eq!(format("0;;;;;;\n").as_deref(), Some("0;\n"));
    }

    #[test]
    fn puts_statements_and_block_delimiters_on_separate_lines() {
        let input = "ready = 1; if (ready) notify(player, \"Ready\"); endif\n";
        let expected = "ready = 1;\nif (ready)\n  notify(player, \"Ready\");\nendif\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn puts_simple_statements_on_separate_lines() {
        let input = "first = 1; second = 2; notify(player, \"Done\");\n";
        let expected = "first = 1;\nsecond = 2;\nnotify(player, \"Done\");\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn splits_inline_try_except_blocks() {
        let input = "try risky(); except err (E_PERM) handle(err); except (ANY) return; endtry\n";
        let expected = "try\n  risky();\nexcept err (E_PERM)\n  handle(err);\nexcept (ANY)\n  return;\nendtry\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn splits_inline_try_finally_blocks() {
        let input = "try risky(); finally cleanup(); endtry\n";
        let expected = "try\n  risky();\nfinally\n  cleanup();\nendtry\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn splits_inline_conditional_branches() {
        let input = "if (first) one(); elseif (second) two(); else three(); endif\n";
        let expected = "if (first)\n  one();\nelseif (second)\n  two();\nelse\n  three();\nendif\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn refuses_to_format_invalid_programs() {
        assert_eq!(format("if (x)\nvalue = 1;\n"), None);
    }
}
