.PHONY: wasm wasm-vscode wasm-browser wasm-targets npm-build npm-test npm-pack-check

WASI_TARGET := wasm32-wasip1-threads
BROWSER_TARGET := wasm32-unknown-unknown
WASI_SDK_PATH ?=

# Build both WebAssembly variants into the layout consumed by the VS Code
# extension and browser clients.
wasm: wasm-vscode wasm-browser

# The threaded WASI target needs the clang and sysroot shipped in the WASI SDK.
wasm-vscode:
	@test -n "$(WASI_SDK_PATH)" && test -x "$(WASI_SDK_PATH)/bin/clang" || { \
		echo "WASI_SDK_PATH must point to an installed WASI SDK" >&2; \
		echo "See README.md for setup instructions" >&2; \
		exit 1; \
	}
	CC_wasm32_wasip1_threads="$(WASI_SDK_PATH)/bin/clang" \
		WASI_SDK_PATH="$(WASI_SDK_PATH)" \
		cargo build --locked --target "$(WASI_TARGET)"
	mkdir -p dist/web
	cp "target/$(WASI_TARGET)/debug/moo-lsp-rs.wasm" dist/web/moo-lsp-rs.wasm

wasm-browser:
	@command -v wasm-pack >/dev/null || { \
		echo "wasm-pack is required; install it with: cargo install wasm-pack --locked" >&2; \
		exit 1; \
	}
	wasm-pack build --dev --target web --out-dir dist/browser

# One-time Rust target setup. wasm-pack and the WASI SDK are separate tools.
wasm-targets:
	rustup target add "$(WASI_TARGET)" "$(BROWSER_TARGET)"

npm-build:
	npm --prefix npm ci --ignore-scripts
	npm --prefix npm run build

npm-test: npm-build
	npm --prefix npm test
	npm --prefix npm run test:types
	npm --prefix npm run pack:check

npm-pack-check: npm-build
	npm --prefix npm run pack:check
