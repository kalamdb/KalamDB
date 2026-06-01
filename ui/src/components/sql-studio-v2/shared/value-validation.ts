export type FieldKind =
  | "boolean"
  | "smallint"
  | "int"
  | "bigint"
  | "float"
  | "decimal"
  | "datetime"
  | "date"
  | "time"
  | "json"
  | "uuid"
  | "embedding"
  | "bytes"
  | "text";

const SMALLINT_MIN = -32_768;
const SMALLINT_MAX = 32_767;
const INT_MIN = -2_147_483_648;
const INT_MAX = 2_147_483_647;
const BIGINT_MIN = -9_223_372_036_854_775_808n;
const BIGINT_MAX = 9_223_372_036_854_775_807n;

export function classifyFieldKind(dataType: string): FieldKind {
  const t = dataType.trim().toUpperCase();
  if (t === "BOOLEAN" || t === "BOOL") return "boolean";
  if (t === "SMALLINT") return "smallint";
  if (t === "INT" || t === "INTEGER") return "int";
  if (t === "BIGINT") return "bigint";
  if (t === "FLOAT" || t === "DOUBLE" || t === "REAL") return "float";
  if (t.startsWith("DECIMAL") || t.startsWith("NUMERIC")) return "decimal";
  if (t === "TIMESTAMP" || t === "DATETIME") return "datetime";
  if (t === "DATE") return "date";
  if (t === "TIME") return "time";
  if (t === "JSON") return "json";
  if (t === "UUID") return "uuid";
  if (t.startsWith("EMBEDDING")) return "embedding";
  if (t === "BYTES") return "bytes";
  return "text";
}

export interface CoerceResult {
  value: unknown;
  error: string | null;
}

const FLOAT_REGEX = /^-?(\d+\.?\d*|\.\d+)([eE][+\-]?\d+)?$/;
const INT_REGEX = /^-?\d+$/;
const UUID_REGEX = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const DATETIME_LOCAL_REGEX = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/;

function normalizeDatetimeLocalForSql(value: string): string | null {
  const match = DATETIME_LOCAL_REGEX.exec(value);
  if (!match) {
    return null;
  }

  const [, year, month, day, hour, minute, second] = match;
  const normalizedSecond = second ?? "00";
  return `${year}-${month}-${day} ${hour}:${minute}:${normalizedSecond}`;
}

export function coerceFieldValue(raw: string, kind: FieldKind): CoerceResult {
  const trimmed = raw.trim();
  if (trimmed === "") return { value: undefined, error: null };

  switch (kind) {
    case "boolean": {
      const low = trimmed.toLowerCase();
      if (low === "true" || low === "1") return { value: true, error: null };
      if (low === "false" || low === "0") return { value: false, error: null };
      return { value: raw, error: "Must be true or false" };
    }
    case "smallint":
    case "int": {
      if (!INT_REGEX.test(trimmed)) return { value: raw, error: "Must be a whole number" };
      const n = Number(trimmed);
      if (!Number.isFinite(n)) return { value: raw, error: "Out of range" };
      const [min, max] = kind === "smallint" ? [SMALLINT_MIN, SMALLINT_MAX] : [INT_MIN, INT_MAX];
      if (n < min || n > max) {
        return { value: raw, error: `Out of range for ${kind.toUpperCase()} (${min} to ${max})` };
      }
      return { value: n, error: null };
    }
    case "bigint": {
      if (!INT_REGEX.test(trimmed)) return { value: raw, error: "Must be a whole number" };
      let big: bigint;
      try {
        big = BigInt(trimmed);
      } catch {
        return { value: raw, error: "Invalid bigint" };
      }
      if (big < BIGINT_MIN || big > BIGINT_MAX) {
        return { value: raw, error: "Out of range for BIGINT" };
      }
      return { value: big, error: null };
    }
    case "float": {
      if (!FLOAT_REGEX.test(trimmed)) return { value: raw, error: "Must be a number" };
      const n = Number(trimmed);
      if (!Number.isFinite(n)) return { value: raw, error: "Out of range" };
      return { value: n, error: null };
    }
    case "decimal": {
      if (!FLOAT_REGEX.test(trimmed)) return { value: raw, error: "Must be a decimal number" };
      const n = Number(trimmed);
      if (!Number.isFinite(n)) return { value: raw, error: "Out of range" };
      return { value: n, error: null };
    }
    case "datetime":
      {
        const normalized = normalizeDatetimeLocalForSql(trimmed);
        if (!normalized) {
          return { value: raw, error: "Must be a valid datetime" };
        }
        return { value: normalized, error: null };
      }
    case "date":
    case "time": {
      return { value: trimmed, error: null };
    }
    case "json": {
      try {
        JSON.parse(trimmed);
        return { value: trimmed, error: null };
      } catch {
        return { value: raw, error: "Invalid JSON" };
      }
    }
    case "uuid": {
      if (!UUID_REGEX.test(trimmed)) {
        return { value: raw, error: "Must be a valid UUID (e.g. 550e8400-e29b-41d4-a716-446655440000)" };
      }
      return { value: trimmed, error: null };
    }
    case "embedding": {
      try {
        const parsed = JSON.parse(trimmed);
        if (!Array.isArray(parsed)) return { value: raw, error: "Must be a JSON array of numbers" };
        if (!parsed.every((x) => typeof x === "number" && Number.isFinite(x))) {
          return { value: raw, error: "All elements must be finite numbers" };
        }
        return { value: trimmed, error: null };
      } catch {
        return { value: raw, error: "Invalid JSON array" };
      }
    }
    case "bytes": {
      return { value: trimmed, error: null };
    }
    case "text":
    default:
      return { value: raw, error: null };
  }
}
