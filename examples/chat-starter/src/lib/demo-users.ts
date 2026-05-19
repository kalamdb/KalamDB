// Demo user credentials shared between server/index.ts (which brokers
// browser tokens) and src/agent/index.ts (which logs in directly as the
// task's owner to scope its KalamDB operations to that user's partition).
//
// THIS IS A DEMO. The passwords below match the CREATE USER statements in
// chat-app.sql and are SHARED — anyone who picks "alice" becomes alice.
// Production deployments should:
//   1. Remove this map and the chat-app.sql CREATE USER seeds.
//   2. Replace the agent's per-user client lookup with whatever your real
//      identity service provides (likely a token-exchange / impersonation
//      flow against KalamDB instead of password login).
//   3. Keep the production fence in server/index.ts active — it refuses to
//      boot under NODE_ENV=production unless explicitly opted out.

export const DEMO_USER_PASSWORDS = {
  alice: "demo-alice-pw",
  bob: "demo-bob-pw",
  carol: "demo-carol-pw",
} as const;

export type DemoUser = keyof typeof DEMO_USER_PASSWORDS;

export const DEMO_USER_LIST: ReadonlyArray<DemoUser> = ["alice", "bob", "carol"];
