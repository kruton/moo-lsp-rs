import { execFile } from "node:child_process";
import { mkdtemp, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";

const exec = promisify(execFile);

async function prepare() {
  const browserDirectory = path.dirname(fileURLToPath(import.meta.url));
  const packageDirectory = path.resolve(browserDirectory, "../../../dist/npm");
  const npmEnvironment = {
    ...Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.toLowerCase().startsWith("npm_"))),
    NPM_CONFIG_CACHE: path.resolve(browserDirectory, "../../../target/npm-cache"),
    npm_config_cache: path.resolve(browserDirectory, "../../../target/npm-cache"),
  };
  const packDirectory = await mkdtemp(path.join(tmpdir(), "moo-lsp-browser-pack-"));
  await exec("npm", ["pack", packageDirectory, "--pack-destination", packDirectory, "--json"], { env: npmEnvironment });
  const filename = (await readdir(packDirectory)).find((entry) => entry.endsWith(".tgz"));
  if (!filename) throw new Error("npm pack did not produce a tarball");
  await exec("npm", [
    "install", "--ignore-scripts", "--no-audit", "--no-fund", "--no-save",
    path.join(packDirectory, filename),
  ], { cwd: browserDirectory, env: npmEnvironment });
}

await prepare();
