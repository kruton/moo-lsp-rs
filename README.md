# moo-lsp-rs

An LSP (Language Server Protocol) server for LambdaMOO code. It uses the
[Tree-sitter-lambdamoo](https://github.com/kruton/tree-sitter-lambdamoo) parser
to assist developers in writing code. It features the ability to pinpoint errors
at the character granularity. Features include syntax highlighting and code
formatting.

## Release binaries

Version tags matching `vX.Y.Z` build language-server binaries for the VS Code
desktop targets `linux-x64`, `linux-arm64`, `linux-armhf`, `alpine-x64`,
`alpine-arm64`, `darwin-x64`, `darwin-arm64`, `win32-x64`, and `win32-arm64`,
plus a `web` target for vscode.dev. Each archive contains the binary under a
directory named for its VS Code target, so an extension packaging job can extract
all ten archives into one `dist` directory. The web archive contains
`web/moo-lsp-rs.wasm`, built for `wasm32-wasip1-threads`.

The web binary uses the same stdio LSP transport as the native binaries. A web
extension should run it with `@vscode/wasm-wasi-lsp`, using
`createStdioOptions()`, `startServer()`, shared WebAssembly memory, and a workspace
folder mount. This requires the `ms-vscode.wasm-wasi-core` extension on the host.

Every archive and the release checksum manifest have GitHub build-provenance
attestations. After downloading an asset, verify its origin and integrity with:

```sh
gh attestation verify moo-lsp-rs-linux-x64.tar.gz --repo kruton/moo-lsp-rs
sha256sum --check SHA256SUMS
```

The workflow can also be run manually to test every build without publishing a
GitHub release.

## License
Licensed under the MIT License. See LICENSE or file headers for details.
