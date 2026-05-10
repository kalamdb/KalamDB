import type { RowData } from '../cell_value.js';
import type { LiveGetKey } from '../types.js';
import type { LiveProjectionPlan } from './projection.js';
import { parseLiveOrderBy } from './projection.js';

export type LiveQueryDescriptorMode = 'sql' | 'drizzle';

export interface LiveQueryDescriptor<TRow = RowData> {
  mode: LiveQueryDescriptorMode;
  name?: string;
  sourceSql: string;
  subscriptionSql: string;
  projection: LiveProjectionPlan<TRow>;
  tableName?: string;
  getKey?: LiveGetKey<TRow>;
  mapRow?: (row: RowData) => TRow;
}

export interface LiveQueryDescriptorInput<TRow = RowData> {
  mode: LiveQueryDescriptorMode;
  name?: string;
  sourceSql: string;
  subscriptionSql?: string;
  projection?: LiveProjectionPlan<TRow>;
  tableName?: string;
  getKey?: LiveGetKey<TRow>;
  mapRow?: (row: RowData) => TRow;
}

export interface RawSqlLiveDescriptorOptions<TRow = RowData> {
  name?: string;
  limit?: number;
  getKey?: LiveGetKey<TRow>;
  mapRow?: (row: RowData) => TRow;
}

export interface NormalizedLiveSql {
  sourceSql: string;
  subscriptionSql: string;
  tableName: string;
  projection: LiveProjectionPlan<RowData>;
}

const UNSUPPORTED_LIVE_SQL = /\b(group\s+by|having|union|intersect|except|join|with|distinct)\b/i;
const TABLE_REF = '(?:"[^"]+"|[A-Za-z_][\\w$]*)(?:\\.(?:"[^"]+"|[A-Za-z_][\\w$]*))?';
const LIVE_SELECT_RE = new RegExp(
  `^select\\s+(.+?)\\s+from\\s+(${TABLE_REF})(?:\\s+where\\s+(.+?))?(?:\\s+order\\s+by\\s+(.+?))?(?:\\s+limit\\s+(\\d+))?\\s*$`,
  'i',
);

export class LiveQueryDescriptorError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'LiveQueryDescriptorError';
  }
}

export function createLiveQueryDescriptor<TRow>(input: LiveQueryDescriptorInput<TRow>): LiveQueryDescriptor<TRow> {
  const subscriptionSql = (input.subscriptionSql ?? input.sourceSql).trim();
  if (!subscriptionSql) {
    throw new LiveQueryDescriptorError('Live query descriptor requires a subscription SQL string.');
  }

  return {
    mode: input.mode,
    ...(input.name ? { name: input.name } : {}),
    sourceSql: input.sourceSql.trim(),
    subscriptionSql,
    projection: input.projection ?? {},
    ...(input.tableName ? { tableName: input.tableName } : {}),
    ...(input.getKey ? { getKey: input.getKey } : {}),
    ...(input.mapRow ? { mapRow: input.mapRow } : {}),
  };
}

export function createRawSqlLiveDescriptor(
  query: string,
  options: RawSqlLiveDescriptorOptions<RowData> = {},
): LiveQueryDescriptor<RowData> {
  const normalized = normalizeLiveSql(query, options.limit);

  return createLiveQueryDescriptor<RowData>({
    mode: 'sql',
    name: options.name,
    sourceSql: normalized.sourceSql,
    subscriptionSql: normalized.subscriptionSql,
    projection: normalized.projection,
    tableName: normalized.tableName,
    getKey: options.getKey,
    mapRow: options.mapRow,
  });
}

export function normalizeLiveSql(query: string, optionLimit?: number): NormalizedLiveSql {
  const sourceSql = normalizeSqlText(query);
  if (sourceSql.length === 0) {
    throw new LiveQueryDescriptorError('Live SQL query cannot be empty.');
  }
  if (sourceSql.includes(';')) {
    throw new LiveQueryDescriptorError('Live SQL query must contain exactly one SELECT statement without semicolons.');
  }
  if (UNSUPPORTED_LIVE_SQL.test(sourceSql)) {
    throw new LiveQueryDescriptorError(
      'Raw SQL live query mode supports SELECT ... FROM ... WHERE ... with optional ORDER BY/LIMIT only in v1.',
    );
  }

  const match = sourceSql.match(LIVE_SELECT_RE);
  if (!match) {
    throw new LiveQueryDescriptorError(
      'Raw SQL live query mode supports SELECT ... FROM ... WHERE ... with optional ORDER BY and LIMIT in v1.',
    );
  }

  const [, selectList, tableName, whereClause, orderBySql, limitSql] = match;
  const subscriptionSql = [`SELECT ${selectList.trim()} FROM ${tableName.trim()}`];
  if (whereClause) {
    subscriptionSql.push(`WHERE ${whereClause.trim()}`);
  }

  const queryLimit = limitSql ? Number.parseInt(limitSql, 10) : undefined;
  const limit = combineLimits(queryLimit, optionLimit);
  const projection: LiveProjectionPlan<RowData> = {
    ...(orderBySql ? { orderBy: parseLiveOrderBy(orderBySql) } : {}),
    ...(limit !== undefined ? { limit } : {}),
  };

  return {
    sourceSql,
    subscriptionSql: subscriptionSql.join(' '),
    tableName: unquoteTableName(tableName),
    projection,
  };
}

function normalizeSqlText(query: string): string {
  return query.trim().replace(/;\s*$/, '').replace(/\s+/g, ' ');
}

function combineLimits(queryLimit?: number, optionLimit?: number): number | undefined {
  if (queryLimit === undefined) {
    return optionLimit;
  }
  if (optionLimit === undefined) {
    return queryLimit;
  }
  return Math.min(queryLimit, optionLimit);
}

function unquoteTableName(tableName: string): string {
  return tableName
    .split('.')
    .map((part) => part.replace(/^"|"$/g, ''))
    .join('.');
}