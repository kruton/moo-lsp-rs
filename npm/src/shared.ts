import type { BrowserServer } from "./raw/moo_lsp_rs.js";
import type { Diagnostic, JsonRpcMessage, LanguageServer } from "../index.js";

interface FormatSuccess {
  ok: true;
  formatted: string;
}

interface FormatFailure {
  ok: false;
  diagnostics: Diagnostic[];
}

export class FormatError extends Error {
  readonly diagnostics: Diagnostic[];

  constructor(diagnostics: Diagnostic[]) {
    super("Cannot format invalid LambdaMOO source");
    this.name = "FormatError";
    this.diagnostics = diagnostics;
  }
}

function asError(error: unknown): Error {
  if (error instanceof Error) return error;
  return new Error(typeof error === "string" ? error : String(error));
}

export function wrapServer(inner: BrowserServer): LanguageServer {
  let disposed = false;
  return {
    handleMessage(message: JsonRpcMessage): JsonRpcMessage[] {
      if (disposed) throw new Error("Language server has been disposed");
      try {
        return JSON.parse(inner.handle_message(JSON.stringify(message))) as JsonRpcMessage[];
      } catch (error) {
        throw asError(error);
      }
    },
    dispose(): void {
      if (!disposed) {
        disposed = true;
        inner.free();
      }
    },
  };
}

export function parseDiagnostics(json: string): Diagnostic[] {
  try {
    return JSON.parse(json) as Diagnostic[];
  } catch (error) {
    throw asError(error);
  }
}

export function parseFormatResult(json: string): string {
  try {
    const result = JSON.parse(json) as FormatSuccess | FormatFailure;
    if (result.ok) return result.formatted;
    throw new FormatError(result.diagnostics);
  } catch (error) {
    throw asError(error);
  }
}
