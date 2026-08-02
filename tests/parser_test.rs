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
    parser::collect_diagnostics(root, &line_index, &mut diagnostics);
    assert!(!diagnostics.is_empty());
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
        "1 |. 2;",
        "1 ^. 2;",
        "1 &. 2;",
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
