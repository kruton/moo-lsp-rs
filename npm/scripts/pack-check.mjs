import { spawnSync } from "node:child_process";
import { readdir, readFile, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const npmDirectory = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packageDirectory = path.resolve(npmDirectory, "..", "dist", "npm");
const npmEnvironment = {
  ...Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.toLowerCase().startsWith("npm_"))),
  NPM_CONFIG_CACHE: path.resolve(npmDirectory, "..", "target", "npm-cache"),
  npm_config_cache: path.resolve(npmDirectory, "..", "target", "npm-cache"),
};
const packed = spawnSync("npm", ["pack", "--dry-run", "--json"], {
  cwd: packageDirectory,
  env: npmEnvironment,
  encoding: "utf8",
});
if (packed.status !== 0) throw new Error(packed.stderr || `npm pack exited ${packed.status}`);
async function listFiles(directory, prefix = "") {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...await listFiles(path.join(directory, entry.name), relative));
    else files.push(relative);
  }
  return files;
}
const files = new Set(await listFiles(packageDirectory));
for (const required of [
  "LICENSE", "README.md", "browser.js", "index.d.ts", "node.js", "package.json",
  "raw/moo_lsp_rs.d.ts", "raw/moo_lsp_rs.js", "raw/moo_lsp_rs_bg.wasm",
  "shared.js",
]) {
  if (!files.has(required)) throw new Error(`packed artifact is missing ${required}`);
}
for (const forbidden of ["Cargo.toml", "src/web.rs", "raw/package.json"]) {
  if (files.has(forbidden)) throw new Error(`packed artifact unexpectedly contains ${forbidden}`);
}
const metadata = JSON.parse(await readFile(path.join(packageDirectory, "package.json"), "utf8"));
if (metadata.name !== "@kruton/moo-lsp") throw new Error(`unexpected package name ${metadata.name}`);
const wasmSize = (await stat(path.join(packageDirectory, "raw", "moo_lsp_rs_bg.wasm"))).size;
process.stdout.write(`${metadata.name}@${metadata.version}: ${files.size} files assembled, ${wasmSize} byte WASM verified by npm pack --dry-run\n`);
