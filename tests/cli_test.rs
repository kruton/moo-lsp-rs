// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_moo-lsp-rs")
}

fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new(binary());
    command.args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn temp_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("moo-lsp-cli-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn check_defaults_to_human_diagnostics_with_caret() {
    let output = run(&["check", "-"], Some("if (x)\nvalue = 1;\n"));

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("<stdin>:1:1: error[unclosed-block]"));
    assert!(stderr.contains("1 | if (x)\n  | ^"));
}

#[test]
fn json_is_versioned_and_warning_strictness_is_optional() {
    let normal = run(&["check", "--json", "-"], Some("custom();\n"));
    assert_eq!(normal.status.code(), Some(0));
    assert!(normal.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&normal.stdout).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["files"][0]["path"], "<stdin>");
    assert_eq!(value["summary"]["errors"], 0);
    assert_eq!(value["summary"]["warnings"], 1);
    assert_eq!(
        value["files"][0]["diagnostics"][0]["code"],
        "unknown-builtin"
    );

    let strict = run(&["check", "--deny-warnings", "-"], Some("custom();\n"));
    assert_eq!(strict.status.code(), Some(1));
}

#[test]
fn format_supports_stdout_check_write_and_directories() {
    let directory = temp_directory();
    let nested = directory.join("nested");
    fs::create_dir(&nested).unwrap();
    let source = nested.join("verb.moo");
    fs::write(&source, "if (x)\nvalue = 1;\nendif\n").unwrap();
    fs::write(nested.join("ignored.txt"), "not moo").unwrap();

    let preview = run(&["format", source.to_str().unwrap()], None);
    assert_eq!(preview.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(preview.stdout).unwrap(),
        "if (x)\n  value = 1;\nendif\n"
    );
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "if (x)\nvalue = 1;\nendif\n"
    );

    let check = run(&["format", "--check", directory.to_str().unwrap()], None);
    assert_eq!(check.status.code(), Some(1));
    assert!(
        String::from_utf8(check.stderr)
            .unwrap()
            .contains("verb.moo: needs formatting")
    );

    let write = run(&["format", "--write", directory.to_str().unwrap()], None);
    assert_eq!(write.status.code(), Some(0));
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "if (x)\n  value = 1;\nendif\n"
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_usage_and_invalid_formatting_have_distinct_statuses() {
    let conflict = run(&["format", "--check", "--write", "-"], None);
    assert_eq!(conflict.status.code(), Some(2));

    let invalid = run(&["format", "-"], Some("if (x)\nreturn;\n"));
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert!(
        String::from_utf8(invalid.stderr)
            .unwrap()
            .contains("error[unclosed-block]")
    );
}
