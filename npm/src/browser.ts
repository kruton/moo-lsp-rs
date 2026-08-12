import init, {
  BrowserServer,
  check as rawCheck,
  format as rawFormat,
} from "./raw/moo_lsp_rs.js";
import type { Diagnostic, LanguageServer } from "../index.js";
import { FormatError, parseDiagnostics, parseFormatResult, wrapServer } from "./shared.js";

let initialization: Promise<unknown> | undefined;

function initialize(): Promise<unknown> {
  return initialization ??= init({
    module_or_path: new URL("./raw/moo_lsp_rs_bg.wasm", import.meta.url),
  });
}

export async function createLanguageServer(): Promise<LanguageServer> {
  await initialize();
  return wrapServer(new BrowserServer());
}

export async function check(source: string): Promise<Diagnostic[]> {
  await initialize();
  return parseDiagnostics(rawCheck(source));
}

export async function format(source: string): Promise<string> {
  await initialize();
  return parseFormatResult(rawFormat(source));
}

export { FormatError };
