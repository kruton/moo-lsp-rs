// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range};
use unicode_width::UnicodeWidthChar;

use crate::{analysis, formatting};

const HELP: &str = "moo-lsp-rs - LambdaMOO language tools

Usage:
  moo-lsp-rs                         Run the stdio language server
  moo-lsp-rs lsp                     Run the stdio language server
  moo-lsp-rs check [OPTIONS] <INPUT>...
  moo-lsp-rs format [--check|--write] <INPUT>...

Inputs may be files, directories (searched recursively for *.moo), or - for stdin.

Check options:
  --json          Emit versioned JSON instead of human-readable diagnostics
  --deny-warnings Return exit status 1 when warnings are present

Format options:
  --check         Report files whose formatting differs without changing them
  --write         Format files in place (stdin is not accepted)

Exit status: 0 success, 1 diagnostics or formatting differences, 2 usage or I/O error.
";

#[derive(Debug)]
struct Source {
    name: String,
    path: Option<PathBuf>,
    text: String,
}

pub fn run(args: &[OsString]) -> i32 {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("lsp") if args.len() == 1 => run_lsp(),
        Some("check") => run_check(&args[1..]),
        Some("format") => run_format(&args[1..]),
        Some("help" | "--help" | "-h") if args.len() == 1 => {
            print!("{HELP}");
            0
        }
        Some("--version" | "-V") if args.len() == 1 => {
            println!("moo-lsp-rs {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some(command) => usage_error(format!("unknown command or extra argument: {command}")),
        None => usage_error("missing command"),
    }
}

fn run_lsp() -> i32 {
    let (connection, io_threads) = lsp_server::Connection::stdio();
    match crate::server::run(connection).and_then(|()| {
        io_threads
            .join()
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
    }) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("moo-lsp-rs: {error}");
            2
        }
    }
}

