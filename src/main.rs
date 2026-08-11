// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use lsp_server::Connection;
#[cfg(not(target_arch = "wasm32"))]
use moo_lsp_rs::cli;
use moo_lsp_rs::server;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli::run(&args));
    }

    run_lsp()
}

#[cfg(target_arch = "wasm32")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_lsp()
}

fn run_lsp() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (connection, io_threads) = Connection::stdio();
    server::run(connection)?;
    io_threads.join()?;
    Ok(())
}
