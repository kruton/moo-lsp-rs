import { execFile } from "node:child_process";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import path from "node:path";

const exec = promisify(execFile);
const npmDirectory = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repository = path.resolve(npmDirectory, "..");
const output = path.join(repository, "dist", "npm");
const raw = path.join(npmDirectory, "raw");

await rm(output, { recursive: true, force: true });
await rm(raw, { recursive: true, force: true });
await mkdir(output, { recursive: true });

await exec(process.env.MOO_LSP_WASM_PACK ?? "wasm-pack", [
  "build",
  "--release",
  "--target", "web",
  "--out-dir", raw,
], {
  cwd: repository,
  env: {
    ...process.env,
    CCACHE_DIR: path.join(repository, "target", "ccache"),
    WASM_PACK_CACHE: path.join(repository, "target", "wasm-pack-cache"),
  },
});
await exec(path.join(npmDirectory, "node_modules", ".bin", "tsc"), [
  "-p", path.join(npmDirectory, "tsconfig.json"),
], { cwd: repository });

await mkdir(path.join(output, "raw"), { recursive: true });
for (const filename of ["moo_lsp_rs.js", "moo_lsp_rs.d.ts", "moo_lsp_rs_bg.wasm", "moo_lsp_rs_bg.wasm.d.ts"]) {
  await cp(path.join(raw, filename), path.join(output, "raw", filename));
}
await cp(path.join(npmDirectory, "index.d.ts"), path.join(output, "index.d.ts"));
await cp(path.join(npmDirectory, "README.md"), path.join(output, "README.md"));
await cp(path.join(repository, "LICENSE"), path.join(output, "LICENSE"));

const cargo = await readFile(path.join(repository, "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
const packageJson = JSON.parse(await readFile(path.join(npmDirectory, "package.json"), "utf8"));
if (!cargoVersion || cargoVersion !== packageJson.version) {
  throw new Error(`Cargo version ${cargoVersion ?? "<missing>"} does not match npm version ${packageJson.version}`);
}
delete packageJson.scripts;
delete packageJson.devDependencies;
await writeFile(path.join(output, "package.json"), `${JSON.stringify(packageJson, null, 2)}\n`);
