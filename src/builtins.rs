// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

//! LambdaMOO built-in signatures and syntax-local type checking.

use crate::line_index::{LineIndex, safe_slice};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, Hover, HoverContents, MarkupContent, MarkupKind,
    NumberOrString, ParameterInformation, ParameterLabel, Position, Range, SignatureHelp,
    SignatureInformation,
};
use tree_sitter::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MooType {
    Any,
    Obj,
    Str,
    Int,
    Float,
    Numeric,
    List,
    Err,
    Waif,
}

impl MooType {
    fn label(self) -> &'static str {
        match self {
            Self::Any => "ANY",
            Self::Obj => "OBJ",
            Self::Str => "STR",
            Self::Int => "INT",
            Self::Float => "FLOAT",
            Self::Numeric => "NUMERIC",
            Self::List => "LIST",
            Self::Err => "ERR",
            Self::Waif => "WAIF",
        }
    }

    fn accepts(self, actual: Self) -> bool {
        self == Self::Any
            || self == actual
            || (self == Self::Numeric && matches!(actual, Self::Int | Self::Float))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    pub name: &'static str,
    pub min: usize,
    pub max: Option<usize>,
    pub args: &'static [MooType],
    pub returns: Option<MooType>,
    pub parameter_names: &'static [&'static str],
}

use MooType::*;
macro_rules! b {
    ($n:literal,$min:literal,$max:expr,[$($t:ident),*]) => {
        Builtin { name:$n, min:$min, max:$max, args:&[$($t),*], returns:None, parameter_names:&[] }
    };
    ($n:literal,$min:literal,$max:expr,[$($t:ident),*] => $r:ident) => {
        Builtin { name:$n, min:$min, max:$max, args:&[$($t),*], returns:Some($r), parameter_names:&[] }
    };
    ($n:literal,$min:literal,$max:expr,[$($t:ident),*] => $r:ident; [$($p:literal),*]) => {
        Builtin { name:$n, min:$min, max:$max, args:&[$($t),*], returns:Some($r), parameter_names:&[$($p),*] }
    };
}

