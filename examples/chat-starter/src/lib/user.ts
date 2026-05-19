// User-identifier helpers shared by the agent (server-side) and the
// userStore (browser-side). Kept platform-neutral so both tsconfigs can
// include this file.
//
// USER_RE is the single source of truth for what counts as a valid KalamDB
// user identifier in this starter. It must stay narrow enough that
// escapeUser's single-quote escape is sufficient defense-in-depth for the
// `EXECUTE AS USER '<...>'` SQL wrapping — i.e. no quotes, no backslashes,
// no parentheses, nothing that could break out of the literal.

export const USER_RE = /^[a-zA-Z0-9_.-]{1,64}$/;

/** Escapes a user identifier for safe inlining into EXECUTE AS USER '<...>'.
 *  USER_RE already forbids single quotes, so this is defense-in-depth. */
export function escapeUser(user: string): string {
  return user.replace(/'/g, "''");
}

/** Wraps `sql` (single statement, no trailing ;) in
 *  `EXECUTE AS USER '<user>' (...)`. Used by callers that need the wrapper
 *  without the SDK's one-shot executeAsUser() helper — primarily live()
 *  subscriptions. */
export function asUser(user: string, sql: string): string {
  return `EXECUTE AS USER '${escapeUser(user)}' (${sql.trim().replace(/;+\s*$/g, "")})`;
}
