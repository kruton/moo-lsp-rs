import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";

const exec = promisify(execFile);
const npmDirectory = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packageDirectory = path.resolve(npmDirectory, "..", "dist", "npm");
const npmEnvironment = {
  ...Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.toLowerCase().startsWith("npm_"))),
  NPM_CONFIG_CACHE: path.resolve(npmDirectory, "..", "target", "npm-cache"),
  npm_config_cache: path.resolve(npmDirectory, "..", "target", "npm-cache"),
};

async function installedFixture() {
  const packDirectory = await mkdtemp(path.join(tmpdir(), "moo-lsp-pack-"));
  const fixture = await mkdtemp(path.join(tmpdir(), "moo-lsp-node-"));
  await exec("npm", ["pack", packageDirectory, "--pack-destination", packDirectory, "--json"], { env: npmEnvironment });
  const filename = (await readdir(packDirectory)).find((entry) => entry.endsWith(".tgz"));
  assert.ok(filename, "npm pack did not produce a tarball");
  await writeFile(path.join(fixture, "package.json"), '{"type":"module"}\n');
  await exec("npm", ["install", "--ignore-scripts", "--no-audit", "--no-fund", path.join(packDirectory, filename)], { cwd: fixture, env: npmEnvironment });
  return fixture;
}

test("packed Node package exposes independent LSP sessions and direct tools", async () => {
  const fixture = await installedFixture();
  const script = `
    import assert from "node:assert/strict";
    import { FormatError, check, createLanguageServer, format } from "@kruton/moo-lsp";
    const diagnostics = await check("return 1\\n");
    assert.ok(diagnostics.some((item) => item.code === "missing-semicolon"));
    assert.equal(await format("if (x)\\nreturn;\\nendif\\n"), "if (x)\\n  return;\\nendif\\n");
    await assert.rejects(() => format("if (x)\\nreturn;\\n"), (error) => error instanceof FormatError && error.diagnostics.length > 0);
    const first = await createLanguageServer();
    const second = await createLanguageServer();
    for (const server of [first, second]) {
      const outgoing = server.handleMessage({ jsonrpc: "2.0", id: 1, method: "initialize", params: { capabilities: {} } });
      assert.equal(outgoing[0].id, 1);
    }
    assert.throws(
      () => second.handleMessage({ jsonrpc: "2.0" }),
      (error) => error instanceof Error,
    );
    first.dispose();
    first.dispose();
    assert.throws(() => first.handleMessage({ jsonrpc: "2.0", method: "initialized", params: {} }), /disposed/);
    second.dispose();
  `;
  const scriptPath = path.join(fixture, "test.mjs");
  await writeFile(scriptPath, script);
  await exec(process.execPath, [scriptPath], { cwd: fixture });
  const metadata = JSON.parse(await readFile(path.join(fixture, "node_modules", "@kruton", "moo-lsp", "package.json"), "utf8"));
  assert.equal(metadata.scripts, undefined);
  assert.equal(metadata.devDependencies, undefined);
});
