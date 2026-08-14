// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

//! Static resolution of remote LambdaMOO verb locations.

use std::collections::HashSet;

use lsp_types::{Location, Position, Range, Uri};
use tree_sitter::Node;

use crate::{
    line_index::{LineIndex, safe_slice},
    locals,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Constant {
    Object(ObjectPath),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObjectPath {
    number: u64,
    properties: Vec<String>,
}

struct Resolver<'a, 'tree> {
    root: Node<'tree>,
    text: &'a str,
    source_uri: &'a Uri,
    this_object: u64,
}

/// Resolve a definition request on a verb name to a `moo:` document location.
pub fn find_definition(
    root: Node,
    text: &str,
    position: Position,
    source_uri: &Uri,
) -> Option<Location> {
    let (authority, this_object) = source_context(source_uri)?;
    let offset = LineIndex::new(text).offset(text, position);
    let mut node = root.descendant_for_byte_range(offset, offset)?;
    if offset > 0 && !position_is_in(node, offset) {
        node = root.descendant_for_byte_range(offset - 1, offset - 1)?;
    }

    let call = ancestors(node)
        .find(|candidate| matches!(candidate.kind(), "verb_call" | "system_verb_call"))?;
    let verb = call.child_by_field_name("verb")?;
    if !position_is_in(verb, offset) && !(offset > 0 && position_is_in(verb, offset - 1)) {
        return None;
    }

    let resolver = Resolver {
        root,
        text,
        source_uri,
        this_object,
    };
    let verb_name = if verb.kind() == "identifier" || verb.kind() == "invalid_identifier" {
        safe_slice(text, verb.byte_range()).to_owned()
    } else {
        match resolver.resolve(verb, &mut HashSet::new())? {
            Constant::String(value) => value,
            Constant::Object(_) => return None,
        }
    };
    let receiver = if call.kind() == "system_verb_call" {
        ObjectPath {
            number: 0,
            properties: Vec::new(),
        }
    } else {
        match resolver.resolve(call.child_by_field_name("receiver")?, &mut HashSet::new())? {
            Constant::Object(value) => value,
            Constant::String(_) => return None,
        }
    };
    let uri = build_uri(authority, &receiver, &verb_name).parse().ok()?;
    Some(Location {
        uri,
        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
    })
}

impl Resolver<'_, '_> {
    fn resolve(&self, node: Node, active: &mut HashSet<(usize, usize)>) -> Option<Constant> {
        let node = unwrap(node);
        match node.kind() {
            "object" => {
                let raw = safe_slice(self.text, node.byte_range()).strip_prefix('#')?;
                Some(Constant::Object(ObjectPath {
                    number: raw.parse().ok()?,
                    properties: Vec::new(),
                }))
            }
            "string" => Some(Constant::String(parse_string(safe_slice(
                self.text,
                node.byte_range(),
            ))?)),
            "identifier" => {
                let name = safe_slice(self.text, node.byte_range());
                if name.eq_ignore_ascii_case("this") {
                    return Some(Constant::Object(ObjectPath {
                        number: self.this_object,
                        properties: Vec::new(),
                    }));
                }
                self.resolve_identifier(node, active)
            }
            "prop_access" => self.resolve_property(node, active),
            _ => None,
        }
    }

    fn resolve_identifier(
        &self,
        node: Node,
        active: &mut HashSet<(usize, usize)>,
    ) -> Option<Constant> {
        let key = (node.start_byte(), node.end_byte());
        if !active.insert(key) {
            return None;
        }
        let index = LineIndex::new(self.text);
        let position = index.clamp_point(
            self.text,
            node.start_position().row,
            node.start_position().column,
        );
        let range = index.clamp_range(
            self.text,
            node.start_position().row,
            node.start_position().column,
            node.end_position().row,
            node.end_position().column,
        );
        if locals::analyze_locals(self.root, self.text)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range == range)
        {
            active.remove(&key);
            return None;
        }
        let definitions = locals::find_definitions(self.root, self.text, position, self.source_uri);
        let mut value: Option<Constant> = None;
        for definition in definitions {
            let rhs = assignment_rhs_at(self.root, self.text, definition.range)?;
            let candidate = self.resolve(rhs, active)?;
            if value
                .as_ref()
                .is_some_and(|existing| existing != &candidate)
            {
                active.remove(&key);
                return None;
            }
            value = Some(candidate);
        }
        active.remove(&key);
        value
    }

    fn resolve_property(
        &self,
        node: Node,
        active: &mut HashSet<(usize, usize)>,
    ) -> Option<Constant> {
        let mut named = named_children(node);
        if named.len() == 1 && safe_slice(self.text, node.byte_range()).starts_with('$') {
            let properties = named
                .into_iter()
                .map(|property| property_name(self, property, active))
                .collect::<Option<Vec<_>>>()?;
            return Some(Constant::Object(ObjectPath {
                number: 0,
                properties,
            }));
        }
        let property = named.pop()?;
        let receiver = named.pop()?;
        let Constant::Object(mut receiver) = self.resolve(receiver, active)? else {
            return None;
        };
        receiver
            .properties
            .push(property_name(self, property, active)?);
        Some(Constant::Object(receiver))
    }
}

fn property_name(
    resolver: &Resolver<'_, '_>,
    node: Node,
    active: &mut HashSet<(usize, usize)>,
) -> Option<String> {
    if matches!(node.kind(), "identifier" | "invalid_identifier") {
        Some(safe_slice(resolver.text, node.byte_range()).to_owned())
    } else {
        match resolver.resolve(node, active)? {
            Constant::String(value) => Some(value),
            Constant::Object(_) => None,
        }
    }
}

fn assignment_rhs_at<'tree>(root: Node<'tree>, text: &str, range: Range) -> Option<Node<'tree>> {
    let index = LineIndex::new(text);
    let offset = index.offset(text, range.start);
    let identifier = root.descendant_for_byte_range(offset, offset)?;
    ancestors(identifier)
        .find(|node| node.kind() == "assignment")?
        .named_child(1)
}

