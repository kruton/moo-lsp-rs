// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use tree_sitter::Node;

use crate::parser;

const INDENT_WIDTH: usize = 2;

/// Format a verb in the same two-space block style used by `verb_code()`.
///
/// Only leading whitespace is changed. This deliberately leaves expression
/// spelling, comments, and line endings alone.
pub fn format(text: &str) -> Option<String> {
    let tree = parser::parse(text)?;
    if tree.root_node().has_error() {
        return None;
    }

    let line_count = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let mut keywords = vec![Vec::new(); line_count];
    collect_keywords(tree.root_node(), text.as_bytes(), &mut keywords);

    let mut depth = 0usize;
    let mut formatted = String::with_capacity(text.len());
    for (line_number, line) in text.split_inclusive('\n').enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let (content, newline) = match content.strip_suffix('\r') {
            Some(content) if line.ends_with("\r\n") => (content, "\r\n"),
            _ if line.ends_with('\n') => (content, "\n"),
            _ => (content, ""),
        };

        let line_keywords = &keywords[line_number];
        let starts_with_dedent = line_keywords.first().is_some_and(|keyword| {
            matches!(
                keyword.as_str(),
                "elseif"
                    | "else"
                    | "except"
                    | "finally"
                    | "endif"
                    | "endfor"
                    | "endwhile"
                    | "endfork"
                    | "endtry"
            )
        });
        let line_depth = depth.saturating_sub(usize::from(starts_with_dedent));

        let trimmed = content.trim_start_matches([' ', '\t']);
        if !trimmed.is_empty() {
            formatted.extend(std::iter::repeat_n(' ', line_depth * INDENT_WIDTH));
            formatted.push_str(trimmed);
        }
        formatted.push_str(newline);

        for keyword in line_keywords {
            match keyword.as_str() {
                "if" | "for" | "while" | "fork" | "try" => depth += 1,
                "elseif" | "else" | "except" | "finally" => {
                    // These close one arm and open the next, so the net depth
                    // is unchanged.
                }
                "endif" | "endfor" | "endwhile" | "endfork" | "endtry" => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    Some(formatted)
}

fn collect_keywords(node: Node<'_>, source: &[u8], lines: &mut [Vec<String>]) {
    if node.child_count() == 0 {
        let kind = node.kind();
        if matches!(
            kind,
            "if" | "elseif"
                | "else"
                | "endif"
                | "for"
                | "endfor"
                | "while"
                | "endwhile"
                | "fork"
                | "endfork"
                | "try"
                | "except"
                | "finally"
                | "endtry"
        ) && node.utf8_text(source).is_ok()
        {
            lines[node.start_position().row].push(kind.to_owned());
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_keywords(child, source, lines);
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
        let input = "if (x)\r\n\t/* endif is not code */\r\n  y   =   \"if\";\r\nendif\r\n";
        let expected = "if (x)\r\n  /* endif is not code */\r\n  y   =   \"if\";\r\nendif\r\n";
        assert_eq!(format(input).as_deref(), Some(expected));
    }

    #[test]
    fn refuses_to_format_invalid_programs() {
        assert_eq!(format("if (x)\nvalue = 1;\n"), None);
    }
}
