import type { KalamDBClient, QueryResponse } from '@kalamdb/client';
import { parseKalamDataType, type KalamDataTypeDescriptor } from './data-types.js';
import type { KalamSystemColumnName, KalamTableType } from './ktable.js';

interface TableInfo {
  tableId: string;
  tableName: string;
  namespaceId: string;
  tableType?: KalamTableType;
  columns?: ColumnInfo[];
}

interface ColumnInfo {
  name: string;
  dataType: unknown;
  nullable: boolean;
  hasDefault?: boolean;
  isPrimaryKey?: boolean;
  ordinalPosition?: number;
  comment?: string;
}

type BigIntMode = 'string' | 'bigint' | 'number';

interface RenderContext {
  ormImports: Set<string>;
  pgImports: Set<string>;
  needsSql: boolean;
  options: GenerateOptions;
  emitUnqualifiedNames: boolean;
}

const HIDDEN_TABLES = ['system.live', 'system.server_logs', 'system.cluster', 'system.settings', 'system.stats'];

const RESERVED_IDENTIFIERS = new Set([
  'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'debugger', 'default', 'delete',
  'do', 'else', 'enum', 'export', 'extends', 'false', 'finally', 'for', 'function', 'if', 'import',
  'in', 'instanceof', 'new', 'null', 'return', 'super', 'switch', 'this', 'throw', 'true', 'try',
  'typeof', 'var', 'void', 'while', 'with', 'yield', 'let', 'static', 'implements', 'interface',
  'package', 'private', 'protected', 'public',
]);

function parseBoolean(value: unknown): boolean | undefined {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return value !== 0;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (['true', 't', 'yes', 'y', '1'].includes(normalized)) return true;
    if (['false', 'f', 'no', 'n', '0'].includes(normalized)) return false;
  }
  return undefined;
}

function parseInteger(value: unknown): number | undefined {
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isInteger(parsed) ? parsed : undefined;
}

function optionalString(value: unknown): string | undefined {
  if (value === null || value === undefined) return undefined;
  const text = String(value);
  return text.length > 0 ? text : undefined;
}

function parseNullable(value: unknown, fallback = true): boolean {
  return parseBoolean(value) ?? fallback;
}

function hasDefault(value: unknown): boolean | undefined {
  if (value === undefined) return undefined;
  if (value === null) return false;
  if (typeof value === 'string') {
    const normalized = value.trim();
    return normalized.length > 0 && normalized.toLowerCase() !== 'none' && normalized.toLowerCase() !== 'null';
  }
  if (typeof value !== 'object') return Boolean(value);

  const obj = value as Record<string, unknown>;
  return Object.keys(obj).length > 0 && !('None' in obj);
}

function normalizeTableType(value: unknown): KalamTableType | undefined {
  const normalized = String(value ?? '').trim().toLowerCase();
  if (normalized === 'shared' || normalized === 'user' || normalized === 'stream' || normalized === 'system') {
    return normalized;
  }

  return undefined;
}

function parseColumn(rawColumn: Record<string, unknown>): ColumnInfo | null {
  const name = optionalString(rawColumn.column_name ?? rawColumn.name);
  if (!name || name.startsWith('_')) return null;

  const defaultValue = rawColumn.default_value ?? rawColumn.column_default;
  return {
    name,
    dataType: rawColumn.data_type ?? rawColumn.dataType ?? rawColumn.type ?? 'Text',
    nullable: parseNullable(rawColumn.is_nullable ?? rawColumn.nullable),
    hasDefault: hasDefault(defaultValue),
    isPrimaryKey: parseBoolean(rawColumn.is_primary_key ?? rawColumn.primary_key),
    ordinalPosition: parseInteger(rawColumn.ordinal_position),
    comment: optionalString(rawColumn.column_comment ?? rawColumn.comment),
  };
}

function parseColumnsJson(columnsJson: string): ColumnInfo[] {
  try {
    const cols = JSON.parse(columnsJson) as Record<string, unknown>[];
    return cols.map(parseColumn).filter((col): col is ColumnInfo => Boolean(col));
  } catch {
    return [];
  }
}

