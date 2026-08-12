import { check, createLanguageServer, Diagnostic, FormatError, format } from "@kruton/moo-lsp";

const diagnostics: Diagnostic[] = await check("return;");
const formatted: string = await format("return;");
const server = await createLanguageServer();
server.handleMessage({ jsonrpc: "2.0", method: "initialized", params: {} });
server.dispose();
void [diagnostics, formatted, FormatError];
