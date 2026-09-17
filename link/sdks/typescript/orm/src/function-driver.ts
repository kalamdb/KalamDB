import { drizzle } from 'drizzle-orm/pg-proxy';
import type { RemoteCallback } from 'drizzle-orm/pg-proxy';
import { stripQuotedIdentifiers } from './query-normalize.js';
import { stripDefaults } from './strip-defaults.js';

const EXECUTE_AS_USER = /^[A-Za-z0-9._-]+$/;

export interface FunctionDbHost {
  query(sql: string, params?: unknown[]): Promise<unknown>;
  execute(sql: string, params?: unknown[]): Promise<unknown>;
}

export interface KalamFunctionDriverOptions {
  executeAs?: string;
}

function wrapExecuteAs(sql: string, user: string): string {
  if (!EXECUTE_AS_USER.test(user)) {
    throw new Error(`unsupported user for EXECUTE AS: ${user}`);
  }
  return `EXECUTE AS '${user}' (${sql})`;
}

export function toDrizzleRows(result: unknown): unknown[][] {
  if (result == null) {
    return [];
  }
  const list = Array.isArray(result) ? result : [result];
  return list.map((row) => {
    if (Array.isArray(row)) {
      return row;
    }
    if (row && typeof row === 'object') {
      return Object.values(row as Record<string, unknown>);
    }
    return [row];
  });
}

export function kalamFunctionDriver(
  db: FunctionDbHost,
  options?: KalamFunctionDriverOptions,
): RemoteCallback {
  const executeAs = options?.executeAs;
  return async (sql, params, method) => {
    const stripped = stripDefaults(stripQuotedIdentifiers(sql), params ?? []);
    const text = executeAs ? wrapExecuteAs(stripped.sql, executeAs) : stripped.sql;
    if (method === 'execute') {
      await db.execute(text, stripped.params);
      return { rows: [] };
    }
    const result = await db.query(text, stripped.params);
    return { rows: toDrizzleRows(result) };
  };
}

export function kalamFunctionDb(db: FunctionDbHost, options?: KalamFunctionDriverOptions) {
  return drizzle(kalamFunctionDriver(db, options));
}

export type FunctionOrm = ReturnType<typeof kalamFunctionDb>;

export type ProcedureOrm = FunctionOrm & {
  as(executeAs: string): FunctionOrm;
};

export function bindFunctionOrm(db: FunctionDbHost): ProcedureOrm {
  const orm = kalamFunctionDb(db) as ProcedureOrm;
  const cache = new Map<string, FunctionOrm>();
  Object.defineProperty(orm, 'as', {
    enumerable: false,
    value: (executeAs: string) => {
      const existing = cache.get(executeAs);
      if (existing) {
        return existing;
      }
      const bound = kalamFunctionDb(db, { executeAs });
      cache.set(executeAs, bound);
      return bound;
    },
  });
  return orm;
}
