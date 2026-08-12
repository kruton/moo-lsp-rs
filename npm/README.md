# @kruton/moo-lsp

LambdaMOO syntax analysis, formatting, and an in-process Language Server
Protocol session for Node.js and browsers.

```js
import { check, createLanguageServer, format } from "@kruton/moo-lsp";

const diagnostics = await check("return 1\n");
const formatted = await format("if (ready)\nreturn;\nendif\n");

const server = await createLanguageServer();
const messages = server.handleMessage({
  jsonrpc: "2.0",
  id: 1,
  method: "initialize",
  params: { capabilities: {} },
});
server.dispose();
```

`format()` rejects with `FormatError` when the source is invalid; its
`diagnostics` property describes the errors. Calls on one `LanguageServer`
instance are synchronous and must not overlap. Create separate instances when
independent document state is required.

The WebAssembly module initializes lazily on the first API call. The package has
no install scripts and does not require Rust or a native compiler.
