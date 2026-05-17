// Tiny lexer-ish walker over a SQL string that respects single-quoted
// literals (with doubled-quote escapes). Shared between scripts/sql-split.ts
// (the chat-app.sql splitter) and src/agent/sql-guard.ts (the LLM
// query_database tool's defense layer) so legitimate user content like
// "WHERE body LIKE '%--%'" or "WHERE body = 'a;b'" doesn't trip the guard.
//
// Not a full SQL lexer — no double-quoted identifiers, no E'...' escapes,
// no nested block comments. Sufficient for chat-app.sql and for guarding
// the very narrow subset of SQL the LLM is allowed to produce.

export interface SqlScanResult {
  /** Indices (in the input) of '--' line-comment starts outside string literals. */
  lineComments: number[];
  /** Indices of '/*' block-comment starts outside string literals. */
  blockComments: number[];
  /** Indices of ';' characters outside string literals. */
  semicolons: number[];
}

export function scanSql(sql: string): SqlScanResult {
  const out: SqlScanResult = { lineComments: [], blockComments: [], semicolons: [] };
  let inSingle = false;
  for (let i = 0; i < sql.length; i++) {
    const ch = sql[i];
    if (ch === "'") {
      // Doubled-quote escape: '' inside a string stays a string.
      if (inSingle && sql[i + 1] === "'") {
        i++;
        continue;
      }
      inSingle = !inSingle;
      continue;
    }
    if (inSingle) continue;
    if (ch === "-" && sql[i + 1] === "-") {
      out.lineComments.push(i);
      // Skip to end of line so we don't double-report semicolons inside the comment.
      const nl = sql.indexOf("\n", i + 2);
      i = nl === -1 ? sql.length : nl;
      continue;
    }
    if (ch === "/" && sql[i + 1] === "*") {
      out.blockComments.push(i);
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? sql.length : end + 1;
      continue;
    }
    if (ch === ";") {
      out.semicolons.push(i);
    }
  }
  return out;
}

export function hasCommentOutsideStrings(sql: string): boolean {
  const r = scanSql(sql);
  return r.lineComments.length > 0 || r.blockComments.length > 0;
}

export function semicolonPositionsOutsideStrings(sql: string): number[] {
  return scanSql(sql).semicolons;
}
