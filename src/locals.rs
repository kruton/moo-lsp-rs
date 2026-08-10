// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use crate::line_index::{LineIndex, safe_slice};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DocumentHighlight, DocumentHighlightKind, Location,
    NumberOrString, Position, Range, Uri,
};
use std::{collections::HashMap, sync::LazyLock};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

static LOCALS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    let language = tree_sitter_lambdamoo::LANGUAGE.into();
    let query = format!(
        "{}\n(scattering_assignment (scatter_list) @local.scatter)",
        tree_sitter_lambdamoo::LOCALS_QUERY
    );
    Query::new(&language, &query).expect("Failed to compile locals Tree-sitter query")
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolLocation {
    pub name: String,
    pub is_definition: bool,
    pub range: Range,
}

const PREDEFINED_LOCALS: &[&str] = &[
    "int", "float", "obj", "str", "list", "err", "num", "player", "this", "caller", "verb", "args",
    "argstr", "dobj", "dobjstr", "prepstr", "iobj", "iobjstr",
];

#[derive(Clone, Default, PartialEq, Eq)]
struct Binding {
    definite: bool,
    definitions: Vec<Range>,
    predefined: bool,
}

type State = HashMap<String, Binding>;

#[derive(Default)]
pub struct LocalAnalysis {
    pub diagnostics: Vec<Diagnostic>,
    definitions: Vec<(Range, Vec<Range>)>,
}

struct Analyzer<'a> {
    text: &'a str,
    line_index: LineIndex,
    result: LocalAnalysis,
    recording: bool,
}

pub fn analyze_locals(root: Node, text: &str) -> LocalAnalysis {
    let mut state = State::new();
    for name in PREDEFINED_LOCALS {
        state.insert(
            (*name).to_owned(),
            Binding {
                definite: true,
                definitions: Vec::new(),
                predefined: true,
            },
        );
    }
    let mut analyzer = Analyzer {
        text,
        line_index: LineIndex::new(text),
        result: LocalAnalysis::default(),
        recording: true,
    };
    analyzer.analyze_block(root, state);
    analyzer.result
}

