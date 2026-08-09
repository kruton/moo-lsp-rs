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
plus two WebAssembly targets. Each platform archive contains the binary under a
directory named for its VS Code target, so an extension packaging job can extract
the native and `web` archives into one `dist` directory.

`moo-lsp-rs-web.tar.gz` supports vscode.dev. It contains `web/moo-lsp-rs.wasm`,
built for `wasm32-wasip1-threads`, and uses the stdio LSP transport with
`@vscode/wasm-wasi-lsp`, shared WebAssembly memory, and a workspace folder mount.
It requires `ms-vscode.wasm-wasi-core` on the host.

`moo-lsp-rs-browser.tar.gz` contains an ES module, TypeScript declarations, and
WebAssembly built for `wasm32-unknown-unknown`. This package is intended for
CodeMirror and other browser clients.

The browser module has no WASI dependency and exchanges complete, headerless
JSON-RPC messages. Run it in a dedicated worker and pass CodeMirror LSP messages
directly to `BrowserServer.handle_message`. The method accepts one serialized
JSON-RPC object and returns a serialized array of response/notification objects:

```js
import init, { BrowserServer } from "./browser/moo_lsp_rs.js";

await init();
const server = new BrowserServer();

self.onmessage = ({ data }) => {
  const outgoing = JSON.parse(server.handle_message(JSON.stringify(data)));
  for (const message of outgoing) self.postMessage(message);
};
```

The native and VS Code web binaries use the stdio transport. The browser package
does not import `wasi:thread-spawn` and does not require a VS Code extension host.

Every archive and the release checksum manifest have GitHub build-provenance
attestations. After downloading an asset, verify its origin and integrity with:

```sh
gh attestation verify moo-lsp-rs-linux-x64.tar.gz --repo kruton/moo-lsp-rs
sha256sum --check SHA256SUMS
```

The workflow can also be run manually to test every build without publishing a
GitHub release.

## Development WebAssembly builds

Install the Rust targets and `wasm-pack` once:

```sh
make wasm-targets
cargo install wasm-pack --locked
```

The VS Code build also needs the [WASI SDK](https://github.com/WebAssembly/wasi-sdk/releases)
because Tree-sitter includes C code. Set `WASI_SDK_PATH` to the extracted SDK,
then build both development bundles:

```sh
make wasm WASI_SDK_PATH=/path/to/wasi-sdk
```

The output is written to `dist/web/moo-lsp-rs.wasm` for VS Code and
`dist/browser/` for browser clients. To build only one bundle, use
`make wasm-vscode WASI_SDK_PATH=/path/to/wasi-sdk` or `make wasm-browser`.

## License
Licensed under the MIT License. See LICENSE or file headers for details.