fn run_check(args: &[OsString]) -> i32 {
    let mut json = false;
    let mut deny_warnings = false;
    let mut inputs = Vec::new();
    let mut options = true;
    for arg in args {
        match arg.to_str() {
            Some("--") if options => options = false,
            Some("--json") if options => json = true,
            Some("--deny-warnings") if options => deny_warnings = true,
            Some("--help" | "-h") if options => {
                print!("{HELP}");
                return 0;
            }
            Some(value) if options && value.starts_with('-') && value != "-" => {
                return usage_error(format!("unknown check option: {value}"));
            }
            _ => inputs.push(arg.clone()),
        }
    }
    if inputs.is_empty() {
        return usage_error("check requires at least one input");
    }

    let sources = match load_sources(&inputs, true) {
        Ok(sources) => sources,
        Err(error) => return io_error(error),
    };
    let results: Vec<_> = sources
        .iter()
        .map(|source| (source, analysis::diagnostics(&source.text)))
        .collect();

    let mut errors = 0usize;
    let mut warnings = 0usize;
    for (_, diagnostics) in &results {
        for diagnostic in diagnostics {
            if diagnostic.severity == Some(DiagnosticSeverity::WARNING) {
                warnings += 1;
            } else {
                errors += 1;
            }
        }
    }

    if json {
        let files: Vec<_> = results
            .iter()
            .map(|(source, diagnostics)| {
                serde_json::json!({
                    "path": source.name,
                    "diagnostics": diagnostics.iter().map(diagnostic_json).collect::<Vec<_>>()
                })
            })
            .collect();
        let value = serde_json::json!({
            "version": 1,
            "files": files,
            "summary": {
                "files": results.len(),
                "errors": errors,
                "warnings": warnings
            }
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("JSON value serializes")
        );
    } else {
        let mut stderr = io::stderr().lock();
        for (source, diagnostics) in &results {
            for diagnostic in diagnostics {
                let _ = write_human_diagnostic(&mut stderr, source, diagnostic);
            }
        }
    }

    i32::from(errors > 0 || (deny_warnings && warnings > 0))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormatMode {
    Stdout,
    Check,
    Write,
}

fn run_format(args: &[OsString]) -> i32 {
    let mut mode = FormatMode::Stdout;
    let mut mode_set = false;
    let mut inputs = Vec::new();
    let mut options = true;
    for arg in args {
        match arg.to_str() {
            Some("--") if options => options = false,
            Some("--check") if options && !mode_set => {
                mode = FormatMode::Check;
                mode_set = true;
            }
            Some("--write") if options && !mode_set => {
                mode = FormatMode::Write;
                mode_set = true;
            }
            Some("--check" | "--write") if options => {
                return usage_error("--check and --write are mutually exclusive");
            }
            Some("--help" | "-h") if options => {
                print!("{HELP}");
                return 0;
            }
            Some(value) if options && value.starts_with('-') && value != "-" => {
                return usage_error(format!("unknown format option: {value}"));
            }
            _ => inputs.push(arg.clone()),
        }
    }
    if inputs.is_empty() {
        return usage_error("format requires at least one input");
    }
    if mode == FormatMode::Stdout && inputs.len() != 1 {
        return usage_error("format without --check or --write requires exactly one input");
    }
    if mode == FormatMode::Stdout {
        let path = PathBuf::from(&inputs[0]);
        if path != Path::new("-") && path.is_dir() {
            return usage_error("format-to-stdout does not accept directories");
        }
    }
    if mode == FormatMode::Write && inputs.iter().any(|input| input == "-") {
        return usage_error("format --write does not accept stdin");
    }

    let sources = match load_sources(&inputs, mode != FormatMode::Stdout) {
        Ok(sources) => sources,
        Err(error) => return io_error(error),
    };
    let mut failed = false;
    for source in sources {
        let Some(formatted) = formatting::format(&source.text) else {
            let diagnostics = analysis::diagnostics(&source.text);
            let mut stderr = io::stderr().lock();
            for diagnostic in &diagnostics {
                let _ = write_human_diagnostic(&mut stderr, &source, diagnostic);
            }
            if diagnostics.is_empty() {
                eprintln!("{}: cannot format invalid LambdaMOO source", source.name);
            }
            failed = true;
            continue;
        };

        match mode {
            FormatMode::Stdout => {
                if let Err(error) = io::stdout().lock().write_all(formatted.as_bytes()) {
                    return io_error(error);
                }
            }
            FormatMode::Check if formatted != source.text => {
                eprintln!("{}: needs formatting", source.name);
                failed = true;
            }
            FormatMode::Write if formatted != source.text => {
                let Some(path) = source.path else {
                    return usage_error("format --write does not accept stdin");
                };
                if let Err(error) = fs::write(&path, formatted) {
                    return io_error(format!("{}: {error}", path.display()));
                }
            }
            _ => {}
        }
    }
    i32::from(failed)
}

fn load_sources(inputs: &[OsString], allow_directories: bool) -> Result<Vec<Source>, String> {
    let mut paths = BTreeSet::new();
    let mut stdin_requested = false;
    for input in inputs {
        if input == "-" {
            if stdin_requested {
                return Err("stdin may be specified only once".to_owned());
            }
            stdin_requested = true;
            continue;
        }
        let path = PathBuf::from(input);
        if path.is_dir() {
            if !allow_directories {
                return Err(format!(
                    "{}: directory input is not allowed",
                    path.display()
                ));
            }
            collect_moo_files(&path, &mut paths)?;
        } else {
            paths.insert(path);
        }
    }

    let mut sources = Vec::new();
    for path in paths {
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        sources.push(Source {
            name: path.display().to_string(),
            path: Some(path),
            text,
        });
    }
    if stdin_requested {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("<stdin>: {error}"))?;
        sources.push(Source {
            name: "<stdin>".to_owned(),
            path: None,
            text,
        });
    }
    Ok(sources)
}

fn collect_moo_files(directory: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        let path = entry.path();
        if file_type.is_dir() && !file_type.is_symlink() {
            collect_moo_files(&path, paths)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "moo") {
            paths.insert(path);
        }
    }
    Ok(())
}

fn diagnostic_json(diagnostic: &Diagnostic) -> serde_json::Value {
    serde_json::json!({
        "severity": severity_name(diagnostic),
        "code": diagnostic.code.as_ref().map(code_string),
        "message": diagnostic.message,
        "range": range_json(diagnostic.range),
        "relatedInformation": diagnostic.related_information.as_ref().map(|items| {
            items.iter().map(|item| serde_json::json!({
                "message": item.message,
                "uri": item.location.uri.as_str(),
                "range": range_json(item.location.range)
            })).collect::<Vec<_>>()
        }).unwrap_or_default()
    })
}

fn range_json(range: Range) -> serde_json::Value {
    serde_json::json!({
        "start": { "line": range.start.line, "character": range.start.character },
        "end": { "line": range.end.line, "character": range.end.character }
    })
}

