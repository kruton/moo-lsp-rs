// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

mod server;

use lsp_server::Connection;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (connection, io_threads) = Connection::stdio();
    server::run(connection)?;
    io_threads.join()?;
    Ok(())
}
