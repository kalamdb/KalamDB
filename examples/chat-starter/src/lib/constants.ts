// Cross-cutting constants for the chat-starter.

/**
 * Sentinel conversation_id used in App.tsx's LiveQueries `where` clauses when
 * no conversation is selected — produces a SELECT that matches zero rows so
 * the subscription stays cheap.
 *
 * This exists because `@kalamdb/react` LiveQueries doesn't yet accept a
 * `skip` option to disable a query entirely. Replace with the real skip
 * mechanism when it lands (tracked as an SDK ask).
 */
export const NO_CONVERSATION_SENTINEL = "__none__" as const;

/**
 * Source tag stamped on every doc the seed-docs script inserts. Used by the
 * seed script as a wide-net predicate when wiping prior seeds, so a renamed
 * or removed doc id doesn't leave orphans.
 */
export const SEED_SOURCE_TAG = "starter/seed" as const;
