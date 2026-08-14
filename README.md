# moo-lsp-rs

An LSP (Language Server Protocol) server for LambdaMOO code. It uses the
[Tree-sitter-lambdamoo](https://github.com/kruton/tree-sitter-lambdamoo) parser
to assist developers in writing code. It features the ability to pinpoint errors
at the character granularity. Features include syntax highlighting and code
formatting.

Running `moo-lsp-rs` without arguments starts the stdio-based language server;
`moo-lsp-rs lsp` is the explicit equivalent. Other commands are possible and can
be observed with `moo-lsp-rs --help`.

## Command-line checking and formatting

`moo-lsp-rs` can be used to check syntax and format files from the commmand-line.

### Syntax checking

```console
$ moo-lsp-rs check example.moo
example.moo:1:9: error[missing-semicolon]: Missing ';' at end of statement
1 | return 1
  | --------^
```

Pass multiple files, directories (searched recursively for files ending in `.moo`),
or `-` to read stdin. Use `--json` for stable, versioned machine-readable output and
`--deny-warnings` when warnings should make the check fail:

```sh
moo-lsp-rs check src/
moo-lsp-rs check --json example.moo
moo-lsp-rs check --deny-warnings example.moo
```

### Formatting code

Formatting writes one file or stdin to stdout by default. Use `--check` to verify
formatting without changing files, or `--write` to update files in place:

```sh
moo-lsp-rs format example.moo
moo-lsp-rs format --check src/
moo-lsp-rs format --write src/
```

Both commands return status 0 for success, 1 for diagnostics or formatting
differences, and 2 for usage or I/O errors. Formatting refuses invalid source.

## Agent skill

The repository includes a portable `lambdamoo-coding` agent skill. It currently
allows the agent to syntax check and format code via a skill.

Install it for supported coding agents with:

```sh
npx skills add kruton/moo-lsp-rs --skill lambdamoo-coding
```

## Editor setup

Install a release binary or build the server with `cargo build --release`, then
make sure `moo-lsp-rs` is available on your `PATH`.

### Vim with YouCompleteMe

First, teach Vim to recognize LambdaMOO source files. Add the following to
`~/.vim/ftdetect/moo.vim`:

```vim
augroup moo_filetype
  autocmd!
  autocmd BufRead,BufNewFile *.moo setfiletype moo
augroup END
```

Then add the server to your YouCompleteMe configuration, for example in your
`.vimrc`:

```vim
let g:ycm_language_server = get(g:, 'ycm_language_server', []) + [
    \ {
    \   'name': 'moo-lsp-rs',
    \   'cmdline': ['moo-lsp-rs'],
    \   'filetypes': ['moo'],
    \   'project_root_files': ['.git'],
    \ },
    \ ]

augroup moo_ycm
  autocmd!
  autocmd FileType moo let b:ycm_enable_semantic_highlighting = 1
augroup END
```

Restart Vim and open a `.moo` file. Run `:YcmDebugInfo` if you need to confirm
that YouCompleteMe found and started the server.

### Vim with vim-lsp

Install [vim-lsp](https://github.com/prabirshrestha/vim-lsp), configure the
`moo` filetype as shown above, and register the server in your `.vimrc`:

```vim
if executable('moo-lsp-rs')
  augroup moo_lsp
    autocmd!
    autocmd User lsp_setup call lsp#register_server({
        \ 'name': 'moo-lsp-rs',
        \ 'cmd': {server_info->['moo-lsp-rs']},
        \ 'allowlist': ['moo'],
        \ })
  augroup END
endif
```

Open a `.moo` file and use commands such as `:LspDefinition`, `:LspHover`, and
`:LspDocumentFormat`. See `:help vim-lsp` for available commands and optional
key mappings.

`moo-lsp-rs` does not currently advertise semantic completion, so vim-lsp will
not offer LSP completion candidates for LambdaMOO code.

## Remote MOO document locations

When a client advertises the standard
`InitializeParams.capabilities.window.showDocument.support` capability,
“Go to Definition” may return transport-independent remote verb locations.
The canonical form is:

```text
moo://<authority>/object/<number>/verb/<name>
```

The authority and current object are taken from the open `moo:` document; the
language server never receives WebDAV endpoints or credentials. Statically
known object-valued property traversal uses typed path segments, for example
`$local.webdav:foo()` links to:

```text
moo://<authority>/object/0/property/local/object/property/webdav/object/verb/foo
```

Each `/object` directory exposes the referenced object's WebDAV tree. Property
and verb names preserve their source case and are UTF-8 percent-encoded as
individual path segments. Clients that provide the `moo:` filesystem scheme
are responsible for resolving and opening these resources.

### Emacs with Eglot

Emacs 29 and later include Eglot. Add this configuration to your init file; the
small derived mode can be omitted if you already use a LambdaMOO major mode:

```elisp
(require 'eglot)

(define-derived-mode moo-mode prog-mode "MOO"
  "Major mode for editing LambdaMOO source files.")

(add-to-list 'auto-mode-alist '("\\.moo\\'" . moo-mode))
(add-to-list 'eglot-server-programs '(moo-mode . ("moo-lsp-rs")))
(add-hook 'moo-mode-hook #'eglot-ensure)
```

To use newer Eglot features than those provided by the version bundled with
Emacs, install the current Eglot release from GNU ELPA with
`M-x package-install RET eglot RET`. For example, replace the final `add-hook`
above with the following to enable the semantic tokens provided
by `moo-lsp-rs`:

```elisp
(defun moo-eglot-setup ()
  (eglot-ensure)
  (eglot-semantic-tokens-mode 1))

(add-hook 'moo-mode-hook #'moo-eglot-setup)
```

After evaluating the configuration, open a `.moo` file. Use
`M-x eglot-events-buffer` to inspect the connection if the server does not
start.

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

## JavaScript package

The `wasm32-unknown-unknown` server is also packaged as
`@kruton/moo-lsp` for Node.js and browsers. It exposes lazy, asynchronous
factories for independent LSP sessions plus direct `check()` and `format()`
operations. Generated wasm-bindgen names are private package internals.

Build and inspect the publishable package with:

```sh
make npm-build
make npm-pack-check
```

The disposable artifact is assembled in `dist/npm`. See `npm/README.md` for
the public JavaScript API.

## Related projects

- [vscode-lambdamoo](https://github.com/kruton/vscode-lambdamoo) - Plugin using this LSP in VS Code (and clones)
- [codemirror-lambdamoo](https://github.com/kruton/codemirror-lambdamoo) - Language extension support for CodeMirror using this LSP
- [tree-sitter-lambdamoo](http://github.com/kruton/tree-sitter-lambdamoo) - Tree-sitter parser language support for LambdaMOO used in this LSP

## License
Licensed under the MIT License. See LICENSE or file headers for details.
