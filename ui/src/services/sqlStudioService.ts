import { executeQuery } from "@/lib/kalam-client";
import { getDb } from "@/lib/db";
import type { SystemNamespaceRow, SystemSchemaRow } from "@/lib/models";
import { system_namespaces, system_schemas } from "@/lib/schema";
import type { SchemaField } from "@kalamdb/client";
import type {
  QueryLogEntry,
  QueryResultData,
  QueryResultSchemaField,
  StudioNamespace,
  StudioTable,
} from "@/components/sql-studio-v2/shared/types";
import { eq } from "drizzle-orm";

const MAX_SQL_STUDIO_RENDER_ROWS = 1000;

interface RawSqlStatementResult {
  schema?: SchemaField[];
  rows?: unknown[][];
  /** Pre-computed named rows from Rust WASM (schema → map transformation). */
  named_rows?: Record<string, unknown>[];
  row_count?: number;
  message?: string;
  as_user?: string;
}

export type RawQuerySchemaField = SchemaField;

function formatSchemaDataType(dataType: SchemaField["data_type"]): string {
  if (typeof dataType === "string") {
    return dataType;
  }

  if (dataType && typeof dataType === "object") {
    const entries = Object.entries(dataType as Record<string, unknown>);
    const [variant, value] = entries[0] ?? [];
    if (!variant) {
      return "Unknown";
    }

    if (typeof value === "number" || typeof value === "string") {
      return `${variant}(${value})`;
    }

    return variant;
  }

  return "Unknown";
}

type FieldFlags = Array<"pk" | "nn" | "uq">;

function isPrimaryKeyFlag(flags: FieldFlags | undefined): boolean {
  if (!flags || flags.length === 0) {
    return false;
  }

  return flags.includes("pk");
}

export function normalizeSchema(
  rawSchema: SchemaField[] | undefined,
): QueryResultSchemaField[] {
  if (!rawSchema) {
    return [];
  }

  return rawSchema.map((field) => ({
    name: field.name,
    dataType: formatSchemaDataType(field.data_type),
    index: field.index,
    flags: field.flags,
    isPrimaryKey: isPrimaryKeyFlag(field.flags),
  }));
}

function rowsToObjects(
  schema: QueryResultSchemaField[],
  rows: unknown[][] | undefined,
  namedRows?: Record<string, unknown>[],
): Record<string, unknown>[] {
  const normalizeValue = (value: unknown): unknown => {
    if (Array.isArray(value)) {
      return value.map((entry) => normalizeValue(entry));
    }

    if (value && typeof value === "object") {
      const maybeSerializable = value as { toJson?: () => unknown };
      if (typeof maybeSerializable.toJson === "function") {
        try {
          return normalizeValue(maybeSerializable.toJson());
        } catch {
          // Fall through to entry-wise normalization.
        }
      }

      return Object.fromEntries(
        Object.entries(value as Record<string, unknown>).map(([key, entry]) => [
          key,
          normalizeValue(entry),
        ]),
      );
    }

    return value;
  };

  // Prefer named_rows: Rust WASM pre-computes the schema→map transformation.
  if (namedRows && namedRows.length > 0) {
    return namedRows.slice(0, MAX_SQL_STUDIO_RENDER_ROWS).map((row) => {
      const item: Record<string, unknown> = {};
      for (const key of Object.keys(row)) {
        item[key] = normalizeValue(row[key] ?? null);
      }
      return item;
    });
  }

  // Fallback: positional rows + schema (older server versions)
  if (!rows || schema.length === 0) {
    return [];
  }

  const rowsToRender = rows.slice(0, MAX_SQL_STUDIO_RENDER_ROWS);
  return rowsToRender.map((row) => {
    const item: Record<string, unknown> = {};
    schema.forEach((field) => {
      item[field.name] = normalizeValue(row[field.index] ?? null);
    });
    return item;
  });
}

function toQueryLogEntry(
  result: RawSqlStatementResult,
  statementIndex: number,
  createdAt: string,
): QueryLogEntry {
  const rowCount = typeof result.row_count === "number" ? result.row_count : 0;
  const explicitMessage =
    typeof result.message === "string" ? result.message.trim() : "";

  let message = explicitMessage;
  if (!message) {
    if (Array.isArray(result.schema) && result.schema.length > 0) {
      message = `Statement ${statementIndex + 1} returned ${rowCount} row${rowCount === 1 ? "" : "s"}.`;
    } else {
      message = `Statement ${statementIndex + 1} executed successfully.`;
    }
  }

  const asUser =
    typeof result.as_user === "string" ? result.as_user : undefined;

  return {
    id: `${createdAt}-stmt-${statementIndex}`,
    level: "info",
    message,
    response: result,
    asUser,
    rowCount,
    statementIndex,
    createdAt,
  };
}