pub static BUILTINS: &[Builtin] = &[
    b!("log_cache_stats", 0, Some(0), []),
    b!("verb_cache_stats", 0, Some(0), []),
    b!("disassemble", 2, Some(2), [Obj, Any]),
    b!("call_function", 1, None, [Str]),
    b!("raise", 1, Some(3), [Any, Str, Any]),
    b!("suspend", 0, Some(1), [Int]),
    b!("read", 0, Some(2), [Obj, Any]),
    b!("seconds_left", 0, Some(0), []),
    b!("ticks_left", 0, Some(0), []),
    b!("pass", 0, None, []),
    b!("set_task_perms", 1, Some(1), [Obj]),
    b!("caller_perms", 0, Some(0), []),
    b!("callers", 0, Some(1), [Any]),
    b!("task_stack", 1, Some(2), [Int, Any]),
    b!("read_stdin", 0, Some(0), []),
    b!("function_info", 0, Some(1), [Str]),
    b!("load_server_options", 0, Some(0), []),
    b!("value_bytes", 1, Some(1), [Any]),
    b!("value_hash", 1, Some(1), [Any]),
    b!("string_hash", 1, Some(1), [Str]),
    b!("binary_hash", 1, Some(1), [Str]),
    b!("decode_binary", 1, Some(2), [Str, Any]),
    b!("encode_binary", 0, None, []),
    b!("length", 1, Some(1), [Any] => Int; ["value"]),
    b!("setadd", 2, Some(2), [List, Any]),
    b!("setremove", 2, Some(2), [List, Any]),
    b!("listappend", 2, Some(3), [List, Any, Int]),
    b!("listinsert", 2, Some(3), [List, Any, Int]),
    b!("listdelete", 2, Some(2), [List, Int]),
    b!("listset", 3, Some(3), [List, Any, Int]),
    b!("equal", 2, Some(2), [Any, Any]),
    b!("is_member", 2, Some(2), [Any, List] => Int; ["value", "list"]),
    b!("tostr", 0, None, [] => Str; []),
    b!("toliteral", 1, Some(1), [Any] => Str; ["value"]),
    b!("match", 2, Some(3), [Str, Str, Any]),
    b!("rmatch", 2, Some(3), [Str, Str, Any]),
    b!("substitute", 2, Some(2), [Str, List]),
    b!("crypt", 1, Some(2), [Str, Str]),
    b!("index", 2, Some(3), [Str, Str, Any]),
    b!("rindex", 2, Some(3), [Str, Str, Any]),
    b!("strcmp", 2, Some(2), [Str, Str]),
    b!("strsub", 3, Some(4), [Str, Str, Str, Any]),
    b!("tochar", 1, Some(1), [Any] => Str; ["value"]),
    b!("charname", 1, Some(1), [Str] => Str; ["character"]),
    b!("ord", 1, Some(1), [Str] => Int; ["character"]),
    b!("encode_chars", 2, Some(2), [Any, Str]),
    b!("decode_chars", 2, Some(3), [Str, Str, Any]),
    b!("server_log", 1, Some(2), [Str, Any]),
    b!("toint", 1, Some(1), [Any] => Int; ["value"]),
    b!("tonum", 1, Some(1), [Any] => Int; ["value"]),
    b!("tofloat", 1, Some(1), [Any] => Float; ["value"]),
    b!("min", 1, None, [Numeric]),
    b!("max", 1, None, [Numeric]),
    b!("abs", 1, Some(1), [Numeric]),
    b!("random", 0, Some(1), [Int] => Int),
    b!("time", 0, Some(0), [] => Int),
    b!("ftime", 0, Some(0), [] => Float),
    b!("ctime", 0, Some(2), [Int, Str] => Str),
    b!("floatstr", 2, Some(3), [Float, Int, Any] => Str),
    b!("sqrt", 1, Some(1), [Float]),
    b!("sin", 1, Some(1), [Float]),
    b!("cos", 1, Some(1), [Float]),
    b!("tan", 1, Some(1), [Float]),
    b!("asin", 1, Some(1), [Float]),
    b!("acos", 1, Some(1), [Float]),
    b!("atan", 1, Some(2), [Float, Float]),
    b!("sinh", 1, Some(1), [Float]),
    b!("cosh", 1, Some(1), [Float]),
    b!("tanh", 1, Some(1), [Float]),
    b!("asinh", 1, Some(1), [Float]),
    b!("acosh", 1, Some(1), [Float]),
    b!("atanh", 1, Some(1), [Float]),
    b!("exp", 1, Some(1), [Float]),
    b!("log", 1, Some(1), [Float]),
    b!("log10", 1, Some(1), [Float]),
    b!("ceil", 1, Some(1), [Float]),
    b!("floor", 1, Some(1), [Float]),
    b!("trunc", 1, Some(1), [Float]),
    b!("expm1", 1, Some(1), [Float]),
    b!("log1p", 1, Some(1), [Float]),
    b!("erf", 1, Some(1), [Float]),
    b!("erfc", 1, Some(1), [Float]),
    b!("lgamma", 1, Some(1), [Float]),
    b!("j", 2, Some(2), [Int, Float]),
    b!("y", 2, Some(2), [Int, Float]),
    b!("toobj", 1, Some(1), [Any] => Obj; ["value"]),
    b!("typeof", 1, Some(1), [Any] => Int; ["value"]),
    b!("create", 1, Some(2), [Obj, Obj] => Obj),
    b!("recycle", 1, Some(1), [Obj]),
    b!("object_bytes", 1, Some(1), [Obj]),
    b!("valid", 1, Some(1), [Obj]),
    b!("parent", 1, Some(1), [Obj]),
    b!("children", 1, Some(1), [Obj]),
    b!("chparent", 2, Some(2), [Obj, Obj]),
    b!("max_object", 0, Some(0), []),
    b!("players", 0, Some(0), []),
    b!("is_player", 1, Some(1), [Obj]),
    b!("set_player_flag", 2, Some(2), [Obj, Any]),
    b!("move", 2, Some(2), [Obj, Obj]),
    b!("properties", 1, Some(1), [Obj]),
    b!("property_info", 2, Some(2), [Obj, Str]),
    b!("set_property_info", 3, Some(3), [Obj, Str, List]),
    b!("add_property", 4, Some(4), [Obj, Str, Any, List]),
    b!("delete_property", 2, Some(2), [Obj, Str]),
    b!("clear_property", 2, Some(2), [Obj, Str]),
    b!("is_clear_property", 2, Some(2), [Obj, Str]),
    b!("server_version", 0, Some(1), [Any]),
    b!("renumber", 1, Some(1), [Obj]),
    b!("reset_max_object", 0, Some(0), []),
    b!("memory_usage", 0, Some(0), []),
    b!("shutdown", 0, Some(1), [Str]),
    b!("dump_database", 0, Some(0), []),
    b!("db_disk_size", 0, Some(0), []),
    b!("open_network_connection", 0, None, []),
    b!("connected_players", 0, Some(1), [Any]),
    b!("connected_seconds", 1, Some(1), [Obj]),
    b!("idle_seconds", 1, Some(1), [Obj]),
    b!("connection_name", 1, Some(1), [Obj]),
    b!("notify", 2, Some(3), [Obj, Str, Any]),
    b!("boot_player", 1, Some(1), [Obj]),
    b!("set_connection_option", 3, Some(3), [Obj, Str, Any]),
    b!("connection_option", 2, Some(2), [Obj, Str]),
    b!("connection_options", 1, Some(1), [Obj]),
    b!("listen", 2, Some(3), [Obj, Any, Any]),
    b!("unlisten", 1, Some(1), [Any]),
    b!("listeners", 0, Some(0), []),
    b!("buffered_output_length", 0, Some(1), [Obj]),
    b!("task_id", 0, Some(0), []),
    b!("queued_tasks", 0, Some(0), []),
    b!("kill_task", 1, Some(1), [Int]),
    b!("output_delimiters", 1, Some(1), [Obj]),
    b!("queue_info", 0, Some(1), [Obj]),
    b!("resume", 1, Some(2), [Int, Any]),
    b!("force_input", 2, Some(3), [Obj, Str, Any]),
    b!("flush_input", 1, Some(2), [Obj, Any]),
    b!("verbs", 1, Some(1), [Obj]),
    b!("verb_info", 2, Some(2), [Obj, Any]),
    b!("set_verb_info", 3, Some(3), [Obj, Any, List]),
    b!("verb_args", 2, Some(2), [Obj, Any]),
    b!("set_verb_args", 3, Some(3), [Obj, Any, List]),
    b!("add_verb", 3, Some(3), [Obj, List, List]),
    b!("delete_verb", 2, Some(2), [Obj, Any]),
    b!("verb_code", 2, Some(4), [Obj, Any, Any, Any]),
    b!("set_verb_code", 3, Some(3), [Obj, Any, List]),
    b!("eval", 1, Some(1), [Str]),
    b!("new_waif", 0, Some(0), [] => Waif),
    b!("xml_parse_tree", 1, Some(1), [Str]),
    b!("xml_parse_document", 1, Some(1), [Str]),
];

