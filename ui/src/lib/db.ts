import { drizzle } from 'drizzle-orm/pg-proxy';
import { kalamDriver } from '@kalamdb/orm';
import { getClient } from './kalam-client';

let db: ReturnType<typeof drizzle> | null = null;

export function getDb() {
  if (!db) {
    const client = getClient();
    if (!client) throw new Error('KalamDB client not initialized');
    db = drizzle(kalamDriver(client));
  }
  return db;
}

export function sqlWithInlineParams(sql: string, params: unknown[]): string {
  let compiled = sql;
  for (let index = params.length; index >= 1; index -= 1) {
    compiled = compiled.split(`$${index}`).join(sqlLiteral(params[index - 1]));
  }
  return compiled;
}

function sqlLiteral(value: unknown): string {
  if (value === null || value === undefined) {
    return "NULL";
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }
  if (typeof value === "boolean") {
    return value ? "TRUE" : "FALSE";
  }
  return `'${String(value).replace(/'/g, "''")}'`;
}