function buildQueryLogs(
  statementResults: RawSqlStatementResult[] | undefined,
): QueryLogEntry[] {
  if (!statementResults || statementResults.length === 0) {
    return [];
  }

  const createdAt = new Date().toISOString();
  return statementResults.map((result, index) =>
    toQueryLogEntry(result, index, createdAt),
  );
}

function hasTabularPayload(result: RawSqlStatementResult): boolean {
  return (
    Array.isArray(result.schema) &&
    result.schema.length > 0 &&
    (Array.isArray(result.named_rows) || Array.isArray(result.rows))
  );
}

function ensureNamespace(
  namespaces: Map<string, StudioNamespace>,
  databaseName: string,
  namespaceName: string,
): StudioNamespace {
  let namespace = namespaces.get(namespaceName);
  if (!namespace) {
    namespace = {
      database: databaseName,
      name: namespaceName,
      tables: [],
    };
    namespaces.set(namespaceName, namespace);
  }
  return namespace;
}

function ensureTable(
  namespaces: Map<string, StudioNamespace>,
  tableMap: Map<string, StudioTable>,
  databaseName: string,
  namespaceName: string,
  tableName: string,
  tableType: string,
): StudioTable {
  const tableKey = `${namespaceName}.${tableName}`;
  let table = tableMap.get(tableKey);

  if (!table) {
    table = {
      database: databaseName,
      namespace: namespaceName,
      name: tableName,
      tableType,
      columns: [],
      storageId: null,
      accessLevel: null,
      useUserStorage: null,
      version: null,
      options: null,
      comment: null,
      updatedAt: null,
      createdAt: null,
    };
    tableMap.set(tableKey, table);
    ensureNamespace(namespaces, databaseName, namespaceName).tables.push(table);
  }

  return table;
}

function normalizeNullable(value: unknown): boolean {
  if (typeof value === "boolean") {
    return value;
  }

  if (typeof value === "number") {
    return value !== 0;
  }

  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "yes" || normalized === "true" || normalized === "1") {
      return true;
    }
    if (normalized === "no" || normalized === "false" || normalized === "0") {
      return false;
    }
  }

  return Boolean(value);
}

function normalizeTextValue(value: unknown): string | null {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }

  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }

  return null;
}

function normalizeNumericValue(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }

  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) {
      return null;
    }

    const parsed = Number(trimmed);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

function normalizeBooleanValue(value: unknown): boolean | null {
  if (value == null) {
    return null;
  }

  if (typeof value === "boolean") {
    return value;
  }

  if (typeof value === "number") {
    return value !== 0;
  }

  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "true" || normalized === "1" || normalized === "yes") {
      return true;
    }
    if (normalized === "false" || normalized === "0" || normalized === "no") {
      return false;
    }
  }

  return null;
}

function normalizeStructuredValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => normalizeStructuredValue(entry));
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, entry]) => [
        key,
        normalizeStructuredValue(entry),
      ]),
    );
  }

  return value;
}

function normalizeTableOptions(value: unknown): Record<string, unknown> | null {
  if (value == null) {
    return null;
  }

  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) {
      return null;
    }

    try {
      return normalizeTableOptions(JSON.parse(trimmed) as unknown);
    } catch {
      return { value: trimmed };
    }
  }

  if (typeof value !== "object" || Array.isArray(value)) {
    return { value: normalizeStructuredValue(value) };
  }

  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([key, entry]) => [
      key,
      normalizeStructuredValue(entry),
    ]),
  );
}

function normalizeTimestampValue(value: unknown): string | number | null {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }

  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }

  if (value instanceof Date && !Number.isNaN(value.getTime())) {
    return value.toISOString();
  }

  return null;
}

function applyTableMetadata(
  table: StudioTable,
  row: Partial<
    Pick<
      SystemSchemaRow,
      | "storage_id"
      | "access_level"
      | "use_user_storage"
      | "schema_version"
      | "options"
      | "table_comment"
      | "updated_at"
      | "created_at"
    >
  >,
): void {
  table.storageId = normalizeTextValue(row.storage_id);
  table.accessLevel = normalizeTextValue(row.access_level);
  table.useUserStorage = normalizeBooleanValue(row.use_user_storage);
  table.version = normalizeNumericValue(row.schema_version);
  table.options = normalizeTableOptions(row.options);
  table.comment = normalizeTextValue(row.table_comment);
  table.updatedAt = normalizeTimestampValue(row.updated_at);
  table.createdAt = normalizeTimestampValue(row.created_at);
}

