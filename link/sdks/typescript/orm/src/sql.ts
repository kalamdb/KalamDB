import type { KalamDBClient, QueryResponse, UploadProgress, UserId } from '@kalamdb/client';
import type { SQLWrapper } from 'drizzle-orm';
import { PgDialect } from 'drizzle-orm/pg-core';
import { rewriteSqlParamsForFileUploads } from './file-upload.js';
import { stripDefaults } from './driver.js';
import { stripQuotedIdentifiers } from './query-normalize.js';

type ExecuteAsUserClient = Pick<KalamDBClient, 'executeAsUser'>;
type QueryWithFilesClient = Pick<KalamDBClient, 'queryWithFiles'>;

export interface CompiledQuery {
  sql: string;
  params: unknown[];
}

export interface QueryBuilderLike {
  toSQL(): CompiledQuery;
}

export type SqlSource = SQLWrapper | QueryBuilderLike | string;

const dialect = new PgDialect();

function hasGetSQL(value: SqlSource): value is SQLWrapper {
  return typeof value === 'object' && value !== null && 'getSQL' in value && typeof value.getSQL === 'function';
}

function hasToSQL(value: SqlSource): value is QueryBuilderLike {
  return typeof value === 'object' && value !== null && 'toSQL' in value && typeof value.toSQL === 'function';
}

function normalizeSql(sql: string): string {
  return stripQuotedIdentifiers(sql);
}

function normalizeCompiledQuery(compiled: CompiledQuery): CompiledQuery {
  return stripDefaults(normalizeSql(compiled.sql), compiled.params);
}

export function compileQuery(source: Exclude<SqlSource, string>): CompiledQuery {
  if (hasGetSQL(source)) {
    const compiled = dialect.sqlToQuery(source.getSQL());
    return normalizeCompiledQuery({ sql: compiled.sql, params: compiled.params });
  }

  if (hasToSQL(source)) {
    const compiled = source.toSQL();
    return normalizeCompiledQuery(compiled);
  }

  throw new Error('Unsupported SQL source');
}

export function compileInlineQuery(source: Exclude<SqlSource, string>): CompiledQuery {
  if (hasGetSQL(source)) {
    const compiled = dialect.sqlToQuery(source.getSQL().inlineParams());
    return { sql: normalizeSql(compiled.sql), params: [] };
  }

  if (hasToSQL(source)) {
    const compiled = source.toSQL();
    if (compiled.params.length > 0) {
      throw new Error('This query needs inline parameter support. Pass a Drizzle SQL or query builder that exposes getSQL().');
    }

    return { sql: normalizeSql(compiled.sql), params: [] };
  }

  throw new Error('Unsupported SQL source');
}

export async function executeAsUser(
  client: ExecuteAsUserClient,
  source: SqlSource,
  user: UserId | string,
): Promise<QueryResponse> {
  if (typeof source === 'string') {
    return client.executeAsUser(source, user);
  }

  const compiled = compileQuery(source);
  return client.executeAsUser(compiled.sql, user, compiled.params);
}

/**
 * Compile a Drizzle builder (or raw SQL) and execute it with FILE uploads.
 *
 * Drizzle-generated SQL is normalized for KalamDB before calling `queryWithFiles`.
 */
export async function queryWithFiles(
  client: QueryWithFilesClient,
  source: SqlSource,
  files: Record<string, File | Blob>,
  params?: unknown[],
  onProgress?: (progress: UploadProgress) => void,
): Promise<QueryResponse> {
  if (typeof source === 'string') {
    return client.queryWithFiles(source, files, params, onProgress);
  }

  const compiled = compileQuery(source);
  const rewritten = rewriteSqlParamsForFileUploads(compiled.sql, compiled.params);
  return client.queryWithFiles(
    rewritten.sql,
    { ...files, ...rewritten.files },
    rewritten.params,
    onProgress,
  );
}
