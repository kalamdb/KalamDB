// Shared user-identifier validation. The agent (server-side) uses this to
// reject malformed `change.user` payloads from the consumer event stream;
// userStore (browser-side) uses it to refuse tampered localStorage entries.
//
// The narrow character set is deliberate — these IDs end up in SQL WHERE
// clauses and in the EXECUTE AS USER / login path, so anything that could
// break out of a string literal (quotes, backslashes, parens) is forbidden.

export const USER_RE = /^[a-zA-Z0-9_.-]{1,64}$/;
