// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

//! Remote LambdaMOO document URI handling and verb header documentation.

use std::str::FromStr;

use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Uri};

use crate::{parser, remote_navigation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerbTarget {
    pub uri: Uri,
    pub resolution_uri: Uri,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerbHeader {
    pub comments: Vec<String>,
    pub arguments: Option<String>,
}

pub fn canonical_owned_uri(uri: &Uri) -> Uri {
    let raw = uri.as_str();
    let Some(rest) = raw.strip_prefix("moo://") else {
        return uri.clone();
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return uri.clone();
    };
    let Some(owned) = path.strip_prefix("owned/") else {
        return uri.clone();
    };
    let split = owned.find('/').unwrap_or(owned.len());
    let (number, suffix) = owned.split_at(split);
    if number.parse::<i64>().is_err() {
        return uri.clone();
    }
    Uri::from_str(&format!("moo://{authority}/object/{number}{suffix}"))
        .unwrap_or_else(|_| uri.clone())
}

pub fn verb_target(uri: &Uri) -> Option<VerbTarget> {
    let uri = canonical_owned_uri(uri);
    let raw = uri.as_str();
    let (prefix, verb) = raw.rsplit_once("/verb/")?;
    if verb.is_empty() || verb.contains('/') {
        return None;
    }
    let resolution_uri = Uri::from_str(&format!("{prefix}/resolve/verb/{verb}/defined-on")).ok()?;
    Some(VerbTarget {
        uri,
        resolution_uri,
    })
}

pub fn resolve_verb_uri(target: &VerbTarget, contents: &str) -> Option<Uri> {
    let number = contents.trim().strip_prefix('#').unwrap_or(contents.trim());
    number.parse::<i64>().ok()?;
    let raw = target.uri.as_str().strip_prefix("moo://")?;
    let (authority, path) = raw.split_once('/')?;
    let verb = path.rsplit_once("/verb/")?.1;
    Uri::from_str(&format!("moo://{authority}/object/{number}/verb/{verb}")).ok()
}

pub fn verb_header(source: &str) -> VerbHeader {
    let Some(tree) = parser::parse(source) else {
        return VerbHeader::default();
    };
    let mut result = VerbHeader::default();
    let mut cursor = tree.root_node().walk();
    for statement in tree.root_node().named_children(&mut cursor) {
        let Some(expression) = statement.named_child(0) else {
            break;
        };
        let expression = unwrap_expression(expression);
        if expression.kind() == "string"
            && result.arguments.is_none()
            && let Some(comment) = remote_navigation::parse_string_literal(
                source.get(expression.byte_range()).unwrap_or_default(),
            )
        {
            result.comments.push(comment.trim().to_owned());
            continue;
        }
        if expression.kind() == "scattering_assignment"
            && result.arguments.is_none()
            && scattering_rhs_is_args(expression, source)
        {
            result.arguments = source
                .get(statement.byte_range())
                .map(str::trim)
                .map(ToOwned::to_owned);
            continue;
        }
        break;
    }
    result
}

pub fn hover(uri: &Uri, source: Option<&str>, resolved: bool) -> Hover {
    let header = source.map(verb_header).unwrap_or_default();
    let mut parts = Vec::new();
    if let Some(arguments) = header.arguments {
        parts.push(format!("```moo\n{arguments}\n```"));
    }
    if !header.comments.is_empty() {
        parts.push(header.comments.join("\n"));
    }
    parts.push(format!(
        "{}: `{}`",
        if resolved { "Defined on" } else { "Target" },
        uri.as_str()
    ));
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: parts.join("\n\n"),
        }),
        range: None,
    }
}

fn unwrap_expression(mut node: tree_sitter::Node<'_>) -> tree_sitter::Node<'_> {
    while matches!(node.kind(), "expression_statement" | "expression")
        && node.named_child_count() == 1
    {
        node = node.named_child(0).unwrap_or(node);
    }
    node
}

fn scattering_rhs_is_args(node: tree_sitter::Node<'_>, source: &str) -> bool {
    let Some(rhs) = node.named_child(node.named_child_count().saturating_sub(1) as u32) else {
        return false;
    };
    let rhs = unwrap_expression(rhs);
    rhs.kind() == "identifier"
        && source
            .get(rhs.byte_range())
            .is_some_and(|value| value.eq_ignore_ascii_case("args"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(value: &str) -> Uri {
        value.parse().unwrap()
    }

    #[test]
    fn canonicalizes_owned_objects() {
        assert_eq!(
            canonical_owned_uri(&uri("moo://waterpoint/owned/-1/verb/look")).as_str(),
            "moo://waterpoint/object/-1/verb/look"
        );
        assert_eq!(
            canonical_owned_uri(&uri("moo://waterpoint/owned")).as_str(),
            "moo://waterpoint/owned"
        );
    }

    #[test]
    fn resolves_property_chain_verbs() {
        let target = verb_target(&uri(
            "moo://waterpoint/object/0/property/string_utils/object/verb/explode",
        ))
        .unwrap();
        assert_eq!(
            target.resolution_uri.as_str(),
            "moo://waterpoint/object/0/property/string_utils/object/resolve/verb/explode/defined-on"
        );
        assert_eq!(
            resolve_verb_uri(&target, "#18\n").unwrap().as_str(),
            "moo://waterpoint/object/18/verb/explode"
        );
        assert!(resolve_verb_uri(&target, "not an object").is_none());
    }

    #[test]
    fn extracts_leading_verb_documentation() {
        let header = verb_header(
            r#""Syntax: \"foo\" \\ bar";
"Returns a value.";
{x, y, ?z = 3} = args;
return x;
"not documentation";"#,
        );
        assert_eq!(
            header.comments,
            ["Syntax: \"foo\" \\ bar", "Returns a value."]
        );
        assert_eq!(header.arguments.as_deref(), Some("{x, y, ?z = 3} = args;"));
    }

    #[test]
    fn supports_arguments_without_comments() {
        let header = verb_header("{x, y, z} = args;\nreturn x;");
        assert!(header.comments.is_empty());
        assert_eq!(header.arguments.as_deref(), Some("{x, y, z} = args;"));
    }
}