impl Analyzer<'_> {
    fn range(&self, node: Node) -> Range {
        self.line_index.clamp_range(
            self.text,
            node.start_position().row,
            node.start_position().column,
            node.end_position().row,
            node.end_position().column,
        )
    }

    fn name(&self, node: Node) -> String {
        safe_slice(self.text, node.byte_range()).to_ascii_lowercase()
    }

    fn record_reference(&mut self, node: Node, state: &State) {
        if !self.recording || node.kind() != "identifier" {
            return;
        }
        let name = self.name(node);
        let binding = state.get(&name);
        let range = self.range(node);
        let definitions = binding
            .filter(|binding| !binding.predefined)
            .map(|binding| binding.definitions.clone())
            .unwrap_or_default();
        if let Some((_, existing)) = self
            .result
            .definitions
            .iter_mut()
            .find(|(existing, _)| *existing == range)
        {
            for definition in definitions {
                if !existing.contains(&definition) {
                    existing.push(definition);
                }
            }
            existing.sort_by_key(|range| (range.start.line, range.start.character));
        } else {
            self.result.definitions.push((range, definitions));
        }
        if !binding.is_some_and(|binding| binding.definite) {
            let message = format!(
                "Local variable '{}' may be unbound",
                safe_slice(self.text, node.byte_range())
            );
            if !self
                .result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.range == range && diagnostic.message == message)
            {
                self.result.diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("unbound-local".to_owned())),
                    code_description: None,
                    source: Some("moo-lsp-rs".to_owned()),
                    message,
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }
    }

    fn bind(&mut self, node: Node, state: &mut State) {
        if node.kind() != "identifier" {
            return;
        }
        let name = self.name(node);
        let range = self.range(node);
        let predefined = state.get(&name).is_some_and(|binding| binding.predefined);
        state.insert(
            name,
            Binding {
                definite: true,
                definitions: if predefined { Vec::new() } else { vec![range] },
                predefined,
            },
        );
        if self.recording {
            self.result
                .definitions
                .push((range, if predefined { Vec::new() } else { vec![range] }));
        }
    }

    fn maybe_bind(&mut self, node: Node, state: &mut State) {
        let name = self.name(node);
        let range = self.range(node);
        let binding = state.entry(name).or_default();
        if !binding.predefined && !binding.definitions.contains(&range) {
            binding.definitions.push(range);
        }
        if self.recording {
            self.result.definitions.push((
                range,
                if binding.predefined {
                    Vec::new()
                } else {
                    vec![range]
                },
            ));
        }
    }

    fn analyze_block(&mut self, node: Node, mut state: State) -> Option<State> {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "statement" {
                continue;
            }
            state = self.analyze_statement(child, state)?;
        }
        Some(state)
    }

    fn analyze_statement(&mut self, node: Node, state: State) -> Option<State> {
        if node.is_missing() || node.kind() == "ERROR" || node.has_error() {
            return Some(state);
        }
        let inner = if node.kind() == "statement" {
            node.named_child(0).unwrap_or(node)
        } else {
            node
        };
        match inner.kind() {
            "if_statement" => self.analyze_if(inner, state),
            "for_statement" => Some(self.analyze_for(inner, state)),
            "while_statement" => Some(self.analyze_while(inner, state)),
            "fork_statement" => Some(self.analyze_fork(inner, state)),
            "try_statement" => self.analyze_try(inner, state),
            "return_statement" => {
                let state = self.analyze_named_expressions(inner, state);
                let _ = state;
                None
            }
            "break_statement" | "continue_statement" => None,
            _ => Some(self.analyze_expression(inner, state)),
        }
    }

    fn analyze_expression(&mut self, node: Node, mut state: State) -> State {
        if node.is_missing() || node.kind() == "ERROR" || node.has_error() {
            return state;
        }
        match node.kind() {
            "identifier" => self.record_reference(node, &state),
            "assignment" => {
                if let (Some(lhs), Some(rhs)) = (node.named_child(0), node.named_child(1)) {
                    state = self.analyze_expression(rhs, state);
                    if lhs.kind() == "identifier" {
                        self.bind(lhs, &mut state);
                    } else {
                        state = self.analyze_expression(lhs, state);
                    }
                }
            }
            "scattering_assignment" => state = self.analyze_scatter(node, state),
            "call_expression" | "system_verb_call" => {
                if let Some(args) = node.child_by_field_name("arguments") {
                    state = self.analyze_expression(args, state);
                }
            }
            "verb_call" => {
                if let Some(receiver) = node.child_by_field_name("receiver") {
                    state = self.analyze_expression(receiver, state);
                }
                if let Some(verb) = node.child_by_field_name("verb")
                    && verb.kind() == "expression"
                {
                    state = self.analyze_expression(verb, state);
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    state = self.analyze_expression(args, state);
                }
            }
            "prop_access" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() != "identifier" {
                        state = self.analyze_expression(child, state);
                    }
                }
            }
            "binary_expression" => {
                let children = named_children(node);
                if let Some(first) = children.first() {
                    state = self.analyze_expression(*first, state);
                }
                if let Some(second) = children.get(1) {
                    let rhs = self.analyze_expression(*second, state.clone());
                    let short_circuit = node
                        .children(&mut node.walk())
                        .any(|child| matches!(child.kind(), "&&" | "||"));
                    state = if short_circuit {
                        join_states(&[state, rhs])
                    } else {
                        rhs
                    };
                }
            }
            "ternary_expression" => {
                let children = named_children(node);
                if let Some(condition) = children.first() {
                    state = self.analyze_expression(*condition, state);
                }
                if children.len() >= 3 {
                    let yes = self.analyze_expression(children[1], state.clone());
                    let no = self.analyze_expression(children[2], state.clone());
                    state = join_states(&[yes, no]);
                }
            }
            "catch_expression" => {
                let children = named_children(node);
                if let Some(protected) = children.first() {
                    let success = self.analyze_expression(*protected, state.clone());
                    let mut failure = state.clone();
                    if let Some(codes) = children.iter().find(|child| child.kind() == "codes") {
                        failure = self.analyze_expression(*codes, failure);
                    }
                    if let Some(fallback) = children
                        .iter()
                        .rev()
                        .find(|child| child.kind() == "expression" && **child != *protected)
                    {
                        failure = self.analyze_expression(*fallback, failure);
                        state = join_states(&[success, failure]);
                    } else {
                        state = success;
                    }
                }
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    state = self.analyze_expression(child, state);
                }
            }
        }
        state
    }

    fn analyze_named_expressions(&mut self, node: Node, mut state: State) -> State {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "expression" {
                state = self.analyze_expression(child, state);
            }
        }
        state
    }

    fn analyze_scatter(&mut self, node: Node, mut state: State) -> State {
        let Some(list) = node.named_child(0) else {
            return state;
        };
        let Some(rhs) = node.named_child(1) else {
            return state;
        };
        state = self.analyze_expression(rhs, state);
        let mut deferred = Vec::new();
        let mut cursor = list.walk();
        let children: Vec<_> = list.children(&mut cursor).collect();
        for item in children.split(|child| child.kind() == ",") {
            let Some(target) = item
                .iter()
                .copied()
                .find(|child| child.kind() == "identifier")
            else {
                continue;
            };
            let raw = item
                .first()
                .zip(item.last())
                .map(|(first, last)| safe_slice(self.text, first.start_byte()..last.end_byte()))
                .unwrap_or_default()
                .trim_start();
            let default = item
                .iter()
                .copied()
                .find(|child| child.kind() == "expression");
            if raw.starts_with('?') && default.is_none() {
                self.maybe_bind(target, &mut state);
            } else if let Some(default) = default {
                deferred.push((target, default));
            } else {
                self.bind(target, &mut state);
            }
        }
        for (target, default) in deferred {
            state = self.analyze_expression(default, state);
            self.bind(target, &mut state);
        }
        state
    }

    fn analyze_if(&mut self, node: Node, mut state: State) -> Option<State> {
        let children = named_children(node);
        let Some(condition) = children.iter().find(|child| child.kind() == "expression") else {
            return Some(state);
        };
        state = self.analyze_expression(*condition, state);
        let mut branches = Vec::new();
        let direct = children
            .iter()
            .copied()
            .filter(|child| child.kind() == "statement");
        branches.push(self.analyze_statement_list(direct, state.clone()));
        let mut has_else = false;
        let mut false_state = state.clone();
        for clause in children {
            match clause.kind() {
                "elseif_clause" => {
                    let clause_children = named_children(clause);
                    if let Some(condition) = clause_children
                        .iter()
                        .find(|child| child.kind() == "expression")
                    {
                        false_state = self.analyze_expression(*condition, false_state);
                    }
                    branches.push(
                        self.analyze_statement_list(
                            clause_children
                                .into_iter()
                                .filter(|child| child.kind() == "statement"),
                            false_state.clone(),
                        ),
                    );
                }
                "else_clause" => {
                    has_else = true;
                    branches.push(
                        self.analyze_statement_list(
                            named_children(clause)
                                .into_iter()
                                .filter(|child| child.kind() == "statement"),
                            false_state.clone(),
                        ),
                    );
                }
                _ => {}
            }
        }
        if !has_else {
            branches.push(Some(false_state));
        }
        join_reachable(branches)
    }

    fn analyze_statement_list<'tree>(
        &mut self,
        statements: impl IntoIterator<Item = Node<'tree>>,
        mut state: State,
    ) -> Option<State> {
        for statement in statements {
            state = self.analyze_statement(statement, state)?;
        }
        Some(state)
    }

    fn analyze_for(&mut self, node: Node, mut state: State) -> State {
        let children = named_children(node);
        for expression in children.iter().filter(|child| child.kind() == "expression") {
            state = self.analyze_expression(*expression, state);
        }
        let mut body_state = state.clone();
        if let Some(variable) = children.iter().find(|child| child.kind() == "identifier") {
            self.bind(*variable, &mut body_state);
        }
        let statements: Vec<_> = children
            .into_iter()
            .filter(|child| child.kind() == "statement")
            .collect();
        let recording = self.recording;
        self.recording = false;
        let mut head = body_state.clone();
        loop {
            let body = self
                .analyze_statement_list(statements.iter().copied(), head.clone())
                .unwrap_or_else(|| head.clone());
            let next = join_states(&[body_state.clone(), body]);
            if next == head {
                break;
            }
            head = next;
        }
        self.recording = recording;
        let body = self
            .analyze_statement_list(statements, head.clone())
            .unwrap_or(head);
        join_states(&[state, body])
    }

    fn analyze_while(&mut self, node: Node, mut state: State) -> State {
        let children = named_children(node);
        let condition = children
            .iter()
            .copied()
            .find(|child| child.kind() == "expression");
        let statements: Vec<_> = children
            .into_iter()
            .filter(|child| child.kind() == "statement")
            .collect();
        let entry = state.clone();
        let recording = self.recording;
        self.recording = false;
        let mut head = entry.clone();
        loop {
            let condition_state = condition
                .map(|condition| self.analyze_expression(condition, head.clone()))
                .unwrap_or_else(|| head.clone());
            let body = self
                .analyze_statement_list(statements.iter().copied(), condition_state)
                .unwrap_or_else(|| head.clone());
            let next = join_states(&[entry.clone(), body]);
            if next == head {
                break;
            }
            head = next;
        }
        self.recording = recording;
        state = if let Some(condition) = condition {
            self.analyze_expression(condition, head)
        } else {
            head
        };
        let _ = self.analyze_statement_list(statements, state.clone());
        state
    }

    fn analyze_fork(&mut self, node: Node, mut state: State) -> State {
        let children = named_children(node);
        if let Some(delay) = children.iter().find(|child| child.kind() == "expression") {
            state = self.analyze_expression(*delay, state);
        }
        if let Some(variable) = children.iter().find(|child| child.kind() == "identifier") {
            self.bind(*variable, &mut state);
        }
        let _ = self.analyze_statement_list(
            children
                .into_iter()
                .filter(|child| child.kind() == "statement"),
            state.clone(),
        );
        state
    }

    fn analyze_try(&mut self, node: Node, state: State) -> Option<State> {
        let children = named_children(node);
        let direct: Vec<_> = children
            .iter()
            .copied()
            .filter(|child| child.kind() == "statement")
            .collect();
        let clauses: Vec<_> = children
            .iter()
            .copied()
            .filter(|child| child.kind() == "except_clause")
            .collect();
        if !clauses.is_empty() {
            let normal = self.analyze_statement_list(direct, state.clone());
            let mut paths = vec![normal];
            for clause in clauses {
                let clause_children = named_children(clause);
                let mut handler = state.clone();
                if let Some(variable) = clause_children
                    .iter()
                    .find(|child| child.kind() == "identifier")
                {
                    self.bind(*variable, &mut handler);
                }
                for codes in clause_children
                    .iter()
                    .filter(|child| child.kind() == "codes")
                {
                    handler = self.analyze_expression(*codes, handler);
                }
                paths.push(
                    self.analyze_statement_list(
                        clause_children
                            .into_iter()
                            .filter(|child| child.kind() == "statement"),
                        handler,
                    ),
                );
            }
            return join_reachable(paths);
        }

        let mut before_finally = Vec::new();
        let mut after_finally = Vec::new();
        let mut seen_finally = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "finally" {
                seen_finally = true;
            } else if child.kind() == "statement" {
                if seen_finally {
                    after_finally.push(child)
                } else {
                    before_finally.push(child)
                }
            }
        }
        let _ = self.analyze_statement_list(after_finally.iter().copied(), state.clone());
        let normal = self.analyze_statement_list(before_finally, state.clone())?;
        let recording = self.recording;
        self.recording = false;
        let output = self.analyze_statement_list(after_finally, normal);
        self.recording = recording;
        output
    }
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn join_reachable(states: Vec<Option<State>>) -> Option<State> {
    let states: Vec<_> = states.into_iter().flatten().collect();
    (!states.is_empty()).then(|| join_states(&states))
}

