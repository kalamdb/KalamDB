// Defense-in-depth gate for SQL the LLM produces via the query_database tool.
//
// The LLM is trusted to write reasonable SQL but is NOT trusted to be safe.
// We reject anything that isn't a single read-only SELECT against tables in
// our allowlist, and we cap result size by injecting LIMIT when missing.
//
// This is intentionally conservative. False negatives (rejecting a valid
// query) are recoverable — the model will retry with a simpler query.
// False positives (accepting something destructive) are not.

const ALLOWED_NAMESPACES = ["chat"] as const;

const DEFAULT_LIMIT = 200;
const MAX_SQL_BYTES = 4 * 1024;

export interface GuardResult {
  ok: boolean;
  /** Sanitized SQL with LIMIT injected if missing. Only present when ok. */
  sql?: string;
  /** Reason for rejection. Only present when !ok. */
  reason?: string;
}

/** Strip leading/trailing whitespace AND trailing semicolons. */
function trimStatement(sql: string): string {
  return sql.trim().replace(/;+\s*$/, "");
}

/** Detect SQL comments — they're a common way to dodge keyword filters. */
function containsComment(sql: string): boolean {
  return /--/.test(sql) || /\/\*/.test(sql);
}

/** Detect statement terminators OTHER than the trailing one we stripped. */
function containsExtraStatements(sql: string): boolean {
  // After trimStatement removes the trailing ; (and any whitespace after),
  // any remaining ; means the LLM tried to chain statements.
  return /;/.test(sql);
}

/** First non-whitespace keyword token. */
function leadingKeyword(sql: string): string {
  const m = sql.match(/^\s*([A-Za-z]+)/);
  return m ? m[1]!.toUpperCase() : "";
}

/**
 * Extract every `namespace.table` reference. Cheap, regex-based — not a SQL
 * parser. We accept that a sufficiently weird query (e.g., backticks, schema
 * search-path tricks) might slip through, but the allowlist ensures only
 * tables we own can be touched.
 */
function tableReferences(sql: string): string[] {
  // FROM / JOIN <ns>.<table>
  const re = /(?:from|join)\s+([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)/gi;
  const out: string[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(sql)) !== null) {
    out.push(`${m[1]!.toLowerCase()}.${m[2]!.toLowerCase()}`);
  }
  return out;
}

/** True if every namespace-qualified reference is in our allowlist. */
function allReferencesAllowed(sql: string): boolean {
  const refs = tableReferences(sql);
  if (refs.length === 0) return false; // require explicit ns.table to avoid catalog peeks
  return refs.every((r) =>
    (ALLOWED_NAMESPACES as readonly string[]).some((ns) => r.startsWith(`${ns}.`)),
  );
}

/** Inject a default LIMIT if the SQL doesn't end with one. */
function ensureLimit(sql: string): string {
  if (/\blimit\s+\d+\b/i.test(sql)) return sql;
  return `${sql} LIMIT ${DEFAULT_LIMIT}`;
}

/**
 * Returns {ok:true, sql} for a single read-only SELECT against allowed
 * tables (with LIMIT injected if missing), else {ok:false, reason}.
 */
export function guardSelect(input: string): GuardResult {
  if (typeof input !== "string" || input.trim().length === 0) {
    return { ok: false, reason: "Empty SQL." };
  }
  if (input.length > MAX_SQL_BYTES) {
    return { ok: false, reason: `SQL exceeds ${MAX_SQL_BYTES} byte limit.` };
  }
  if (containsComment(input)) {
    return { ok: false, reason: "Comments are not allowed in tool-issued SQL." };
  }
  const stmt = trimStatement(input);
  if (containsExtraStatements(stmt)) {
    return { ok: false, reason: "Only a single statement is allowed." };
  }
  const head = leadingKeyword(stmt);
  if (head !== "SELECT") {
    return {
      ok: false,
      reason: `Only SELECT is allowed; got ${head || "<empty>"}.`,
    };
  }
  if (!allReferencesAllowed(stmt)) {
    return {
      ok: false,
      reason: `Queries must reference tables in: ${ALLOWED_NAMESPACES.join(", ")} (use ns.table form).`,
    };
  }
  return { ok: true, sql: ensureLimit(stmt) };
}

export const SQL_GUARD = {
  defaultLimit: DEFAULT_LIMIT,
  allowedNamespaces: ALLOWED_NAMESPACES,
  maxBytes: MAX_SQL_BYTES,
} as const;
