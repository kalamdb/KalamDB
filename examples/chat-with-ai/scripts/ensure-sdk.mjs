#!/usr/bin/env node
/**
 * Ensures local TypeScript SDK packages are compiled before running the example.
 * Runs on any OS — no bash required.
 */

import { existsSync, mkdirSync, copyFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(__dirname, '..');
const sdkDir = resolve(projectDir, '../../link/sdks/typescript');
const clientDir = resolve(sdkDir, 'client');
const consumerDir = resolve(sdkDir, 'consumer');
const ormDir = resolve(sdkDir, 'orm');

function run(cmd, args, cwd) {
  const result = spawnSync(cmd, args, { cwd, stdio: 'inherit', shell: process.platform === 'win32' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function ensureBuilt(name, dir, buildScript) {
  const entry = resolve(dir, 'dist/src/index.js');
  if (!existsSync(entry)) {
    console.log(`${name} not compiled — building...`);
    run('npm', ['install', '--no-package-lock'], dir);
    run('npm', ['run', buildScript], dir);
    console.log(`${name} ready`);
  } else {
    console.log(`${name} is ready`);
  }
}

ensureBuilt('@kalamdb/client', clientDir, 'build');
ensureBuilt('@kalamdb/consumer', consumerDir, 'build');
ensureBuilt('@kalamdb/orm', ormDir, 'build');

// Copy WASM to public/wasm/ so Vite can serve it
const wasmSrc = resolve(clientDir, 'dist/wasm/kalam_client_bg.wasm');
const wasmDst = resolve(projectDir, 'public/wasm/kalam_client_bg.wasm');
if (existsSync(wasmSrc)) {
  mkdirSync(dirname(wasmDst), { recursive: true });
  if (!existsSync(wasmDst)) {
    copyFileSync(wasmSrc, wasmDst);
    console.log('WASM file ready at public/wasm/kalam_client_bg.wasm');
  } else {
    console.log('WASM file in public/ is up to date');
  }
}