fn join_states(states: &[State]) -> State {
    let mut result = State::new();
    for state in states {
        for (name, binding) in state {
            let joined = result.entry(name.clone()).or_insert_with(|| Binding {
                definite: true,
                definitions: Vec::new(),
                predefined: binding.predefined,
            });
            joined.predefined |= binding.predefined;
            for definition in &binding.definitions {
                if !joined.definitions.contains(definition) {
                    joined.definitions.push(*definition);
                }
            }
        }
    }
    for (name, binding) in &mut result {
        binding.definite = states
            .iter()
            .all(|state| state.get(name).is_some_and(|candidate| candidate.definite));
        binding
            .definitions
            .sort_by_key(|range| (range.start.line, range.start.character));
    }
    result
}

pub fn collect_locals(root: Node, text: &str) -> Vec<SymbolLocation> {
    let query = &*LOCALS_QUERY;
    let line_index = LineIndex::new(text);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, text.as_bytes());
    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let cap_name = match query.capture_names().get(cap.index as usize) {
                Some(name) => *name,
                None => continue,
            };
            let cap_node = cap.node;

            if cap_name == "local.scatter" {
                collect_scatter_definitions(cap_node, text, &line_index, &mut symbols);
                continue;
            }

            let name = safe_slice(text, cap_node.byte_range()).trim().to_string();

            if name.is_empty() {
                continue;
            }

            let start_pos = cap_node.start_position();
            let end_pos = cap_node.end_position();
            let range = line_index.clamp_range(
                text,
                start_pos.row,
                start_pos.column,
                end_pos.row,
                end_pos.column,
            );

            let is_definition = match cap_name {
                "local.definition" => true,
                "local.reference" => false,
                _ => continue,
            };

            if !is_definition
                && symbols
                    .iter()
                    .any(|s: &SymbolLocation| s.range == range && s.is_definition)
            {
                continue;
            }

            // Avoid duplicate range entries
            if !symbols
                .iter()
                .any(|s: &SymbolLocation| s.range == range && s.is_definition == is_definition)
            {
                symbols.push(SymbolLocation {
                    name,
                    is_definition,
                    range,
                });
            }
        }
    }

    symbols
}