async function fetchTables(client: KalamDBClient): Promise<TableInfo[]> {
  const response: QueryResponse = await client.query('SHOW TABLES');
  const rows = (response.results?.[0]?.named_rows ?? []) as Record<string, unknown>[];
  return rows.map((row) => {
    const namespaceId = String(row.namespace_id ?? row.namespace ?? 'default');
    const tableName = String(row.table_name ?? row.name ?? '');
    const tableId = String(row.table_id ?? `${namespaceId}:${tableName}`);
    return {
      tableId,
      tableName,
      namespaceId,
      tableType: normalizeTableType(row.table_type),
      columns: row.columns ? parseColumnsJson(String(row.columns)) : undefined,
    };
  }).filter((table) => table.tableName.length > 0);
}

async function fetchColumns(client: KalamDBClient, tableId: string): Promise<ColumnInfo[]> {
  const qualifiedName = tableId.replace(':', '.');
  const response: QueryResponse = await client.query(`DESCRIBE ${qualifiedName}`);
  const rows = (response.results?.[0]?.named_rows ?? []) as Record<string, unknown>[];
  return rows.map(parseColumn).filter((col): col is ColumnInfo => Boolean(col));
}

function needsDescribe(columns: ColumnInfo[] | undefined): boolean {
  return !columns || columns.length === 0 || columns.some((column) => (
    column.hasDefault === undefined || column.isPrimaryKey === undefined || column.ordinalPosition === undefined
  ));
}

async function resolveColumns(client: KalamDBClient, table: TableInfo): Promise<ColumnInfo[]> {
  if (!needsDescribe(table.columns)) {
    return sortColumns(table.columns ?? []);
  }

  try {
    const described = await fetchColumns(client, table.tableId);
    if (described.length > 0) return sortColumns(described);
  } catch {
    // Fall back to SHOW TABLES metadata on older servers that cannot DESCRIBE a virtual table.
  }

  return sortColumns(table.columns ?? []);
}

function sortColumns(columns: ColumnInfo[]): ColumnInfo[] {
  return [...columns].sort((left, right) => (
    (left.ordinalPosition ?? Number.MAX_SAFE_INTEGER) - (right.ordinalPosition ?? Number.MAX_SAFE_INTEGER)
      || left.name.localeCompare(right.name)
  ));
}

function toIdentifier(raw: string, fallback: string): string {
  let identifier = raw.replace(/[^A-Za-z0-9_$]+/g, '_').replace(/^_+|_+$/g, '');
  identifier = identifier.replace(/_+/g, '_');
  if (!identifier) identifier = fallback;
  if (/^[0-9]/.test(identifier)) identifier = `_${identifier}`;
  if (RESERVED_IDENTIFIERS.has(identifier)) identifier = `_${identifier}`;
  return identifier;
}

function toVariableName(namespaceId: string, tableName: string, emitUnqualifiedNames: boolean): string {
  return emitUnqualifiedNames
    ? toIdentifier(tableName, 'table')
    : toIdentifier(`${namespaceId}_${tableName}`, 'table');
}

function uniqueIdentifier(base: string, used: Map<string, number>): string {
  const count = used.get(base) ?? 0;
  used.set(base, count + 1);
  return count === 0 ? base : `${base}_${count + 1}`;
}

function tableVariableNames(tables: TableInfo[], emitUnqualifiedNames: boolean): Map<string, string> {
  const used = new Map<string, number>();
  const names = new Map<string, string>();
  for (const table of tables) {
    names.set(
      table.tableId,
      uniqueIdentifier(toVariableName(table.namespaceId, table.tableName, emitUnqualifiedNames), used),
    );
  }
  return names;
}

function toPascalCase(identifier: string): string {
  return identifier
    .split(/_+/)
    .filter(Boolean)
    .map((part) => `${part[0]?.toUpperCase() ?? ''}${part.slice(1)}`)
    .join('') || 'Table';
}

function renderPropertyName(name: string): string {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name) ? name : JSON.stringify(name);
}

function renderString(value: string): string {
  return JSON.stringify(value);
}

function renderSystemColumnsOption(value: GenerateOptions['includeSystemColumns']): string | undefined {
  if (!value) return undefined;
  if (value === true || value === 'all') return `systemColumns: ${JSON.stringify(value)}`;
  return `systemColumns: ${JSON.stringify(value)}`;
}

function resolveGeneratedSystemColumns(
  tableType: KalamTableType | undefined,
  value: GenerateOptions['includeSystemColumns'],
): readonly KalamSystemColumnName[] {
  if (!value) return [];
  if (Array.isArray(value)) return value;
  if (value === 'all') return ['_seq', '_deleted', '_commit_seq'];
  if (tableType === 'stream') return ['_seq'];
  if (tableType === 'system') return [];
  return ['_seq', '_deleted'];
}

