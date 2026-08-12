import { check, createLanguageServer, format } from "@kruton/moo-lsp";

globalThis.mooLspTest = (async () => {
  const diagnostics = await check("return 1\n");
  const formatted = await format("if (x)\nreturn;\nendif\n");
  const server = await createLanguageServer();
  const messages = server.handleMessage({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: { capabilities: {} },
  });
  server.dispose();
  return { diagnostics, formatted, messages };
})();
