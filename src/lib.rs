// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

pub mod analysis;
pub mod builtins;
#[cfg(not(target_arch = "wasm32"))]
pub mod cli;
pub mod formatting;
pub mod inlay_hints;
pub mod line_index;
pub mod locals;
pub mod parser;
pub mod semantic_tokens;
pub mod server;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod tree_sitter_allocator;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
