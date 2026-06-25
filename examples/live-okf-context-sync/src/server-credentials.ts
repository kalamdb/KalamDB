import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const DEFAULT_ROOT_PASSWORD = 'kalamdb123';

function projectRoot(): string {
  return resolve(dirname(fileURLToPath(import.meta.url)), '..');
}

/**
 * Resolve the root password for the local KalamDB instance.
 *
 * `kalam dev` writes the managed server password into
 * `kalam/server/server.toml`, so prefer that as the source of truth. This keeps
 * the example working no matter which password the local server was scaffolded
 * with. An explicit `KALAM_ROOT_PASSWORD` always wins, and we fall back to the
 * documented default when no local server config is present (e.g. when pointing
 * at a remote server).
 *
 * This module is Node-only: it touches the filesystem and must never be
 * imported from browser code.
 */
export function resolveRootPassword(env: Record<string, string | undefined> = process.env): string {
  if (env.KALAM_ROOT_PASSWORD) {
    return env.KALAM_ROOT_PASSWORD;
  }

  try {
    const toml = readFileSync(resolve(projectRoot(), 'kalam/server/server.toml'), 'utf8');
    const match = toml.match(/root_password\s*=\s*"([^"]*)"/);
    if (match?.[1]) {
      return match[1];
    }
  } catch {
    // server.toml is absent (remote server or first run); use the default.
  }

  return DEFAULT_ROOT_PASSWORD;
}