function renderSystemColumnsSpread(columns: readonly KalamSystemColumnName[]): string | undefined {
  if (columns.length === 0) return undefined;
  return `  ...kSystemColumns(${JSON.stringify(columns)} as const),`;
}

function addPgImport(ctx: RenderContext, name: string): void {
  ctx.pgImports.add(name);
}

function addOrmImport(ctx: RenderContext, name: string): void {
  ctx.ormImports.add(name);
}

function renderBigIntColumn(columnName: string, ctx: RenderContext): string {
  const mode: BigIntMode = ctx.options.bigIntMode ?? 'string';
  if (mode === 'string') {
    addPgImport(ctx, 'text');
    return `text(${renderString(columnName)})`;
  }

  addPgImport(ctx, 'bigint');
  return `bigint(${renderString(columnName)}, { mode: ${renderString(mode)} })`;
}

function renderColumnType(columnName: string, type: KalamDataTypeDescriptor, ctx: RenderContext): string {
  switch (type.kind) {
    case 'Boolean':
      addPgImport(ctx, 'boolean');
      return `boolean(${renderString(columnName)})`;
    case 'Int':
      addPgImport(ctx, 'integer');
      return `integer(${renderString(columnName)})`;
    case 'SmallInt':
      addPgImport(ctx, 'smallint');
      return `smallint(${renderString(columnName)})`;
    case 'BigInt':
      return renderBigIntColumn(columnName, ctx);
    case 'Double':
      addPgImport(ctx, 'doublePrecision');
      return `doublePrecision(${renderString(columnName)})`;
    case 'Float':
      addPgImport(ctx, 'real');
      return `real(${renderString(columnName)})`;
    case 'Timestamp':
    case 'DateTime':
      addPgImport(ctx, 'timestamp');
      return `timestamp(${renderString(columnName)}, { mode: 'date' })`;
    case 'Date':
      addPgImport(ctx, 'date');
      return `date(${renderString(columnName)}, { mode: 'date' })`;
    case 'Time':
      addPgImport(ctx, 'time');
      return `time(${renderString(columnName)})`;
    case 'Json':
      addPgImport(ctx, 'jsonb');
      return `jsonb(${renderString(columnName)})`;
    case 'Bytes':
      addOrmImport(ctx, 'bytes');
      return `bytes(${renderString(columnName)})`;
    case 'Embedding':
      addOrmImport(ctx, 'embedding');
      return `embedding(${renderString(columnName)}, ${type.dimension ?? 1536})`;
    case 'Uuid':
      addPgImport(ctx, 'uuid');
      return `uuid(${renderString(columnName)})`;
    case 'Decimal':
      addPgImport(ctx, 'numeric');
      return `numeric(${renderString(columnName)}, { precision: ${type.precision ?? 38}, scale: ${type.scale ?? 10} })`;
    case 'File':
      addOrmImport(ctx, 'file');
      return `file(${renderString(columnName)})`;
    case 'Text':
    default:
      addPgImport(ctx, 'text');
      return `text(${renderString(columnName)})`;
  }
}

function renderColumnDefinition(column: ColumnInfo, ctx: RenderContext): string[] {
  const type = parseKalamDataType(column.dataType);
  let definition = renderColumnType(column.name, type, ctx);

  if (column.hasDefault) {
    ctx.needsSql = true;
    definition += '.default(sql``)';
  }
  if (column.isPrimaryKey) {
    definition += '.primaryKey()';
  } else if (!column.nullable) {
    definition += '.notNull()';
  }

  const lines: string[] = [];
  if (column.comment) {
    lines.push(`  /** ${column.comment.replace(/\*\//g, '* /')} */`);
  }
  lines.push(`  ${renderPropertyName(column.name)}: ${definition},`);
  return lines;
}

