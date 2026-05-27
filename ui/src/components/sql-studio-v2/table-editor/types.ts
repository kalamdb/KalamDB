export function isReadOnlyNamespace(name: string): boolean {
  return (
    name === "information_schema" ||
    name === "pg_catalog" ||
    name === "datafusion" ||
    name.startsWith("system") ||
    name.startsWith("dba")
  );
}

export const KALAMDB_TYPES = [
  "BOOLEAN",
  "SMALLINT",
  "INT",
  "BIGINT",
  "FLOAT",
  "DOUBLE",
  "DECIMAL",
  "TEXT",
  "TIMESTAMP",
  "DATE",
  "DATETIME",
  "TIME",
  "JSON",
  "BYTES",
  "UUID",
  "EMBEDDING",
  "FILE",
] as const;

export type KalamDbType = (typeof KALAMDB_TYPES)[number];

export const DEFAULT_NONE = "__NONE__";
export const DEFAULT_CUSTOM = "__CUSTOM__";

export const DEFAULT_PRESETS: Array<{
  label: string;
  value: string;
  description?: string;
}> = [
  { label: "(none)", value: DEFAULT_NONE },
  {
    label: "SNOWFLAKE_ID()",
    value: "SNOWFLAKE_ID()",
    description: "Auto-generated bigint id",
  },
  { label: "NOW()", value: "NOW()", description: "Current timestamp" },
  {
    label: "UUID_GENERATE_V7()",
    value: "UUID_GENERATE_V7()",
    description: "Auto-generated UUID v7",
  },
  { label: "ULID()", value: "ULID()", description: "ULID string" },
  { label: "Custom...", value: DEFAULT_CUSTOM },
];

export interface DraftColumn {
  id: string;
  name: string;
  type: string;
  isPrimaryKey: boolean;
  isNotNull: boolean;
  isUnique: boolean;
  defaultExpr: string;
  isNew: boolean;
  isDeleted: boolean;
}

export type EditorMode = "idle" | "create" | "edit";

export const TABLE_TYPES = ["user", "shared", "stream"] as const;
export type DraftTableType = (typeof TABLE_TYPES)[number];

export const COMPRESSION_OPTIONS = ["snappy", "none", "lz4", "zstd"] as const;
export type DraftCompression = (typeof COMPRESSION_OPTIONS)[number];

export const ACCESS_LEVEL_OPTIONS = [
  "private",
  "public",
  "restricted",
  "dba",
] as const;
export type DraftAccessLevel = (typeof ACCESS_LEVEL_OPTIONS)[number];

export const EVICTION_STRATEGY_OPTIONS = [
  "time_based",
  "size_based",
  "hybrid",
] as const;
export type DraftEvictionStrategy = (typeof EVICTION_STRATEGY_OPTIONS)[number];

export const FLUSH_POLICY_KINDS = [
  "none",
  "rows",
  "interval",
  "combined",
] as const;
export type DraftFlushPolicyKind = (typeof FLUSH_POLICY_KINDS)[number];

export interface DraftTableOptions {
  storageId: string;
  useUserStorage: boolean;
  flushPolicyKind: DraftFlushPolicyKind;
  flushRows: string;
  flushIntervalSeconds: string;
  accessLevel: DraftAccessLevel;
  ttlSeconds: string;
  evictionStrategy: DraftEvictionStrategy;
  maxStreamSizeBytes: string;
  compression: DraftCompression;
}

export interface DraftTable {
  namespace: string;
  name: string;
  tableType: DraftTableType;
  options: DraftTableOptions;
  columns: DraftColumn[];
}

function newId(): string {
  return typeof crypto !== "undefined" && crypto.randomUUID
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2);
}

export function newDraftColumn(): DraftColumn {
  return {
    id: newId(),
    name: "",
    type: "TEXT",
    isPrimaryKey: false,
    isNotNull: false,
    isUnique: false,
    defaultExpr: "",
    isNew: true,
    isDeleted: false,
  };
}

export function defaultTableOptions(
  tableType: DraftTableType = "user",
): DraftTableOptions {
  return {
    storageId: "local",
    useUserStorage: false,
    flushPolicyKind: "none",
    flushRows: "10000",
    flushIntervalSeconds: "300",
    accessLevel: "private",
    ttlSeconds: tableType === "stream" ? "86400" : "3600",
    evictionStrategy: "time_based",
    maxStreamSizeBytes: "0",
    compression: "snappy",
  };
}

function normalizeTableType(value: unknown): DraftTableType {
  const normalized = String(value ?? "")
    .trim()
    .toLowerCase();
  return TABLE_TYPES.includes(normalized as DraftTableType)
    ? (normalized as DraftTableType)
    : "user";
}

function normalizeStringOption(value: unknown): string | null {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return null;
}

function normalizeBooleanOption(value: unknown): boolean | null {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (["true", "1", "yes"].includes(normalized)) return true;
    if (["false", "0", "no"].includes(normalized)) return false;
  }
  return null;
}

function readOption(
  options: Record<string, unknown> | null | undefined,
  ...keys: string[]
): unknown {
  if (!options) return undefined;
  for (const key of keys) {
    if (Object.prototype.hasOwnProperty.call(options, key)) {
      return options[key];
    }
  }
  return undefined;
}

function normalizeCompression(value: unknown): DraftCompression {
  const normalized = String(value ?? "snappy")
    .trim()
    .toLowerCase();
  return COMPRESSION_OPTIONS.includes(normalized as DraftCompression)
    ? (normalized as DraftCompression)
    : "snappy";
}

