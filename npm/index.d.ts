export interface Position {
  line: number;
  character: number;
}

export interface Range {
  start: Position;
  end: Position;
}

export interface DiagnosticRelatedInformation {
  location: { uri: string; range: Range };
  message: string;
}

export interface Diagnostic {
  range: Range;
  severity?: 1 | 2 | 3 | 4;
  code?: number | string;
  source?: string;
  message: string;
  relatedInformation?: DiagnosticRelatedInformation[];
  [key: string]: unknown;
}

export interface JsonRpcMessage {
  jsonrpc: "2.0";
  [key: string]: unknown;
}

export interface LanguageServer {
  handleMessage(message: JsonRpcMessage): JsonRpcMessage[];
  dispose(): void;
}

export class FormatError extends Error {
  readonly diagnostics: Diagnostic[];
}

export function createLanguageServer(): Promise<LanguageServer>;
export function check(source: string): Promise<Diagnostic[]>;
export function format(source: string): Promise<string>;
