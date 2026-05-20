// KalamDB result-row helpers shared by the agent and the agent's tools.
// The WASM client returns cells as wrapped objects exposing asString() or
// toJson(); unwrap() reaches in once and produces a plain JS value the rest
// of the code can pattern-match against.

import type { QueryResponse } from "@kalamdb/client";

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
 * Pulls the first result's `named_rows` out of a QueryResponse, narrowed
 * to a plain Record array. Replaces the same five-deep cast that was
 * sprinkled across the agent + tools.
 */
export function extractRows(res: unknown): Array<Record<string, unknown>> {
  return (
    (res as { results?: Array<{ named_rows?: Array<Record<string, unknown>> }> }).results?.[0]
      ?.named_rows ?? []
  );
}

/**
 * Convert a KalamDB cell that holds a BigInt-typed value (count(*), SUMs,
 * etc.) to a plain `number`. KalamDB returns these as wrapped objects;
 * `Number(wrapper)` yields NaN because the wrapper has no `valueOf`. We
 * route through asString/toJson via unwrap() and then through String() so
 * the bigint→string→number cast is explicit instead of hidden in a five-
 * argument `as` chain.
 *
 * Callers should be aware that values above 2^53 lose precision — fine
 * for row counts in a single tenant's partition; not fine for total bytes
 * or microsecond timestamps.
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