fn collect_scatter_definitions(
    scatter_list: Node,
    text: &str,
    line_index: &LineIndex,
    symbols: &mut Vec<SymbolLocation>,
) {
    let mut cursor = scatter_list.walk();
    let mut needs_target = true;
    for child in scatter_list.children(&mut cursor) {
        if child.kind() == "," {
            needs_target = true;
        } else if needs_target && child.kind() == "identifier" {
            let range = line_index.clamp_range(
                text,
                child.start_position().row,
                child.start_position().column,
                child.end_position().row,
                child.end_position().column,
            );
            let name = safe_slice(text, child.byte_range()).to_string();
            if !symbols.iter().any(|s| s.range == range && s.is_definition) {
                symbols.push(SymbolLocation {
                    name,
                    is_definition: true,
                    range,
                });
            }
            needs_target = false;
        }
    }
}

pub fn find_definitions(root: Node, text: &str, position: Position, uri: &Uri) -> Vec<Location> {
    let analysis = analyze_locals(root, text);
    let ranges = analysis
        .definitions
        .iter()
        .find(|(range, _)| position_in_range(position, *range))
        .or_else(|| {
            (position.character > 0)
                .then(|| Position {
                    line: position.line,
                    character: position.character - 1,
                })
                .and_then(|previous| {
                    analysis
                        .definitions
                        .iter()
                        .find(|(range, _)| position_in_range(previous, *range))
                })
        })
        .map(|(_, definitions)| definitions)
        .cloned()
        .unwrap_or_default();
    ranges
        .into_iter()
        .map(|range| Location {
            uri: uri.clone(),
            range,
        })
        .collect()
}