pub fn find(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name.eq_ignore_ascii_case(name))
}

impl Builtin {
    pub fn signature(&self) -> String {
        let mut parts = Vec::new();
        for (i, ty) in self.args.iter().enumerate() {
            let name = self
                .parameter_names
                .get(i)
                .copied()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("arg{}", i + 1));
            let part = format!("{name}: {}", ty.label());
            parts.push(if i >= self.min {
                format!("[{part}]")
            } else {
                part
            });
        }
        if self.max.is_none() {
            parts.push("...ANY".to_owned());
        }
        let signature = format!("{}({})", self.name, parts.join(", "));
        match self.returns {
            Some(returns) => format!("{signature} -> {}", returns.label()),
            None => signature,
        }
    }
}

fn node_range(node: Node, index: &LineIndex, text: &str) -> Range {
    index.clamp_range(
        text,
        node.start_position().row,
        node.start_position().column,
        node.end_position().row,
        node.end_position().column,
    )
}

fn diagnostic(
    node: Node,
    index: &LineIndex,
    text: &str,
    severity: DiagnosticSeverity,
    code: &str,
    message: String,
) -> Diagnostic {
    Diagnostic {
        range: node_range(node, index, text),
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_owned())),
        code_description: None,
        source: Some("moo-lsp-rs".to_owned()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

fn arg_nodes(call: Node) -> Vec<Node> {
    let Some(args) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut cursor = args.walk();
    args.named_children(&mut cursor).collect()
}

fn expression_of_arg(arg: Node) -> Option<Node> {
    arg.named_child(0).or(Some(arg))
}

fn intrinsic_type(name: &str) -> Option<MooType> {
    match name.to_ascii_lowercase().as_str() {
        "player" | "dobj" | "iobj" => Some(Obj),
        "args" => Some(List),
        "argstr" | "dobjstr" | "iobjstr" | "prepstr" | "verb" => Some(Str),
        _ => None,
    }
}

fn infer_type(node: Node, root: Node, text: &str, before: usize, depth: usize) -> Option<MooType> {
    if depth > 8 {
        return None;
    }
    match node.kind() {
        "object" => Some(Obj),
        "string" => Some(Str),
        "list_literal" => Some(List),
        "error" => Some(Err),
        "number" => Some(if safe_slice(text, node.byte_range()).contains('.') {
            Float
        } else {
            Int
        }),
        "identifier" => {
            let name = safe_slice(text, node.byte_range());
            intrinsic_type(name)
                .or_else(|| find_assignment_type(root, text, name, before, depth + 1))
        }
        "expression" | "arg_item" => {
            infer_type(node.named_child(0)?, root, text, before, depth + 1)
        }
        "call_expression" => {
            let function = node.child_by_field_name("function")?;
            find(safe_slice(text, function.byte_range()))?.returns
        }
        _ => None,
    }
}

fn find_assignment_type(
    root: Node,
    text: &str,
    name: &str,
    before: usize,
    depth: usize,
) -> Option<MooType> {
    let mut stack = vec![root];
    let mut found = None;
    while let Some(node) = stack.pop() {
        if node.start_byte() >= before {
            continue;
        }
        if node.kind() == "assignment"
            && node.end_byte() <= before
            && let (Some(lhs), Some(rhs)) = (node.named_child(0), node.named_child(1))
            && lhs.kind() == "identifier"
            && safe_slice(text, lhs.byte_range()).eq_ignore_ascii_case(name)
        {
            found = infer_type(rhs, root, text, node.start_byte(), depth);
        }
        let mut cursor = node.walk();
        let mut children: Vec<_> = node.named_children(&mut cursor).collect();
        children.reverse();
        stack.extend(children);
    }
    found
}

pub fn collect_diagnostics(root: Node, index: &LineIndex, text: &str, out: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(call) = stack.pop() {
        let mut cursor = call.walk();
        stack.extend(call.named_children(&mut cursor));
        if call.kind() != "call_expression" || call.has_error() {
            continue;
        }
        let Some(function) = call.child_by_field_name("function") else {
            continue;
        };
        if function.kind() == "invalid_identifier" {
            continue;
        }
        let name = safe_slice(text, function.byte_range());
        let Some(builtin) = find(name) else {
            out.push(diagnostic(
                function,
                index,
                text,
                DiagnosticSeverity::WARNING,
                "unknown-builtin",
                format!("Unknown built-in function '{name}'"),
            ));
            continue;
        };
        let args = arg_nodes(call);
        let has_splice = args.iter().any(|a| {
            safe_slice(text, a.byte_range())
                .trim_start()
                .starts_with('@')
        });
        let known_count = args
            .iter()
            .filter(|arg| {
                !safe_slice(text, arg.byte_range())
                    .trim_start()
                    .starts_with('@')
            })
            .count();
        let wrong_count = if has_splice {
            builtin.max.is_some_and(|max| known_count > max)
        } else {
            args.len() < builtin.min || builtin.max.is_some_and(|max| args.len() > max)
        };
        if wrong_count {
            let expected = match builtin.max {
                Some(max) if max == builtin.min => builtin.min.to_string(),
                Some(max) => format!("{} to {max}", builtin.min),
                None => format!("at least {}", builtin.min),
            };
            out.push(diagnostic(
                call,
                index,
                text,
                DiagnosticSeverity::ERROR,
                "builtin-argument-count",
                format!(
                    "{}() expects {expected} arguments, but {} were provided",
                    builtin.name,
                    if has_splice { known_count } else { args.len() }
                ),
            ));
            continue;
        }
        let mut positions_certain = true;
        for (i, arg) in args.iter().enumerate() {
            if safe_slice(text, arg.byte_range())
                .trim_start()
                .starts_with('@')
            {
                positions_certain = false;
                continue;
            }
            if !positions_certain {
                continue;
            }
            let Some(expected) = builtin.args.get(i).copied() else {
                continue;
            };
            if expected == Any {
                continue;
            }
            let Some(expr) = expression_of_arg(*arg) else {
                continue;
            };
            if let Some(actual) = infer_type(expr, root, text, call.start_byte(), 0)
                && !expected.accepts(actual)
            {
                out.push(diagnostic(
                    expr,
                    index,
                    text,
                    DiagnosticSeverity::ERROR,
                    "builtin-argument-type",
                    format!(
                        "Argument {} to {}() expects {}, but this expression is {}",
                        i + 1,
                        builtin.name,
                        expected.label(),
                        actual.label()
                    ),
                ));
            }
        }
    }
}

fn byte_offset(text: &str, pos: Position) -> Option<usize> {
    let start = text
        .split_inclusive('\n')
        .take(pos.line as usize)
        .map(str::len)
        .sum::<usize>();
    let line = text.get(start..)?.split('\n').next()?;
    let mut units = 0usize;
    for (i, ch) in line.char_indices() {
        if units == pos.character as usize {
            return Some(start + i);
        }
        units += ch.len_utf16();
        if units > pos.character as usize {
            return None;
        }
    }
    (units == pos.character as usize).then_some(start + line.len())
}

fn call_at(root: Node, offset: usize) -> Option<Node> {
    let mut node = root.descendant_for_byte_range(offset.saturating_sub(1), offset)?;
    loop {
        if node.kind() == "call_expression" {
            return Some(node);
        }
        node = node.parent()?;
    }
}

pub fn hover(root: Node, text: &str, position: Position) -> Option<Hover> {
    let offset = byte_offset(text, position)?;
    let call = call_at(root, offset)?;
    let function = call.child_by_field_name("function")?;
    let builtin = find(safe_slice(text, function.byte_range()))?;
    let index = LineIndex::new(text);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```moo\n{}\n```", builtin.signature()),
        }),
        range: Some(node_range(function, &index, text)),
    })
}