fn severity_name(diagnostic: &Diagnostic) -> &'static str {
    match diagnostic.severity {
        Some(DiagnosticSeverity::WARNING) => "warning",
        Some(DiagnosticSeverity::INFORMATION) => "information",
        Some(DiagnosticSeverity::HINT) => "hint",
        _ => "error",
    }
}

fn code_string(code: &NumberOrString) -> String {
    match code {
        NumberOrString::Number(number) => number.to_string(),
        NumberOrString::String(string) => string.clone(),
    }
}

fn write_human_diagnostic(
    out: &mut impl Write,
    source: &Source,
    diagnostic: &Diagnostic,
) -> io::Result<()> {
    let start = diagnostic.range.start;
    let code = diagnostic
        .code
        .as_ref()
        .map(|code| format!("[{}]", code_string(code)))
        .unwrap_or_default();
    writeln!(
        out,
        "{}:{}:{}: {}{}: {}",
        source.name,
        start.line + 1,
        start.character + 1,
        severity_name(diagnostic),
        code,
        diagnostic.message
    )?;
    write_excerpt(out, &source.text, diagnostic.range)?;
    if let Some(related) = &diagnostic.related_information {
        for item in related {
            writeln!(out, "note: {}", item.message)?;
            write_excerpt(out, &source.text, item.location.range)?;
        }
    }
    Ok(())
}

fn write_excerpt(out: &mut impl Write, text: &str, range: Range) -> io::Result<()> {
    let Some(raw_line) = text.lines().nth(range.start.line as usize) else {
        return Ok(());
    };
    let line = expand_tabs(raw_line, 4);
    let start_byte = utf16_to_byte(raw_line, range.start.character as usize);
    let prefix_width = display_width_with_tabs(&raw_line[..start_byte], 4);
    let marker_width = if range.start.line == range.end.line {
        let end_byte = utf16_to_byte(raw_line, range.end.character as usize);
        display_width_with_tabs(&raw_line[start_byte..end_byte], 4).max(1)
    } else {
        1
    };
    let line_number = range.start.line + 1;
    let gutter = line_number.to_string().len();
    writeln!(out, "{line_number:>gutter$} | {line}")?;
    writeln!(
        out,
        "{:>gutter$} | {}{}",
        "",
        "-".repeat(prefix_width),
        "^".repeat(marker_width)
    )
}

fn utf16_to_byte(text: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units >= utf16_offset {
            return byte;
        }
        units += character.len_utf16();
        if units > utf16_offset {
            return byte;
        }
    }
    text.len()
}

fn display_width_with_tabs(text: &str, tab_width: usize) -> usize {
    text.chars().fold(0, |column, character| {
        if character == '\t' {
            column + tab_width - (column % tab_width)
        } else {
            column + character.width().unwrap_or(0)
        }
    })
}

fn expand_tabs(text: &str, tab_width: usize) -> String {
    let mut output = String::new();
    let mut column = 0;
    for character in text.chars() {
        if character == '\t' {
            let spaces = tab_width - (column % tab_width);
            output.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            output.push(character);
            column += character.width().unwrap_or(0);
        }
    }
    output
}

fn usage_error(message: impl std::fmt::Display) -> i32 {
    eprintln!("moo-lsp-rs: {message}\nTry 'moo-lsp-rs --help' for more information.");
    2
}

fn io_error(error: impl std::fmt::Display) -> i32 {
    eprintln!("moo-lsp-rs: {error}");
    2
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Position;

    #[test]
    fn utf16_offsets_convert_to_byte_offsets() {
        assert_eq!(utf16_to_byte("a😀b", 0), 0);
        assert_eq!(utf16_to_byte("a😀b", 1), 1);
        assert_eq!(utf16_to_byte("a😀b", 3), 5);
        assert_eq!(utf16_to_byte("a😀b", 4), 6);
    }

    #[test]
    fn tabs_and_wide_characters_have_display_width() {
        assert_eq!(display_width_with_tabs("a\tb", 4), 5);
        assert_eq!(display_width_with_tabs("界", 4), 2);
        assert_eq!(display_width_with_tabs("e\u{301}", 4), 1);
        assert_eq!(expand_tabs("a\tb", 4), "a   b");
    }

    #[test]
    fn excerpt_aligns_utf16_ranges_with_expanded_source() {
        let mut output = Vec::new();
        write_excerpt(
            &mut output,
            "\t界😀x;\n",
            Range::new(Position::new(0, 4), Position::new(0, 5)),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(output, "1 |     界😀x;\n  | --------^\n");
    }
}