function generateTableDefinition(
  table: TableInfo,
  columns: ColumnInfo[],
  varName: string,
  ctx: RenderContext,
): string {
  const generatedTableName = ctx.emitUnqualifiedNames
    ? table.tableName
    : `${table.namespaceId}.${table.tableName}`;
  const factoryName = table.tableType ? `kTable.${table.tableType}` : 'kTable';
  const tableOptions = renderSystemColumnsOption(ctx.options.includeSystemColumns);
  const systemColumnsSpread = renderSystemColumnsSpread(
    resolveGeneratedSystemColumns(table.tableType, ctx.options.includeSystemColumns),
  );
  const lines: string[] = [];

  lines.push(`export const ${varName} = ${factoryName}(${renderString(generatedTableName)}, {`);
  if (systemColumnsSpread) {
    addOrmImport(ctx, 'kSystemColumns');
    lines.push(systemColumnsSpread);
  }

  for (const column of columns) {
    lines.push(...renderColumnDefinition(column, ctx));
  }

  lines.push(tableOptions ? `}, { ${tableOptions} });` : '});');
  addOrmImport(ctx, 'getKalamTableConfig');
  lines.push(`export const ${varName}Config = getKalamTableConfig(${varName})!;`);

  if (ctx.options.includeTypeAliases !== false) {
    const alias = toPascalCase(varName);
    lines.push(`export type ${alias} = typeof ${varName}.$inferSelect;`);
    lines.push(`export type New${alias} = typeof ${varName}.$inferInsert;`);
  }

  return lines.join('\n');
}

function renderImports(ctx: RenderContext): string {
  const lines = [
    '// Generated by @kalamdb/orm. Regenerate with the kalamdb-orm CLI after schema changes.',
    `import { ${Array.from(ctx.ormImports).sort().join(', ')} } from '@kalamdb/orm';`,
  ];

  if (ctx.pgImports.size > 0) {
    lines.push(`import { ${Array.from(ctx.pgImports).sort().join(', ')} } from 'drizzle-orm/pg-core';`);
  }
  if (ctx.needsSql) {
    lines.push("import { sql } from 'drizzle-orm';");
  }

  return lines.join('\n');
}

export interface GenerateOptions {
  includeSystem?: boolean;
  namespaces?: string[];
  includeSystemColumns?: boolean | 'all' | readonly KalamSystemColumnName[];
  /**
   * KalamDB serializes Int64 values as strings to preserve precision. The
   * default keeps generated BIGINT columns as `text()`. Choose `bigint` or
   * `number` when your app wants Drizzle to coerce those values on read.
   */
  bigIntMode?: BigIntMode;
  /** Emit `$inferSelect` / `$inferInsert` aliases next to every table. */
  includeTypeAliases?: boolean;
  /**
   * Emit short names like `messages` / `"messages"` when a single `--namespace`
   * is selected. Default is always namespaced (`chat_messages` /
   * `"chat.messages"`) so generated files stay stable for agents and do not
   * require `configureKalamOrm`.
   */
  unqualifiedNames?: boolean;
}

export async function generateSchema(
  client: KalamDBClient,
  options: GenerateOptions = {},
): Promise<string> {
  const tables = await fetchTables(client);
  const namespaceAllowlist = new Set(
    (options.namespaces ?? []).map((namespace) => namespace.trim()).filter((namespace) => namespace.length > 0),
  );

  for (const qualifiedName of HIDDEN_TABLES) {
    const [namespace, table] = qualifiedName.split('.');
    const tableId = `${namespace}:${table}`;
    if (tables.some((existing) => existing.tableId === tableId)) continue;
    try {
      const columns = await fetchColumns(client, tableId);
      if (columns.length > 0) {
        tables.push({ tableId, tableName: table, namespaceId: namespace, tableType: 'system', columns });
      }
    } catch {
      // Hidden system tables are best-effort; older servers may not expose all of them.
    }
  }

  const deduped = Array.from(new Map(tables.map((table) => [table.tableId, table])).values());
  const filtered = deduped.filter((table) => {
    if (namespaceAllowlist.size > 0) return namespaceAllowlist.has(table.namespaceId);
    if (!options.includeSystem && (table.namespaceId === 'system' || table.namespaceId === 'dba')) return false;
    return true;
  });
  const emitUnqualifiedNames = options.unqualifiedNames === true && namespaceAllowlist.size === 1;
  const names = tableVariableNames(filtered, emitUnqualifiedNames);
  const ctx: RenderContext = {
    ormImports: new Set(['kTable']),
    pgImports: new Set(),
    needsSql: false,
    options,
    emitUnqualifiedNames,
  };

  const definitions: string[] = [];
  for (const table of filtered) {
    const columns = await resolveColumns(client, table);
    definitions.push(generateTableDefinition(
      table,
      columns,
      names.get(table.tableId) ?? toVariableName(table.namespaceId, table.tableName, emitUnqualifiedNames),
      ctx,
    ));
  }

  return `${renderImports(ctx)}\n\n${definitions.join('\n\n')}\n`;
}