function normalizeAccessLevel(value: unknown): DraftAccessLevel {
  const normalized = String(value ?? "private")
    .trim()
    .toLowerCase();
  return ACCESS_LEVEL_OPTIONS.includes(normalized as DraftAccessLevel)
    ? (normalized as DraftAccessLevel)
    : "private";
}

function normalizeEvictionStrategy(value: unknown): DraftEvictionStrategy {
  const normalized = String(value ?? "time_based")
    .trim()
    .toLowerCase();
  return EVICTION_STRATEGY_OPTIONS.includes(normalized as DraftEvictionStrategy)
    ? (normalized as DraftEvictionStrategy)
    : "time_based";
}

function applyFlushPolicyOptions(
  target: DraftTableOptions,
  value: unknown,
): DraftTableOptions {
  if (!value) return target;

  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (!normalized || normalized === "null" || normalized === "default")
      return target;
    const parts = Object.fromEntries(
      normalized.split(",").map((part) => {
        const [key, entry] = part.split(":");
        return [key?.trim(), entry?.trim()];
      }),
    );
    const rows = parts.rows;
    const interval = parts.interval;
    if (rows && interval) {
      return {
        ...target,
        flushPolicyKind: "combined",
        flushRows: rows,
        flushIntervalSeconds: interval,
      };
    }
    if (rows) return { ...target, flushPolicyKind: "rows", flushRows: rows };
    if (interval)
      return {
        ...target,
        flushPolicyKind: "interval",
        flushIntervalSeconds: interval,
      };
    return target;
  }

  if (typeof value !== "object" || Array.isArray(value)) return target;
  const record = value as Record<string, unknown>;
  const type = String(record.type ?? record.Type ?? "")
    .trim()
    .toLowerCase();
  const rows = normalizeStringOption(record.row_limit ?? record.rowLimit);
  const interval = normalizeStringOption(
    record.interval_seconds ?? record.intervalSeconds,
  );

  if (type === "combined" || (rows && interval)) {
    return {
      ...target,
      flushPolicyKind: "combined",
      flushRows: rows ?? target.flushRows,
      flushIntervalSeconds: interval ?? target.flushIntervalSeconds,
    };
  }
  if (type === "row_limit" || type === "rows" || rows) {
    return {
      ...target,
      flushPolicyKind: "rows",
      flushRows: rows ?? target.flushRows,
    };
  }
  if (type === "time_interval" || type === "interval" || interval) {
    return {
      ...target,
      flushPolicyKind: "interval",
      flushIntervalSeconds: interval ?? target.flushIntervalSeconds,
    };
  }
  return target;
}

export function tableToDraft(table: {
  namespace: string;
  name: string;
  tableType?: string;
  storageId?: string | null;
  accessLevel?: string | null;
  useUserStorage?: boolean | null;
  options?: Record<string, unknown> | null;
  columns: Array<{
    name: string;
    dataType: string;
    isNullable: boolean;
    isPrimaryKey: boolean;
  }>;
}): DraftTable {
  const tableType = normalizeTableType(table.tableType);
  const optionsFromJson = table.options ?? null;
  let options = defaultTableOptions(tableType);

  const storageId = normalizeStringOption(
    table.storageId ?? readOption(optionsFromJson, "storage_id", "storageId"),
  );
  if (storageId) options = { ...options, storageId };

  const accessLevel =
    table.accessLevel ??
    readOption(optionsFromJson, "access_level", "accessLevel");
  options = { ...options, accessLevel: normalizeAccessLevel(accessLevel) };

  const useUserStorage = normalizeBooleanOption(
    table.useUserStorage ??
      readOption(optionsFromJson, "use_user_storage", "useUserStorage"),
  );
  if (useUserStorage !== null) options = { ...options, useUserStorage };

  options = applyFlushPolicyOptions(
    options,
    readOption(optionsFromJson, "flush_policy", "flushPolicy"),
  );
  options = {
    ...options,
    ttlSeconds:
      normalizeStringOption(
        readOption(optionsFromJson, "ttl_seconds", "ttlSeconds"),
      ) ?? options.ttlSeconds,
    evictionStrategy: normalizeEvictionStrategy(
      readOption(optionsFromJson, "eviction_strategy", "evictionStrategy"),
    ),
    maxStreamSizeBytes:
      normalizeStringOption(
        readOption(
          optionsFromJson,
          "max_stream_size_bytes",
          "maxStreamSizeBytes",
        ),
      ) ?? options.maxStreamSizeBytes,
    compression: normalizeCompression(
      readOption(optionsFromJson, "compression"),
    ),
  };

  const columns: DraftColumn[] = table.columns
    .filter((c) => !c.name.startsWith("_"))
    .map((c) => ({
      id: newId(),
      name: c.name,
      type: (c.dataType ?? "TEXT").toUpperCase(),
      isPrimaryKey: c.isPrimaryKey,
      isNotNull: !c.isNullable,
      isUnique: false,
      defaultExpr: "",
      isNew: false,
      isDeleted: false,
    }));
  return {
    namespace: table.namespace,
    name: table.name,
    tableType,
    options,
    columns,
  };
}

export function emptyDraft(
  namespace = "default",
  tableType: DraftTableType = "user",
): DraftTable {
  const idCol = newDraftColumn();
  idCol.name = "id";
  idCol.type = "BIGINT";
  idCol.isPrimaryKey = true;
  idCol.isNotNull = true;
  idCol.defaultExpr = "SNOWFLAKE_ID()";
  return {
    namespace,
    name: "",
    tableType,
    options: defaultTableOptions(tableType),
    columns: [idCol],
  };
}