fn source_context(uri: &Uri) -> Option<(&str, u64)> {
    let raw = uri.as_str().strip_prefix("moo://")?;
    let (authority, path) = raw.split_once('/')?;
    if authority.is_empty() {
        return None;
    }
    let mut segments = path.split('/');
    if segments.next()? != "object" {
        return None;
    }
    let number = segments.next()?.parse().ok()?;
    Some((authority, number))
}

fn build_uri(authority: &str, object: &ObjectPath, verb: &str) -> String {
    let mut result = format!("moo://{authority}/object/{}", object.number);
    for property in &object.properties {
        result.push_str("/property/");
        result.push_str(&encode_segment(property));
        result.push_str("/object");
    }
    result.push_str("/verb/");
    result.push_str(&encode_segment(verb));
    result
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn parse_string(raw: &str) -> Option<String> {
    let body = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut result = String::new();
    let mut chars = body.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            result.push(chars.next()?);
        } else {
            result.push(character);
        }
    }
    Some(result)
}

fn unwrap(mut node: Node) -> Node {
    while matches!(node.kind(), "expression" | "arg_item") && node.named_child_count() == 1 {
        node = node.named_child(0).unwrap_or(node);
    }
    node
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(Some(node), |node| node.parent())
}

fn position_is_in(node: Node, offset: usize) -> bool {
    node.start_byte() <= offset && offset <= node.end_byte()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn definition(text: &str, character: u32) -> Option<Location> {
        let tree = parser::parse(text)?;
        let uri = "moo://codepoint/object/42/verb/current".parse().unwrap();
        find_definition(tree.root_node(), text, Position::new(0, character), &uri)
    }

    #[test]
    fn resolves_direct_contextual_and_property_targets() {
        assert_eq!(
            definition("#123:Foo();", 6).unwrap().uri.as_str(),
            "moo://codepoint/object/123/verb/Foo"
        );
        assert_eq!(
            definition("this:look();", 7).unwrap().uri.as_str(),
            "moo://codepoint/object/42/verb/look"
        );
        assert_eq!(
            definition("$local.webdav:foo();", 15).unwrap().uri.as_str(),
            "moo://codepoint/object/0/property/local/object/property/webdav/object/verb/foo"
        );
    }

    #[test]
    fn resolves_reaching_constants_and_encodes_names() {
        let text = "target = #123; name = \"say hi\"; target:(name)();";
        assert_eq!(
            definition(text, 45).unwrap().uri.as_str(),
            "moo://codepoint/object/123/verb/say%20hi"
        );
    }

    #[test]
    fn rejects_ambiguous_constants_and_non_moo_sources() {
        let text = "if (player) target = #1; else target = #2; endif target:foo();";
        assert!(definition(text, 61).is_none());
        let text = "if (player) target = #1; endif target:foo();";
        assert!(definition(text, 40).is_none());
        let tree = parser::parse("#1:foo();").unwrap();
        let uri = "file:///test.moo".parse().unwrap();
        assert!(
            find_definition(tree.root_node(), "#1:foo();", Position::new(0, 4), &uri).is_none()
        );
    }
}
