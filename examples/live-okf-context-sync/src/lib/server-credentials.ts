import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { projectRoot } from './paths.js';

const DEFAULT_ROOT_PASSWORD = 'kalamdb123';

/**
 * Resolve the root password for the local KalamDB instance.
 *
 * `kalam dev` writes the managed server password into
 * `kalam/server/server.toml`, so prefer that as the source of truth.
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
