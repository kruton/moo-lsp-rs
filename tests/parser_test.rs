// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use moo_lsp_rs::line_index::LineIndex;
use moo_lsp_rs::parser;

#[test]
fn test_valid_code() {
    let code = "a = 1; if (a) b = 2; endif;";
    let tree = parser::parse(code).expect("Failed to parse");
    assert!(!tree.root_node().has_error());
}

#[test]
fn test_invalid_code() {
    let code = "a = 1;\nif (a\n  b = 2;\nendif;";
    let tree = parser::parse(code).expect("Failed to parse");
    let root = tree.root_node();
    assert!(root.has_error());

    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(root, &line_index, code, &mut diagnostics);
    assert!(!diagnostics.is_empty());
}

#[test]
fn test_keyword_used_as_identifier_diagnostic() {
    let code = "notify(if, \"hi\"); result = E_NONE;";
    let tree = parser::parse(code).unwrap();
    assert!(!tree.root_node().has_error());

    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "Keyword cannot be used as an identifier"
    );
    assert_eq!(
        diagnostics[0].code.as_ref().unwrap(),
        &lsp_types::NumberOrString::String("invalid-identifier".to_string())
    );
    assert_eq!(diagnostics[0].range.start.character, 7);
    assert_eq!(diagnostics[0].range.end.character, 9);
}

#[test]
fn test_unclosed_block_diagnostics() {
    let code = "if (x)\n  b = 1;\n";
    let tree = parser::parse(code).unwrap();
    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0]
            .message
            .contains("Unclosed 'if' statement (opened on line 1)")
    );

    let code_for = "for x in (y)\n  b = 1;\n";
    let tree_for = parser::parse(code_for).unwrap();
    let line_index_for = LineIndex::new(code_for);
    let mut diags_for = Vec::new();
    parser::collect_diagnostics(
        tree_for.root_node(),
        &line_index_for,
        code_for,
        &mut diags_for,
    );
    assert_eq!(diags_for.len(), 1);
    assert!(
        diags_for[0]
            .message
            .contains("Unclosed 'for' loop (opened on line 1)")
    );
}

#[test]
fn test_diagnostic_range_within_line_bounds() {
    let code = "if (x)\n  b = 1;";
    let tree = parser::parse(code).unwrap();
    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);
    assert!(!diagnostics.is_empty());
    for d in &diagnostics {
        let line_len = |line: u32| {
            let line_str = code.split_inclusive('\n').nth(line as usize).unwrap();
            let content = line_str.strip_suffix('\n').unwrap_or(line_str);
            let content = content.strip_suffix('\r').unwrap_or(content);
            content.encode_utf16().count() as u32
        };

        let start_max_utf16 = line_len(d.range.start.line);
        let end_max_utf16 = line_len(d.range.end.line);

        assert!(
            d.range.start.character <= start_max_utf16,
            "Start character {} exceeds line length {}",
            d.range.start.character,
            start_max_utf16
        );
        assert!(
            d.range.end.character <= end_max_utf16,
            "End character {} exceeds line length {}",
            d.range.end.character,
            end_max_utf16
        );
    }
}

#[test]
fn test_mismatched_block_terminator_diagnostic() {
    let code = "if (x)\n  b = 1;\nendfor;";
    let tree = parser::parse(code).unwrap();
    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);
    assert!(!diagnostics.is_empty());
    assert!(diagnostics.iter().any(|d| {
        d.message
            .contains("Mismatched block terminator: found 'endfor'")
    }));
}

#[test]
fn test_orphan_keyword_diagnostics() {
    let code = "a = 1;\nendif;";
    let tree = parser::parse(code).unwrap();
    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);
    assert!(!diagnostics.is_empty());
    assert!(diagnostics.iter().any(|d| {
        d.message
            .contains("Unmatched 'endif' without a corresponding 'if' statement")
    }));
}

#[test]
fn test_missing_delimiters_and_operators() {
    let code_paren = "a = (1 + 2;";
    let tree_paren = parser::parse(code_paren).unwrap();
    let line_index_paren = LineIndex::new(code_paren);
    let mut diags_paren = Vec::new();
    parser::collect_diagnostics(
        tree_paren.root_node(),
        &line_index_paren,
        code_paren,
        &mut diags_paren,
    );
    assert!(
        diags_paren
            .iter()
            .any(|d| d.message == "Missing closing parenthesis ')'")
    );

    let code_op = "a = 1 + ;";
    let tree_op = parser::parse(code_op).unwrap();
    let line_index_op = LineIndex::new(code_op);
    let mut diags_op = Vec::new();
    parser::collect_diagnostics(tree_op.root_node(), &line_index_op, code_op, &mut diags_op);
    assert!(
        diags_op
            .iter()
            .any(|d| d.message.contains("Expected expression after operator '+'"))
    );

    let code_prop = "a = x.;";
    let tree_prop = parser::parse(code_prop).unwrap();
    let line_index_prop = LineIndex::new(code_prop);
    let mut diags_prop = Vec::new();
    parser::collect_diagnostics(
        tree_prop.root_node(),
        &line_index_prop,
        code_prop,
        &mut diags_prop,
    );
    assert!(
        diags_prop
            .iter()
            .any(|d| d.message == "Expected property name after '.'")
    );
}