pub fn signature_help(root: Node, text: &str, position: Position) -> Option<SignatureHelp> {
    let offset = byte_offset(text, position)?;
    let call = call_at(root, offset)?;
    let function = call.child_by_field_name("function")?;
    let builtin = find(safe_slice(text, function.byte_range()))?;
    let args = arg_nodes(call);
    let raw_active = args
        .iter()
        .position(|a| offset <= a.end_byte())
        .unwrap_or(args.len()) as u32;
    let mut parameters: Vec<_> = builtin
        .args
        .iter()
        .enumerate()
        .map(|(i, t)| ParameterInformation {
            label: ParameterLabel::Simple(format!(
                "{}: {}",
                builtin
                    .parameter_names
                    .get(i)
                    .copied()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("arg{}", i + 1)),
                t.label()
            )),
            documentation: None,
        })
        .collect();
    if builtin.max.is_none() {
        parameters.push(ParameterInformation {
            label: ParameterLabel::Simple("...ANY".to_owned()),
            documentation: None,
        });
    }
    let active = (!parameters.is_empty()).then(|| raw_active.min(parameters.len() as u32 - 1));
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: builtin.signature(),
            documentation: None,
            parameters: (!parameters.is_empty()).then_some(parameters),
            active_parameter: active,
        }],
        active_signature: Some(0),
        active_parameter: active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    #[test]
    fn includes_extensions_and_core_signatures() {
        assert_eq!(BUILTINS.len(), 148);
        assert_eq!(
            find("NOTIFY").unwrap().signature(),
            "notify(arg1: OBJ, arg2: STR, [arg3: ANY])"
        );
        assert!(find("new_waif").is_some());
        assert!(find("xml_parse_tree").is_some());
    }
    #[test]
    fn validates_known_types_and_arity() {
        let text = "notify(#1, 4); notify(player, \"hi\"); notify(player);";
        let tree = parser::parse(text).unwrap();
        let mut d = vec![];
        collect_diagnostics(tree.root_node(), &LineIndex::new(text), text, &mut d);
        assert!(
            d.iter()
                .any(|d| d.code == Some(NumberOrString::String("builtin-argument-type".into())))
        );
        assert!(
            d.iter()
                .any(|d| d.code == Some(NumberOrString::String("builtin-argument-count".into())))
        );
    }

    #[test]
    fn infers_simple_assignments_and_warns_about_unknown_calls() {
        let text = "target = #1; message = 4; notify(target, message); custom(); thing:custom();";
        let tree = parser::parse(text).unwrap();
        let mut diagnostics = Vec::new();
        collect_diagnostics(
            tree.root_node(),
            &LineIndex::new(text),
            text,
            &mut diagnostics,
        );

        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d.code == Some(NumberOrString::String("builtin-argument-type".into())))
                .count(),
            1
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d.code == Some(NumberOrString::String("unknown-builtin".into())))
                .count(),
            1
        );
    }

    #[test]
    fn provides_hover_and_signature_help() {
        let text = "notify(player, \"hi\");";
        let tree = parser::parse(text).unwrap();

        let hover = hover(tree.root_node(), text, Position::new(0, 2)).unwrap();
        let HoverContents::Markup(contents) = hover.contents else {
            panic!("expected markdown hover");
        };
        assert!(contents.value.contains("notify(arg1: OBJ, arg2: STR"));

        let help = signature_help(tree.root_node(), text, Position::new(0, 16)).unwrap();
        assert_eq!(help.active_parameter, Some(1));
        assert_eq!(help.signatures[0].parameters.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn builtin_returns_feed_type_inference_and_signatures() {
        assert_eq!(
            find("toint").unwrap().signature(),
            "toint(value: ANY) -> INT"
        );
        assert_eq!(
            find("is_member").unwrap().signature(),
            "is_member(value: ANY, list: LIST) -> INT"
        );
        assert_eq!(find("tostr").unwrap().returns, Some(Str));
        assert_eq!(find("tofloat").unwrap().returns, Some(Float));

        let text = concat!(
            "number = toint(\"4\"); notify(player, number); ",
            "notify(player, tostr(4));"
        );
        let tree = parser::parse(text).unwrap();
        let mut diagnostics = Vec::new();
        collect_diagnostics(
            tree.root_node(),
            &LineIndex::new(text),
            text,
            &mut diagnostics,
        );

        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code == Some(NumberOrString::String("builtin-argument-type".into()))
                })
                .count(),
            1
        );
    }
}
