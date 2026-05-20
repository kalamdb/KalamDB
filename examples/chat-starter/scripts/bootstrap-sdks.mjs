// Builds the @kalamdb/* SDKs that chat-starter consumes via file: links.
//
// Run this ONCE after cloning the monorepo, BEFORE `npm install` here.
// Without it, `npm install` resolves the file: deps to packages whose
// dist/ folders are gitignored (don't exist on a fresh clone), and the
// Vite build later fails with import errors.
//
// Plain .mjs (not .ts) on purpose: this script runs BEFORE chat-starter's
// own npm install, so we can't yet depend on `tsx`. Node's built-in ESM
// loader is enough.
//
// Prerequisites: Rust toolchain + wasm-pack (the `client` and `consumer`
// SDKs include a Rust WASM crate that must be compiled). The `orm` and
// `react` SDKs are pure TypeScript and build in seconds.
//
// When chat-starter eventually moves to its own repo with published
// @kalamdb/* deps from npm, this whole script becomes unnecessary.

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SDK_ROOT = resolve(__dirname, "..", "..", "..", "link", "sdks", "typescript");
const PACKAGES = ["client", "consumer", "orm", "react"];

function run(cmd, args, cwd) {
  // shell: true so `npm` resolves to `npm.cmd` on Windows without us
  // having to special-case the launcher name.
  const result = spawnSync(cmd, args, { cwd, stdio: "inherit", shell: true });
  if (result.status !== 0) {
    process.stderr.write(
      `\n${cmd} ${args.join(" ")} (in ${cwd}) failed with code ${result.status}\n`,
    );
    process.exit(result.status ?? 1);
  }
}

for (const pkg of PACKAGES) {
  const pkgRoot = resolve(SDK_ROOT, pkg);
  if (!existsSync(pkgRoot)) {
    process.stderr.write(`SDK directory missing: ${pkgRoot}\nIs the monorepo checkout complete?\n`);
    process.exit(1);
  }
  process.stdout.write(`\n>>> Building @kalamdb/${pkg}\n`);
  run("npm", ["install"], pkgRoot);
  run("npm", ["run", "build"], pkgRoot);
}

process.stdout.write(
  "\nDone. The @kalamdb/* dist/ folders are now built. Continue with `npm install`.\n",
);