function normalizeSchemaColumns(
  value: SystemSchemaRow["columns"],
): StudioTable["columns"] {
  const parsedValue =
    typeof value === "string"
      ? (() => {
          try {
            return JSON.parse(value) as unknown;
          } catch {
            return null;
          }
        })()
      : value;

  if (!Array.isArray(parsedValue)) {
    return [];
  }

  return parsedValue
    .map((item, index) => {
      if (!item || typeof item !== "object") {
        return null;
      }

      const record = item as Record<string, unknown>;
      const name = normalizeTextValue(record.column_name ?? record.name);
      if (!name) {
        return null;
      }

      return {
        name,
        dataType: normalizeTextValue(record.data_type) ?? "unknown",
        isNullable: normalizeNullable(record.is_nullable ?? record.nullable),
        isPrimaryKey: Boolean(record.is_primary_key ?? record.primary_key),
        ordinal:
          normalizeNumericValue(record.ordinal_position ?? record.ordinal) ??
          index + 1,
      };
    })
    .filter(
      (column): column is StudioTable["columns"][number] => column !== null,
    )
    .sort((left, right) => left.ordinal - right.ordinal);
}

function inferExplorerTableType(
  namespaceName: SystemNamespaceRow["namespace_id"],
  tableType: SystemSchemaRow["table_type"],
): string {
  const normalizedNamespace = namespaceName.trim().toLowerCase();
  if (
    normalizedNamespace === "system" ||
    normalizedNamespace === "information_schema" ||
    normalizedNamespace === "pg_catalog" ||
    normalizedNamespace === "datafusion"
  ) {
    return "system";
  }

  const normalizedTableType = String(tableType ?? "")
    .trim()
    .toLowerCase();
  if (
    normalizedTableType === "stream" ||
    normalizedTableType === "shared" ||
    normalizedTableType === "user"
  ) {
    return normalizedTableType;
  }
  if (normalizedTableType.includes("view")) {
    return "view";
  }

  return "user";
}

export async function fetchSqlStudioSchemaTree(): Promise<StudioNamespace[]> {
  const databaseName = "database";
  const db = getDb();
  const [namespaceRows, schemaRows] = await Promise.all([
    db.select().from(system_namespaces),
    db.select().from(system_schemas).where(eq(system_schemas.is_latest, true)),
  ]);

  const namespaces = new Map<string, StudioNamespace>();

  namespaceRows.forEach((row) => {
    const namespaceName = String(row.namespace_id ?? "");
    if (!namespaceName) {
      return;
    }

    ensureNamespace(namespaces, databaseName, namespaceName);
  });

  const tableMap = new Map<string, StudioTable>();

  schemaRows.forEach((row) => {
    const namespaceName = String(row.namespace_id ?? "");
    const tableName = String(row.table_name ?? "");
    if (!namespaceName || !tableName) {
      return;
    }

    const table = ensureTable(
      namespaces,
      tableMap,
      databaseName,
      namespaceName,
      tableName,
      inferExplorerTableType(namespaceName, row.table_type),
    );

    applyTableMetadata(table, row);
    table.columns = normalizeSchemaColumns(row.columns);
  });

  const sortedNamespaces = Array.from(namespaces.values())
    .map((namespace) => ({
      ...namespace,
      tables: namespace.tables
        .map((table) => ({
          ...table,
          columns: [...table.columns].sort(
            (left, right) => left.ordinal - right.ordinal,
          ),
        }))
        .sort((left, right) => left.name.localeCompare(right.name)),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));

  return sortedNamespaces;
}

export async function executeSqlStudioQuery(
  sql: string,
): Promise<QueryResultData> {
  const response = await executeQuery(sql);

  if (response.status === "error" && response.error) {
    const createdAt = new Date().toISOString();
    return {
      status: "error",
      rows: [],
      schema: [],
      tookMs: response.took ?? 0,
      rowCount: 0,
      logs: [
        {
          id: `${createdAt}-error`,
          level: "error",
          message: response.error.message,
          response: response.error,
          createdAt,
        },
      ],
      errorMessage: response.error.message,
    };
  }

  const statementResults = (response.results ?? []) as RawSqlStatementResult[];
  const tabularResult = statementResults.find(hasTabularPayload);
  const firstResult = statementResults[0];

  const schema = normalizeSchema(tabularResult?.schema);
  const rows = rowsToObjects(
    schema,
    tabularResult?.rows,
    tabularResult?.named_rows,
  );
  const logs = buildQueryLogs(statementResults);

  return {
    status: "success",
    rows,
    schema,
    tookMs: response.took ?? 0,
    rowCount:
      typeof tabularResult?.row_count === "number"
        ? tabularResult.row_count
        : rows.length,
    logs,
    message: firstResult?.message,
  };
}
