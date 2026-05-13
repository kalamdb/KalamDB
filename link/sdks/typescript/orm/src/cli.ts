#!/usr/bin/env node
import { createClient, Auth } from '@kalamdb/client';
import { generateSchema } from './generate.js';
import type { KalamSystemColumnName } from './ktable.js';
import { writeFileSync } from 'fs';

const args = process.argv.slice(2);

function getArg(name: string): string | undefined {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : undefined;
}

function getArgs(name: string): string[] {
  const values: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === `--${name}` && args[index + 1]) {
      values.push(args[index + 1]);
    }
  }

  return values.flatMap((value) => value.split(',')).map((value) => value.trim()).filter((value) => value.length > 0);
}

function getOptionalArg(name: string): string | true | undefined {
  const index = args.indexOf(`--${name}`);
  if (index < 0) return undefined;

  const next = args[index + 1];
  if (!next || next.startsWith('--')) return true;
  return next;
}

function parseSystemColumnsArg(value: string | true | undefined): boolean | 'all' | KalamSystemColumnName[] | undefined {
  if (!value) return undefined;
  if (value === true || value === 'true') return true;
  if (value === 'all') return 'all' as const;
  const columns = value.split(',').map((item) => item.trim()).filter((item) => item.length > 0);
  const supported = new Set<KalamSystemColumnName>(['_seq', '_deleted', '_commit_seq']);
  for (const column of columns) {
    if (!supported.has(column as KalamSystemColumnName)) {
      throw new Error(`Unsupported system column: ${column}`);
    }
  }

  return columns as KalamSystemColumnName[];
}

function parseBigIntMode(value: string | undefined): 'string' | 'bigint' | 'number' | undefined {
  if (!value) return undefined;
  if (value === 'string' || value === 'bigint' || value === 'number') return value;
  throw new Error(`Unsupported --bigint-mode: ${value}. Expected string, bigint, or number.`);
}

const url = getArg('url') || process.env.KALAMDB_URL || 'http://localhost:2900';
const user = getArg('user') || process.env.KALAMDB_USER || 'admin';
const password = getArg('password') || process.env.KALAMDB_PASSWORD;
const out = getArg('out') || 'schema.ts';
const includeSystem = args.includes('--include-system');
const namespaces = getArgs('namespace');
const includeSystemColumns = parseSystemColumnsArg(getOptionalArg('include-system-columns'));
const bigIntMode = parseBigIntMode(getArg('bigint-mode'));
const includeTypeAliases = !args.includes('--no-type-aliases');

if (!password) {
  console.error('Usage: kalamdb-orm --url <url> --user <user> --password <pass> --out <file>');
  console.error('');
  console.error('Options:');
  console.error('  --url <url>          KalamDB server URL (default: http://localhost:2900)');
  console.error('  --user <user>        Username (default: admin)');
  console.error('  --password <pass>    Password (required)');
  console.error('  --out <file>         Output file (default: schema.ts)');
  console.error('  --include-system     Include system/dba tables');
  console.error('  --include-system-columns [all|_seq,_deleted]');
  console.error('                       Add KalamDB hidden columns to generated table types');
  console.error('  --namespace <name>   Limit output to one or more namespaces (repeatable or comma-separated)');
  console.error('  --bigint-mode <mode> Generate BIGINT as string (default), bigint, or number');
  console.error('  --no-type-aliases    Do not emit $inferSelect/$inferInsert aliases');
  process.exit(1);
}

async function main() {
  const client = createClient({
    url,
    authProvider: async () => Auth.basic(user, password!),
  });
  await client.initialize();

  const options = { includeSystem, namespaces, includeSystemColumns, bigIntMode, includeTypeAliases };
  const schema = await generateSchema(client, options);
  writeFileSync(out, schema);
  const tableCount = (schema.match(/^export const \w+ = kTable(?:\.\w+)?\(/gm) || []).length;
  console.log(`Generated ${tableCount} tables → ${out}`);

  await client.disconnect();
}

main().catch((error) => {
  console.error('Error:', error.message);
  process.exit(1);
});