pub fn find_definition(root: Node, text: &str, position: Position, uri: &Uri) -> Option<Location> {
    find_definitions(root, text, position, uri)
        .into_iter()
        .next()
}

pub fn find_highlights(root: Node, text: &str, position: Position) -> Vec<DocumentHighlight> {
    let delimiter_highlights = find_delimiter_or_keyword_highlights(root, text, position);
    if !delimiter_highlights.is_empty() {
        return delimiter_highlights;
    }

    let locals = collect_locals(root, text);
    let Some(target_symbol) = locals.iter().find(|s| position_in_range(position, s.range)) else {
        return Vec::new();
    };
    let target_name = target_symbol.name.clone();

    locals
        .into_iter()
        .filter(|s| s.name == target_name)
        .map(|s| DocumentHighlight {
            range: s.range,
            kind: if s.is_definition {
                Some(DocumentHighlightKind::WRITE)
            } else {
                Some(DocumentHighlightKind::READ)
            },
        })
        .collect()
}

fn find_delimiter_or_keyword_highlights(
    root: Node,
    text: &str,
    position: Position,
) -> Vec<DocumentHighlight> {
    let point = tree_sitter::Point {
        row: position.line as usize,
        column: position.character as usize,
    };

    let node_at = root.descendant_for_point_range(point, point);
    let mut candidate_nodes_expanded = Vec::new();
    if let Some(n) = node_at {
        candidate_nodes_expanded.push(n);
        if let Some(p) = n.parent() {
            candidate_nodes_expanded.push(p);
        }
    }
    if point.column > 0 {
        let prev_point = tree_sitter::Point {
            row: point.row,
            column: point.column - 1,
        };
        if let Some(prev_node) = root.descendant_for_point_range(prev_point, prev_point) {
            candidate_nodes_expanded.push(prev_node);
            if let Some(p) = prev_node.parent() {
                candidate_nodes_expanded.push(p);
            }
        }
    }

    for node in candidate_nodes_expanded {
        let kind = node.kind();
        let node_slice = safe_slice(text, node.byte_range()).trim();

        if (kind == "(" || kind == ")")
            && let Some(parent) = node.parent()
        {
            let highlights = find_matching_token_children(parent, "(", ")");
            if !highlights.is_empty() {
                return highlights;
            }
        }

        if (kind == "[" || kind == "]")
            && let Some(parent) = node.parent()
        {
            let highlights = find_matching_token_children(parent, "[", "]");
            if !highlights.is_empty() {
                return highlights;
            }
        }

        if (kind == "{" || kind == "}")
            && let Some(parent) = node.parent()
        {
            let highlights = find_matching_token_children(parent, "{", "}");
            if !highlights.is_empty() {
                return highlights;
            }
        }

        match node_slice {
            "if" | "elseif" | "else" | "endif" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "if_statement") {
                    let highlights =
                        find_matching_keywords(stmt_node, text, &["if", "elseif", "else", "endif"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "for" | "endfor" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "for_statement") {
                    let highlights = find_matching_keywords(stmt_node, text, &["for", "endfor"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "while" | "endwhile" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "while_statement") {
                    let highlights =
                        find_matching_keywords(stmt_node, text, &["while", "endwhile"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "fork" | "endfork" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "fork_statement") {
                    let highlights = find_matching_keywords(stmt_node, text, &["fork", "endfork"]);
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            "try" | "except" | "finally" | "endtry" => {
                if let Some(stmt_node) = find_ancestor_of_kind(node, "try_statement") {
                    let highlights = find_matching_keywords(
                        stmt_node,
                        text,
                        &["try", "except", "finally", "endtry"],
                    );
                    if !highlights.is_empty() {
                        return highlights;
                    }
                }
            }
            _ => {}
        }
    }

    Vec::new()
}

fn find_matching_token_children(
    parent: Node,
    open_kind: &str,
    close_kind: &str,
) -> Vec<DocumentHighlight> {
    let mut open_node = None;
    let mut close_node = None;
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        if child.kind() == open_kind && open_node.is_none() {
            open_node = Some(child);
        } else if child.kind() == close_kind {
            close_node = Some(child);
        }
    }

    let mut result = Vec::new();
    if let Some(o) = open_node {
        let start = o.start_position();
        let end = o.end_position();
        result.push(DocumentHighlight {
            range: Range {
                start: Position {
                    line: start.row as u32,
                    character: start.column as u32,
                },
                end: Position {
                    line: end.row as u32,
                    character: end.column as u32,
                },
            },
            kind: Some(DocumentHighlightKind::TEXT),
        });
    }
    if let Some(c) = close_node {
        let start = c.start_position();
        let end = c.end_position();
        result.push(DocumentHighlight {
            range: Range {
                start: Position {
                    line: start.row as u32,
                    character: start.column as u32,
                },
                end: Position {
                    line: end.row as u32,
                    character: end.column as u32,
                },
            },
            kind: Some(DocumentHighlightKind::TEXT),
        });
    }
    if result.len() == 2 {
        result
    } else {
        Vec::new()
    }
}

fn find_matching_keywords(
    stmt_node: Node,
    text: &str,
    keywords: &[&str],
) -> Vec<DocumentHighlight> {
    let mut result = Vec::new();
    let mut cursor = stmt_node.walk();
    for child in stmt_node.children(&mut cursor) {
        let child_slice = safe_slice(text, child.byte_range())
            .trim()
            .trim_end_matches(';')
            .trim();
        if keywords.contains(&child_slice) {
            let start = child.start_position();
            let end = Position {
                line: start.row as u32,
                character: (start.column + child_slice.len()) as u32,
            };
            result.push(DocumentHighlight {
                range: Range {
                    start: Position {
                        line: start.row as u32,
                        character: start.column as u32,
                    },
                    end,
                },
                kind: Some(DocumentHighlightKind::TEXT),
            });
        }
    }
    result
}

fn find_ancestor_of_kind<'a>(mut node: Node<'a>, target_kind: &str) -> Option<Node<'a>> {
    while let Some(parent) = node.parent() {
        if parent.kind() == target_kind {
            return Some(parent);
        }
        node = parent;
    }
    None
}

fn position_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character >= range.end.character {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::str::FromStr;

    #[test]
    fn test_collect_locals() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let locals = collect_locals(tree.root_node(), code);
        assert!(!locals.is_empty());
        let x_defs: Vec<_> = locals
            .iter()
            .filter(|s| s.name == "x" && s.is_definition)
            .collect();
        assert_eq!(x_defs.len(), 1);
    }

    fn unbound_names(code: &str) -> Vec<String> {
        let tree = parser::parse(code).unwrap();
        analyze_locals(tree.root_node(), code)
            .diagnostics
            .into_iter()
            .filter_map(|diagnostic| {
                diagnostic
                    .message
                    .strip_prefix("Local variable '")
                    .and_then(|message| message.strip_suffix("' may be unbound"))
                    .map(str::to_owned)
            })
            .collect()
    }

    #[test]
    fn reports_unbound_reads_and_respects_evaluation_order() {
        assert_eq!(unbound_names("return missing;\n"), ["missing"]);
        assert!(unbound_names("value = 1; return value;\n").is_empty());
        assert_eq!(unbound_names("value = value + 1;\n"), ["value"]);
        assert!(unbound_names("return PLAYER == int && Args == {};\n").is_empty());
    }

    #[test]
    fn intersects_definite_bindings_across_branches() {
        assert_eq!(
            unbound_names("if (player)\n  value = 1;\nendif\nreturn value;\n"),
            ["value"]
        );
        assert!(
            unbound_names("if (player)\n  value = 1;\nelse\n  value = 2;\nendif\nreturn value;\n")
                .is_empty()
        );
    }

    #[test]
    fn handles_scatter_and_construct_bindings() {
        assert_eq!(
            unbound_names(
                "{required, ?optional, ?defaulted = 1, @rest} = args; return {required, optional, defaulted, rest};\n"
            ),
            ["optional"]
        );
        assert!(unbound_names("for item in (args)\n  notify(player, item);\nendfor\n").is_empty());
        assert!(
            unbound_names("fork task (0)\n  notify(player, task);\nendfork\nreturn task;\n")
                .is_empty()
        );
        assert!(
            unbound_names("try\n  return missing;\nexcept error (ANY)\n  return error;\nendtry\n")
                .contains(&"missing".to_owned())
        );
    }

    #[test]
    fn analyzes_catch_and_finally_paths() {
        assert_eq!(
            unbound_names("result = `(left = 1) ! ANY => (right = 2)'; return {left, right};\n"),
            ["left", "right"]
        );
        assert_eq!(
            unbound_names("try\n  return;\nfinally\n  notify(player, missing);\nendtry\n"),
            ["missing"]
        );
        assert!(
            unbound_names(
                "try\n  value = 1;\nfinally\n  notify(player, \"done\");\nendtry\nreturn value;\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn returns_all_reaching_branch_definitions() {
        let code = "if (player)\n  value = 1;\nelse\n  value = 2;\nendif\nreturn value;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();
        let definitions = find_definitions(tree.root_node(), code, Position::new(5, 7), &uri);
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].range.start, Position::new(1, 2));
        assert_eq!(definitions[1].range.start, Position::new(3, 2));
    }

    #[test]
    fn loop_reads_include_loop_carried_definitions() {
        let code = "x = 5;\nmessage = 1;\nwhile (x > 0)\n  x = x - 1;\n  message = message + 1;\nendwhile\nreturn message;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        let x_definitions = find_definitions(tree.root_node(), code, Position::new(3, 6), &uri);
        assert_eq!(x_definitions.len(), 2);
        assert_eq!(x_definitions[0].range.start, Position::new(0, 0));
        assert_eq!(x_definitions[1].range.start, Position::new(3, 2));

        let message_definitions =
            find_definitions(tree.root_node(), code, Position::new(4, 12), &uri);
        assert_eq!(message_definitions.len(), 2);
        assert_eq!(message_definitions[0].range.start, Position::new(1, 0));
        assert_eq!(message_definitions[1].range.start, Position::new(4, 2));
        assert!(
            analyze_locals(tree.root_node(), code)
                .diagnostics
                .is_empty()
        );
    }

    #[test]
    fn test_find_definition() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();
        // Position on 'x' in line 1: `y = x + 2;`
        let pos = Position {
            line: 1,
            character: 4,
        };
        let def_loc = find_definition(tree.root_node(), code, pos, &uri);
        assert!(def_loc.is_some());
        assert_eq!(def_loc.unwrap().range.start.line, 0);
    }

    #[test]
    fn test_find_definition_on_rhs_of_reassignment_uses_previous_assignment() {
        let code = "target = 1;\ntarget = target + 1;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        let def_loc = find_definition(tree.root_node(), code, Position::new(1, 9), &uri).unwrap();

        assert_eq!(
            def_loc.range,
            Range::new(Position::new(0, 0), Position::new(0, 6))
        );
    }

    #[test]
    fn test_find_definition_from_scatter_assignment() {
        let code = "{x} = args;\ny = x;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        let def_loc = find_definition(tree.root_node(), code, Position::new(1, 4), &uri).unwrap();

        assert_eq!(
            def_loc.range,
            Range::new(Position::new(0, 1), Position::new(0, 2))
        );
    }

    #[test]
    fn test_position_in_range_end_is_exclusive() {
        let range = Range {
            start: Position {
                line: 1,
                character: 4,
            },
            end: Position {
                line: 1,
                character: 5,
            },
        };

        assert!(position_in_range(
            Position {
                line: 1,
                character: 4,
            },
            range
        ));
        assert!(!position_in_range(
            Position {
                line: 1,
                character: 5,
            },
            range
        ));
    }

    #[test]
    fn test_find_definition_when_cursor_is_after_symbol() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        // Position one character after `x` in `y = x + 2;`
        let pos = Position {
            line: 1,
            character: 5,
        };

        let def_loc = find_definition(tree.root_node(), code, pos, &uri);
        assert!(def_loc.is_some());
        let def_loc = def_loc.unwrap();
        assert_eq!(def_loc.range.start.line, 0);
        assert_eq!(def_loc.range.start.character, 0);
    }

    #[test]
    fn test_find_definition_uses_utf16_columns() {
        let code = "\"😀\"; x = 1;\n\"😀\"; y = x;\n";
        let tree = parser::parse(code).unwrap();
        let uri = Uri::from_str("file:///test.moo").unwrap();

        // The emoji occupies two UTF-16 code units, but four UTF-8 bytes.
        let pos = Position {
            line: 1,
            character: 10,
        };

        let def_loc = find_definition(tree.root_node(), code, pos, &uri).unwrap();
        assert_eq!(
            def_loc.range,
            Range::new(Position::new(0, 6), Position::new(0, 7))
        );
    }

    #[test]
    fn test_find_highlights() {
        let code = "x = 1;\ny = x + 2;\n";
        let tree = parser::parse(code).unwrap();
        let pos = Position {
            line: 1,
            character: 4,
        };
        let highlights = find_highlights(tree.root_node(), code, pos);
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn test_find_highlights_delimiters() {
        let code = "notify(player, \"hi\");\n";
        let tree = parser::parse(code).unwrap();
        let pos_open = Position {
            line: 0,
            character: 6,
        };
        let highlights = find_highlights(tree.root_node(), code, pos_open);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].range.start.character, 6);
        assert_eq!(highlights[1].range.start.character, 19);
    }

    #[test]
    fn test_find_highlights_block_keywords() {
        let code = "if (x)\n  b = 1;\nendif;\n";
        let tree = parser::parse(code).unwrap();
        let pos_if = Position {
            line: 0,
            character: 0,
        };
        let highlights = find_highlights(tree.root_node(), code, pos_if);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].range.start.line, 0);
        assert_eq!(highlights[1].range.start.line, 2);
    }
}
