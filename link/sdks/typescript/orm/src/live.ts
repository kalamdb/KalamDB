import type { Table, InferSelectModel, SQLWrapper } from 'drizzle-orm';
import { getTableColumns, sql } from 'drizzle-orm';
import type {
  KalamDBClient,
  LiveQueryDescriptor,
  LiveOptions,
  RowData,
  Unsubscribe,
} from '@kalamdb/client';
import { createLiveQueryDescriptor, parseLiveOrderBy } from '@kalamdb/client';
import { normalizeDateValue, normalizeTemporalValue, normalizeTimeValue } from './driver.js';
import { compileInlineQuery } from './sql.js';
import { getKalamTableConfig } from './ktable.js';

type TableQueryOptions = {
  where?: SQLWrapper;
  orderBy?: SQLWrapper | SQLWrapper[];
};

type LiveTableOptions<T> = Omit<LiveOptions<T>, 'mapRow'> & TableQueryOptions;

function unwrapCellValue(cell: unknown): unknown {
  if (cell == null) return null;
  if (typeof cell === 'object' && 'toJson' in cell) {
    return (cell as { toJson: () => unknown }).toJson();
  }
  return cell;
}

export function getLiveTableName(table: Table): string {
  return getKalamTableConfig(table)?.qualifiedName ?? compileInlineQuery(sql`${table}`).sql;
}

function buildTableQuery(table: Table, where?: SQLWrapper): string {
  const statement = sql`SELECT * FROM ${table}`;
  if (where) {
    statement.append(sql` WHERE ${where}`);
  }

  return compileInlineQuery(statement).sql;
}

type ColumnNormalizer = (value: unknown) => unknown;

function normalizerForColumn(column: { dataType?: unknown; columnType?: unknown }): ColumnNormalizer | undefined {
  const dataType = String(column.dataType ?? '').toLowerCase();
  const columnType = String(column.columnType ?? '').toLowerCase();

  if (columnType.includes('timestamp')) return normalizeTemporalValue;
  if (dataType === 'date' || columnType.includes('date')) return normalizeDateValue;
  if (columnType.includes('time')) return normalizeTimeValue;
  return undefined;
}

function mapTableRow<TTable extends Table>(table: TTable, row: RowData): InferSelectModel<TTable> {
  const columns = getTableColumns(table);
  const mapped: Record<string, unknown> = {};

  for (const [key, col] of Object.entries(columns)) {
    const raw = unwrapCellValue(row[col.name]);
    const normalizer = normalizerForColumn(col);
    const driverValue = raw !== undefined && normalizer ? normalizer(raw) : raw;

    if (raw !== undefined && 'mapFromDriverValue' in col) {
      mapped[key] = (col as { mapFromDriverValue: (v: unknown) => unknown }).mapFromDriverValue(driverValue);
    } else {
      mapped[key] = driverValue ?? null;
    }
  }

  return mapped as InferSelectModel<TTable>;
}

function compileOrderBy(orderBy?: SQLWrapper | SQLWrapper[]) {
  if (!orderBy) {
    return undefined;
  }

  const expressions = Array.isArray(orderBy) ? orderBy : [orderBy];
  const compiled = expressions.flatMap((expression) => parseLiveOrderBy(compileInlineQuery(expression).sql));
  return compiled.length > 0 ? compiled : undefined;
}

function inferKeyColumns(table: Table): string[] | undefined {
  const columns = Object.values(getTableColumns(table));
  const primaryKeys = columns
    .filter((column) => Boolean((column as { primary?: boolean; primaryKey?: boolean }).primary ?? (column as { primary?: boolean; primaryKey?: boolean }).primaryKey))
    .map((column) => column.name);

  if (primaryKeys.length > 0) {
    return primaryKeys;
  }

  return columns.some((column) => column.name === 'id') ? ['id'] : undefined;
}

export function compileLiveTableDescriptor<TTable extends Table>(
  table: TTable,
  options: LiveTableOptions<InferSelectModel<TTable>> = {},
): LiveQueryDescriptor<InferSelectModel<TTable>> {
  const { where, orderBy, ...liveOptions } = options;
  const mapRow = (row: RowData): InferSelectModel<TTable> => mapTableRow(table, row);
  const subscriptionSql = buildTableQuery(table, where);
  const compiledOrderBy = compileOrderBy(orderBy);

  return createLiveQueryDescriptor<InferSelectModel<TTable>>({
    mode: 'drizzle',
    sourceSql: subscriptionSql,
    subscriptionSql,
    tableName: getLiveTableName(table),
    mapRow,
    getKey: liveOptions.getKey ?? inferKeyColumns(table),
    projection: {
      ...(compiledOrderBy ? { orderBy: compiledOrderBy } : {}),
      ...(liveOptions.limit !== undefined ? { limit: liveOptions.limit } : {}),
    },
  });
}

export function liveTable<TTable extends Table>(
  client: KalamDBClient,
  table: TTable,
  callback: (rows: InferSelectModel<TTable>[]) => void,
  options: LiveTableOptions<InferSelectModel<TTable>> = {},
): Promise<Unsubscribe> {
  const { where, orderBy: _orderBy, ...liveOptions } = options;
  const descriptor = compileLiveTableDescriptor(table, options);

  return client.connect().then(() => client.live<InferSelectModel<TTable>>(
    buildTableQuery(table, where),
    callback,
    { ...liveOptions, mapRow: descriptor.mapRow },
  ));
}