#[test]
fn test_collect_folding_ranges() {
    let code = "if (x)\n  b = 1;\n  c = 2;\nendif;\n";
    let tree = parser::parse(code).unwrap();
    let folds = parser::collect_folding_ranges(tree.root_node(), code);
    assert_eq!(folds.len(), 1);
    assert_eq!(folds[0].start_line, 0);
    assert_eq!(folds[0].end_line, 3);
}

#[test]
fn test_collect_document_symbols() {
    let code = "a = 1;\nnotify(player, \"hi\");\nitem:run();\n$announce();\n";
    let tree = parser::parse(code).unwrap();
    let symbols = parser::collect_document_symbols(tree.root_node(), code);
    assert!(symbols.iter().any(|symbol| {
        symbol.name == "notify(player, \"hi\")" && symbol.kind == lsp_types::SymbolKind::FUNCTION
    }));
    assert!(symbols.iter().any(|symbol| {
        symbol.name == "item:run()" && symbol.kind == lsp_types::SymbolKind::METHOD
    }));
    assert!(symbols.iter().any(|symbol| {
        symbol.name == "$announce()" && symbol.kind == lsp_types::SymbolKind::METHOD
    }));
}

#[test]
fn test_all_features() {
    let snippets = vec![
        // Statements
        "if (1) a = 1; endif;",
        "for x in (y) a = 1; endfor;",
        "for x in [1..10] a = 1; endfor;",
        "while (1) a = 1; endwhile;",
        "while x (1) a = 1; endwhile;",
        "fork (1) a = 1; endfork;",
        "fork x (1) a = 1; endfork;",
        "a = 1;",
        "break;",
        "break x;",
        "continue;",
        "continue x;",
        "return 1;",
        "return;",
        ";",
        "try a = 1; except (E_NONE) b = 2; endtry;",
        "try a = 1; finally b = 2; endtry;",
        // Expressions
        "1;",
        "1.0;",
        "\"hello\";",
        "#0;",
        "E_NONE;",
        "x;",
        "$foo;",
        "x.foo;",
        "x.(y);",
        "x:foo();",
        "$foo();",
        "x:(y)();",
        "x[1];",
        "x[1..2];",
        "x[$];",
        "x = 1;",
        "{x, ?y} = z;",
        "typeof(x);",
        "1 + 2;",
        "1 - 2;",
        "1 * 2;",
        "1 / 2;",
        "1 % 2;",
        "1 ^ 2;",
        "1 && 2;",
        "1 || 2;",
        "1 == 2;",
        "1 != 2;",
        "1 < 2;",
        "1 <= 2;",
        "1 > 2;",
        "1 >= 2;",
        "1 in 2;",
        "1 .|. 2;",
        "1 .^. 2;",
        "1 .&. 2;",
        "1 << 2;",
        "1 >> 2;",
        "1 >>> 2;",
        "-1;",
        "~1;",
        "!1;",
        "(1 + 2);",
        "{1, 2};",
        "1 ? 2 | 3;",
        "`1 ! E_NONE => 2';",
    ];

    for snippet in snippets {
        let tree = parser::parse(snippet).expect("Failed to parse");
        assert!(
            !tree.root_node().has_error(),
            "Failed to parse snippet: {}",
            snippet
        );
    }
}

#[test]
fn test_unbalanced_paren_and_missing_semicolon_diagnostic() {
    let code = "notify(player, \"hi\"";
    let tree = parser::parse(code).unwrap();
    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);
    assert_eq!(diagnostics.len(), 2);

    assert_eq!(diagnostics[0].message, "Missing closing parenthesis ')'");
    assert_eq!(
        diagnostics[0].code.as_ref().unwrap(),
        &lsp_types::NumberOrString::String("unclosed-delimiter".to_string())
    );

    assert_eq!(diagnostics[1].message, "Missing ';' at end of statement");
    assert_eq!(
        diagnostics[1].code.as_ref().unwrap(),
        &lsp_types::NumberOrString::String("missing-semicolon".to_string())
    );
}

#[test]
fn test_diagnostic_codes_and_related_info() {
    let code = "if (x)\n  b = 1;\n";
    let tree = parser::parse(code).unwrap();
    let line_index = LineIndex::new(code);
    let mut diagnostics = Vec::new();
    parser::collect_diagnostics(tree.root_node(), &line_index, code, &mut diagnostics);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code.as_ref().unwrap(),
        &lsp_types::NumberOrString::String("unclosed-block".to_string())
    );
    let related = diagnostics[0]
        .related_information
        .as_ref()
        .expect("Expected related_information");
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.range.start.line, 0);
    assert_eq!(related[0].location.range.start.character, 0);
    assert_eq!(related[0].message, "'if' block opened here");

    let code_bracket = "a = [1, 2";
    let tree_bracket = parser::parse(code_bracket).unwrap();
    let line_index_bracket = LineIndex::new(code_bracket);
    let mut diags_bracket = Vec::new();
    parser::collect_diagnostics(
        tree_bracket.root_node(),
        &line_index_bracket,
        code_bracket,
        &mut diags_bracket,
    );
    assert_eq!(diags_bracket.len(), 2);

    assert_eq!(diags_bracket[0].message, "Missing closing bracket ']'");
    assert_eq!(
        diags_bracket[0].code.as_ref().unwrap(),
        &lsp_types::NumberOrString::String("unclosed-delimiter".to_string())
    );
    assert_eq!(diags_bracket[1].message, "Missing ';' at end of statement");
    assert_eq!(
        diags_bracket[1].code.as_ref().unwrap(),
        &lsp_types::NumberOrString::String("missing-semicolon".to_string())
    );
}
