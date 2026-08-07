// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

use lsp_server::Connection;
use moo_lsp_rs::server;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (connection, io_threads) = Connection::stdio();
    server::run(connection)?;
    io_threads.join()?;
    Ok(())
}
