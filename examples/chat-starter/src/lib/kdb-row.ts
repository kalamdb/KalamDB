// KalamDB result-row helpers. The runtime is inconsistent about row cell
// types — non-null cells arrive as KalamCellValue wrappers (with typed
// `.asString()` / `.asBool()` / `.asInt()` accessors), but SQL NULL cells
// arrive as plain `null`. Calling `.asString()` on the latter crashes,
// so call sites that care about both go through `unwrap()` (which handles
// either shape and returns a plain JS value) instead of typed accessors.

import type { QueryResponse } from "@kalamdb/client";

/**
 * Pulls the first result's `named_rows` out of a QueryResponse, narrowed
 * to a plain Record array. Each cell is `unknown` because the runtime
 * mixes `KalamCellValue` (non-null cells) and plain `null` (SQL NULL),
 * and TypeScript can't tell which you'll get per column.
 */
export function extractRows(res: unknown): Array<Record<string, unknown>> {
  return (
    (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
      ?.named_rows ?? []
  );
}

/**
 * Reaches into a row cell and returns its plain JS value. Handles both
 * KalamCellValue wrappers (calls `.asString()` or `.toJson()`) and the
 * plain primitives that arrive for SQL NULL / consumer-event payloads.
 */
export function unwrap(value: unknown): unknown {
  if (value && typeof value === "object" && "asString" in value) {
    return (value as { asString: () => string }).asString();
  }
  if (value && typeof value === "object" && "toJson" in value) {
    return (value as { toJson: () => unknown }).toJson();
  }
  return value;
}

/**
 * Convert a KalamDB BigInt-typed cell (count(*), SUMs, …) to a plain
 * `number`. Routes through unwrap so wrapped + plain cells both work.
 * Values above 2^53 lose precision — fine for row counts in a single
 * tenant's partition; not fine for total-bytes or microsecond timestamps.
 */
export function kdbBigIntToNumber(cell: unknown): number {
  const v = unwrap(cell);
  if (v === null || v === undefined) return 0;
  return Number(String(v));
}

/** Re-export the SDK's QueryResponse type so callers don't have to drag the
 *  SDK import along just to type the result they're about to pass through
 *  extractRows. */
export type { QueryResponse };
