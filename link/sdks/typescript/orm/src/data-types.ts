import type { KalamDataType } from '@kalamdb/client';

type KalamParameterizedDataType = Exclude<KalamDataType, string>;

type KalamParameterizedDataTypeKind<T> =
  T extends { Embedding: number }
    ? 'Embedding'
    : T extends { Decimal: { precision: number; scale: number } }
      ? 'Decimal'
      : never;

/**
 * ORM-facing discriminator derived from the client SDK's canonical
 * `KalamDataType`, which in turn tracks the Rust/WASM schema model.
 */
export type KalamDataTypeKind =
  | Extract<KalamDataType, string>
  | KalamParameterizedDataTypeKind<KalamParameterizedDataType>;

export interface KalamDataTypeDescriptor {
  kind: KalamDataTypeKind;
  sqlName: string;
  dimension?: number;
  precision?: number;
  scale?: number;
}

const SIMPLE_TYPE_ALIASES: Record<string, KalamDataTypeKind> = {
  bool: 'Boolean',
  boolean: 'Boolean',
  int: 'Int',
  int32: 'Int',
  integer: 'Int',
  smallint: 'SmallInt',
  int16: 'SmallInt',
  bigint: 'BigInt',
  int64: 'BigInt',
  double: 'Double',
  doubleprecision: 'Double',
  float64: 'Double',
  float8: 'Double',
  float: 'Float',
  real: 'Float',
  float32: 'Float',
  float4: 'Float',
  text: 'Text',
  string: 'Text',
  utf8: 'Text',
  largeutf8: 'Text',
  varchar: 'Text',
  timestamp: 'Timestamp',
  timestampmicrosecond: 'Timestamp',
  timestampmillisecond: 'Timestamp',
  timestampnanosecond: 'Timestamp',
  date: 'Date',
  date32: 'Date',
  date64: 'Date',
  datetime: 'DateTime',
  time: 'Time',
  time64: 'Time',
  time64microsecond: 'Time',
  time64nanosecond: 'Time',
  json: 'Json',
  jsonb: 'Json',
  bytes: 'Bytes',
  binary: 'Bytes',
  largebinary: 'Bytes',
  bytea: 'Bytes',
  uuid: 'Uuid',
  file: 'File',
};

function canonicalKey(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, '');
}

function formatSqlName(kind: KalamDataTypeKind): string {
  return kind === 'DateTime' ? 'DATETIME' : kind.toUpperCase();
}

function parsePositiveInteger(value: unknown): number | undefined {
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : undefined;
}

function objectDescriptor(input: Record<string, unknown>): KalamDataTypeDescriptor | undefined {
  if ('Builtin' in input) {
    return parseKalamDataType(input.Builtin);
  }

  if ('Embedding' in input) {
    const dimension = parsePositiveInteger(input.Embedding);
    if (dimension) return { kind: 'Embedding', dimension, sqlName: `EMBEDDING(${dimension})` };
  }

  if ('Decimal' in input && input.Decimal && typeof input.Decimal === 'object') {
    const decimal = input.Decimal as Record<string, unknown>;
    const precision = parsePositiveInteger(decimal.precision);
    const scale = Number(decimal.scale ?? 0);
    if (precision && Number.isInteger(scale) && scale >= 0) {
      return {
        kind: 'Decimal',
        precision,
        scale,
        sqlName: `DECIMAL(${precision}, ${scale})`,
      };
    }
  }

  return undefined;
}

/**
 * Parse KalamDB schema metadata type strings into the canonical SDK descriptor.
 *
 * Accepts the SQL names emitted by `KalamDataType::sql_name()` plus common Arrow
 * aliases that can appear in older metadata or DESCRIBE output.
 */
export function parseKalamDataType(input: unknown): KalamDataTypeDescriptor {
  if (input && typeof input === 'object') {
    const parsed = objectDescriptor(input as Record<string, unknown>);
    if (parsed) return parsed;
  }

  const raw = String(input ?? '').trim();
  const embeddingMatch = raw.match(/^embedding\s*\(\s*(\d+)\s*\)$/i);
  if (embeddingMatch) {
    const dimension = Number(embeddingMatch[1]);
    return { kind: 'Embedding', dimension, sqlName: `EMBEDDING(${dimension})` };
  }

  const decimalMatch = raw.match(/^(?:decimal|numeric|decimal128)\s*\(\s*(\d+)\s*(?:,\s*(\d+)\s*)?\)$/i);
  if (decimalMatch) {
    const precision = Number(decimalMatch[1]);
    const scale = Number(decimalMatch[2] ?? 0);
    return {
      kind: 'Decimal',
      precision,
      scale,
      sqlName: `DECIMAL(${precision}, ${scale})`,
    };
  }

  const kind = SIMPLE_TYPE_ALIASES[canonicalKey(raw)] ?? 'Text';
  return { kind, sqlName: formatSqlName(kind) };
}