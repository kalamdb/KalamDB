# Chat With AI

Small Vite + React example for the ask-message flow in KalamDB.

What it does:

- the browser writes a user message into the `chat_demo.messages` USER table with the ORM
- `chat_demo.ai_inbox` receives those inserts as a topic
- `src/agent.ts` runs `runConsumer()` from `@kalamdb/consumer`
- the agent writes progress rows into `chat_demo.agent_events`
- the agent commits one assistant reply back into `chat_demo.messages`
- every open browser tab sees the rows through live subscriptions

There is no external AI API call. The agent returns a deterministic demo reply so the example is easy to run locally.

## Quick Start

From this folder:

```bash
npm install
kalam dev
```

`kalam dev` starts a local KalamDB server, applies `kalam/schema.sql`, keeps `kalam/migrations/` up to date, regenerates `src/schema.generated.ts`, and runs both configured dev processes:

- `app = "npm run dev"`
- `agent = "npm run agent"`

Open the Vite URL printed in the terminal, usually `http://127.0.0.1:5173`.

Default local credentials are `root` / `kalamdb123` at `http://127.0.0.1:2900`.

## What To Try

Type a short message such as:

```text
latency spike after deploy
```

You should see your user row, live agent progress, and then a final `AI reply:` row in the same chat thread. Open a second browser tab to see the live updates arrive there too.

## The Tiny Write Path

The browser write is intentionally small. The table comes directly from `src/schema.generated.ts`, then the form submit writes a user row through the ORM:

```ts
await db.insert(chatMessages).values({
  room: ROOM,
  role: 'user',
  author: CHAT_USERNAME,
  sender_username: CHAT_USERNAME,
  content,
});
```

## The Tiny Agent Path

The worker uses the generated row type directly. `runConsumer()` decodes KalamDB topic payloads and unwraps `{ row: ... }` envelopes automatically:

```ts
await runConsumer<ChatDemoMessages>({
  client,
  name: 'chat-ai-agent',
  topic: 'chat_demo.ai_inbox',
  groupId: 'chat-ai-agent',
  onChange: async (_ctx, change) => {
    const row = change.data;
    if (row.role !== 'user') return;
    // Build and persist the assistant reply here.
  },
});
```

## Files Worth Reading

- `kalam.toml`: `kalam dev` project config for schema generation and dev processes.
- `kalam/schema.sql`: USER table, STREAM table, topic route, and seed data.
- `kalam/migrations/0001_init.sql`: initial bootstrap migration.
- `src/App.tsx`: browser client, ORM insert, and live subscriptions.
- `src/schema.generated.ts`: generated `kTable()` definitions and row types.
- `src/agent.ts`: topic consumer and deterministic reply worker.
- `tests/chat.spec.mjs`: browser end-to-end test.

## Tests

With a KalamDB server available:

```bash
npm test
```

The test suite prepares isolated test data, starts the agent, opens browser tabs, sends an ask message, and verifies that both tabs receive the user message and assistant reply.
