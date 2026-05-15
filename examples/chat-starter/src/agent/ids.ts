// Shared identifier helpers used by the agent and any test code that needs
// to validate IDs without importing the agent's runtime (which has top-level
// side-effects).

export const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Validates a UUID and returns it wrapped as a SQL string literal. */
export function uuidLit(id: string): string {
  if (!UUID_RE.test(id)) {
    throw new Error(`agent: refused non-UUID identifier: ${id}`);
  }
  return `'${id}'`;
}
