# moo-lsp-rs

An LSP (Language Server Protocol) server for LambdaMOO code, written in Rust.

## Features
- Standalone parser independent of upstream LambdaMOO code.
- Diagnostics with line and character precision.
- Comprehensive test suite verifying against example MOO code.

## Release binaries

Version tags matching `vX.Y.Z` build native language-server binaries for the VS Code
desktop targets `linux-x64`, `linux-arm64`, `linux-armhf`, `alpine-x64`,
`alpine-arm64`, `darwin-x64`, `darwin-arm64`, `win32-x64`, and `win32-arm64`.
Each archive contains the binary under a directory named for its VS Code target,
so an extension packaging job can extract all nine archives into one `dist`
directory.

Every archive and the release checksum manifest have GitHub build-provenance
attestations. After downloading an asset, verify its origin and integrity with:

```sh
gh attestation verify moo-lsp-rs-linux-x64.tar.gz --repo kruton/moo-lsp-rs
sha256sum --check SHA256SUMS
```

The workflow can also be run manually to test every build without publishing a
GitHub release.

## Background
This project was built by Antigravity with Gemini.

## License
Licensed under the MIT License. See LICENSE or file headers for details.
