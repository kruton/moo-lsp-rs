export class BrowserServer {
  constructor();
  handle_message(message: string): string;
  free(): void;
}

export function check(source: string): string;
export function format(source: string): string;

export default function init(
  input?: {
    module_or_path: RequestInfo | URL | Response | BufferSource | WebAssembly.Module;
  },
): Promise<unknown>;
