import { scanSql } from "../src/agent/sql-scan.js";

// Split a SQL script on statement-terminating semicolons while respecting
// single-quoted string literals (with doubled-quote escapes). Implemented on
// top of the shared scanSql() walker so this matches sql-guard's behavior
// exactly.
export function splitStatements(sql: string): string[] {
  const positions = scanSql(sql).semicolons;
  const out: string[] = [];
  let start = 0;
  for (const pos of positions) {
    const stmt = stripCommentsAndTrim(sql.slice(start, pos));
    if (stmt.length > 0) out.push(stmt);
    start = pos + 1;
  }
  const tail = stripCommentsAndTrim(sql.slice(start));
  if (tail.length > 0) out.push(tail);
  return out;
}

/** Strip `-- ...` line comments and trim. Block comments are left in place
 *  (chat-app.sql doesn't use any). */
function stripCommentsAndTrim(s: string): string {
  return s.replace(/^\s*--.*$/gm, "").trim();
}
