import type { ComponentType } from "react";
import { Radio, User, Users } from "lucide-react";

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
  types?: readonly KalamDbType[];
}> = [
  { label: "(none)", value: DEFAULT_NONE },
  {
    label: "SNOWFLAKE_ID()",
    value: "SNOWFLAKE_ID()",
    description: "Auto-generated bigint id",
    types: ["BIGINT"],
  },
  {
    label: "NOW()",
    value: "NOW()",
    description: "Current timestamp",
    types: ["TIMESTAMP", "DATETIME"],
  },
  {
    label: "CURRENT_DATE",
    value: "CURRENT_DATE",
    description: "Current date",
    types: ["DATE"],
  },
  {
    label: "CURRENT_TIME",
    value: "CURRENT_TIME",
    description: "Current time",
    types: ["TIME"],
  },
  { label: "TRUE", value: "TRUE", description: "Boolean true", types: ["BOOLEAN"] },
  {
    label: "FALSE",
    value: "FALSE",
    description: "Boolean false",
    types: ["BOOLEAN"],
  },
  {
    label: "0",
    value: "0",
    description: "Zero value",
    types: ["SMALLINT", "INT", "FLOAT", "DOUBLE", "DECIMAL"],
  },
  {
    label: "ULID()",
    value: "ULID()",
    description: "ULID string",
    types: ["TEXT"],
  },
  {
    label: "UUID_V7()",
    value: "UUID_V7()",
    description: "Auto-generated UUID v7",
    types: ["TEXT", "UUID"],
  },
  {
    label: "'{}'",
    value: "'{}'",
    description: "Empty JSON object",
    types: ["JSON"],
  },
  { label: "Custom...", value: DEFAULT_CUSTOM },
];

export function defaultPresetsForType(type: string): typeof DEFAULT_PRESETS {
  const normalized = type.trim().toUpperCase() as KalamDbType;
  return DEFAULT_PRESETS.filter((preset) => {
    if (preset.value === DEFAULT_NONE || preset.value === DEFAULT_CUSTOM) {
      return true;
    }
    return preset.types?.includes(normalized) ?? false;
  });
}

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

export const TABLE_TYPE_OPTIONS: Array<{
  value: DraftTableType;
  label: string;
  description: string;
  icon: ComponentType<{ className?: string }>;
  iconClassName: string;
}> = [
  {
    value: "user",
    label: "User",
    description: "per-user rows isolated by the authenticated user.",
    icon: User,
    iconClassName: "text-emerald-400",
  },
  {
    value: "shared",
    label: "Shared",
    description: "shared rows protected by CREATE POLICY row-level security.",
    icon: Users,
    iconClassName: "text-cyan-400",
  },
  {
    value: "stream",
    label: "Stream",
    description: "append-oriented event data with retention and eviction options.",
    icon: Radio,
    iconClassName: "text-violet-400",
  },
];

export const COMPRESSION_OPTIONS = ["none", "snappy", "zstd"] as const;
export type DraftCompression = (typeof COMPRESSION_OPTIONS)[number];

export const POLICY_COMMANDS = [
  "all",
  "select",
  "insert",
  "update",
  "delete",
] as const;
export type DraftPolicyCommand = (typeof POLICY_COMMANDS)[number];

export const POLICY_TARGETS = ["public", "user", "service"] as const;
export type DraftPolicyTarget = (typeof POLICY_TARGETS)[number];

export interface DraftTablePolicy {
  id: string;
  name: string;
  command: DraftPolicyCommand;
  targets: DraftPolicyTarget[];
  usingExpr: string;
  withCheckExpr: string;
  isNew: boolean;
  isDeleted: boolean;
}

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
  policies: DraftTablePolicy[];
}

function newId(): string {
  return typeof crypto !== "undefined" && crypto.randomUUID
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2);
}

export function newDraftPolicy(): DraftTablePolicy {
  return {
    id: newId(),
    name: "",
    command: "select",
    targets: ["user", "service"],
    usingExpr: "",
    withCheckExpr: "",
    isNew: true,
    isDeleted: false,
  };
}

function normalizePolicyCommand(value: unknown): DraftPolicyCommand {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    const nested = record.command ?? record.Command;
    if (nested != null) return normalizePolicyCommand(nested);
  }
  const normalized = String(value ?? "all")
    .trim()
    .toLowerCase();
  return POLICY_COMMANDS.includes(normalized as DraftPolicyCommand)
    ? (normalized as DraftPolicyCommand)
    : "all";
}

function pushPolicyTarget(
  targets: DraftPolicyTarget[],
  candidate: string,
): void {
  if (!POLICY_TARGETS.includes(candidate as DraftPolicyTarget)) return;
  const target = candidate as DraftPolicyTarget;
  if (!targets.includes(target)) targets.push(target);
}

function parsePolicyTarget(value: unknown, targets: DraftPolicyTarget[]): void {
  if (typeof value === "string") {
    pushPolicyTarget(targets, value.trim().toLowerCase());
    return;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return;
  const record = value as Record<string, unknown>;
  if (Object.keys(record).some((key) => key.toLowerCase() === "public")) {
    pushPolicyTarget(targets, "public");
    return;
  }
  const role = record.role ?? record.Role;
  if (typeof role === "string") {
    pushPolicyTarget(targets, role.trim().toLowerCase());
  }
}

export function normalizePolicyTargets(value: unknown): DraftPolicyTarget[] {
  let parsed: unknown = value;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) return ["public"];
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      parsed = trimmed.split(",").map((part) => part.trim());
    }
  }
  const items = Array.isArray(parsed) ? parsed : parsed == null ? [] : [parsed];
  const targets: DraftPolicyTarget[] = [];
  for (const item of items) {
    parsePolicyTarget(item, targets);
  }
  if (targets.includes("public")) return ["public"];
  return targets;
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
  useUserStorage?: boolean | null;
  options?: Record<string, unknown> | null;
  columns: Array<{
    name: string;
    dataType: string;
    isNullable: boolean;
    isPrimaryKey: boolean;
  }>;
  policies?: Array<{
    name: string;
    command?: unknown;
    targets?: unknown;
    usingSql?: string | null;
    withCheckSql?: string | null;
  }>;
}): DraftTable {
  const tableType = normalizeTableType(table.tableType);
  const optionsFromJson = table.options ?? null;
  let options = defaultTableOptions(tableType);

  const storageId = normalizeStringOption(
    table.storageId ?? readOption(optionsFromJson, "storage_id", "storageId"),
  );
  if (storageId) options = { ...options, storageId };

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
  const policies: DraftTablePolicy[] =
    tableType === "shared"
      ? (table.policies ?? []).map((policy) => ({
          id: newId(),
          name: policy.name,
          command: normalizePolicyCommand(policy.command),
          targets: (() => {
            const targets = normalizePolicyTargets(policy.targets);
            return targets.length > 0 ? targets : ["public"];
          })(),
          usingExpr: policy.usingSql?.trim() ?? "",
          withCheckExpr: policy.withCheckSql?.trim() ?? "",
          isNew: false,
          isDeleted: false,
        }))
      : [];
  return {
    namespace: table.namespace,
    name: table.name,
    tableType,
    options,
    columns,
    policies,
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
    policies: [],
  };
}
