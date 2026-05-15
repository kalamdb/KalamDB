// Split a SQL script on statement-terminating semicolons while respecting
// single-quoted string literals (with doubled-quote escapes).
//
// Not a full SQL parser — sufficient for the schema we ship. Comments and
// double-quoted identifiers don't appear in chat-app.sql; if you add them,
// this helper needs to grow.
export function splitStatements(sql: string): string[] {
  const stripped = sql.replace(/^\s*--.*$/gm, "");
  const out: string[] = [];
  let buf = "";
  let inSingle = false;
  for (let i = 0; i < stripped.length; i++) {
    const ch = stripped[i];
    if (ch === "'") {
      if (inSingle && stripped[i + 1] === "'") {
        buf += "''";
        i++;
        continue;
      }
      inSingle = !inSingle;
      buf += ch;
      continue;
    }
    if (ch === ";" && !inSingle) {
      const stmt = buf.trim();
      if (stmt.length > 0) out.push(stmt);
      buf = "";
      continue;
    }
    buf += ch;
  }
  const tail = buf.trim();
  if (tail.length > 0) out.push(tail);
  return out;
}
