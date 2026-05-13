# Chat With AI

This is a small Vite + React example for the simple ask-message flow in KalamDB.

What it does:

- the browser writes a user message into the `chat_demo.messages` USER table with the ORM
- `chat_demo.ai_inbox` receives those inserts as a topic
- `src/agent.ts` runs `runConsumer()` from `@kalamdb/consumer`
- the agent writes progress rows into `chat_demo.agent_events`
- the agent commits one assistant reply back into `chat_demo.messages`
- every open browser tab sees the rows through live subscriptions

There is no Gemini call in this example. The agent returns a deterministic demo reply so the example is easy to run locally.

This README uses the ask path: send one message and get one response. An agent-message workflow with human approval would add a pending approvals table and require a user to approve before the final assistant message is committed.

## Quick Start

From this folder:

```bash
npm install
```

This example is package-driven. It expects the KalamDB SDKs to be installed in `node_modules`, either from local filesystem dependencies such as `file:../../link/sdks/typescript/...` or from npm versions like `latest`.

Start a KalamDB server. The fastest Docker option is:

```bash
docker run -d --name kalamdb -p 8080:8080 -e KALAMDB_SERVER_HOST=0.0.0.0 -e KALAMDB_ROOT_PASSWORD=kalamdb123 -e KALAMDB_JWT_SECRET=local-dev-secret-please-change-32chars -v kalamdb_data:/data jamals86/kalamdb:latest
```

Then prepare the demo schema and environment file:

```bash
npm run setup
```

`setup` also runs `npm run generate:schema`, which writes `src/schema.generated.ts` from the live `chat_demo` namespace with `kTable.user(...)`, `kTable.stream(...)`, table config exports, inferred row types, and typed KalamDB system columns such as `_seq`.

Run the agent worker in one terminal:

```bash
npm run agent
```

Run the browser app in another terminal:

```bash
npm run dev
```

Open the Vite URL printed by `npm run dev`, usually `http://127.0.0.1:5173`.

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

The generated table metadata is also available in code:

```ts
chatMessagesConfig.tableType; // "user"
chatMessagesConfig.systemColumns; // ["_seq", "_deleted"]
```

## The Tiny Agent Path

The worker uses the generated row type directly. `runConsumer()` decodes KalamDB topic payloads and unwraps `{ row: ... }` envelopes automatically, so there is no custom parser in the demo worker:

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

That insert is enough to wake the agent because setup creates this route:

```sql
CREATE TOPIC chat_demo.ai_inbox;
ALTER TOPIC chat_demo.ai_inbox ADD SOURCE chat_demo.messages ON INSERT;
```

## What To Try

Type a short message such as:

```text
latency spike after deploy
```

You should see your user row, live agent progress, and then a final `AI reply:` row in the same chat thread. Open a second browser tab to see the live updates arrive there too.

## Files Worth Reading

- [src/App.tsx](src/App.tsx): browser client, ORM insert, and live subscriptions
- [src/schema.generated.ts](src/schema.generated.ts): generated `kTable()` definitions with `_seq`, table-kind metadata, config exports, and inferred row types
- [src/agent.ts](src/agent.ts): topic consumer and deterministic reply worker
- [chat-app.sql](chat-app.sql): USER table, STREAM table, and namespace setup
- [setup.mjs](setup.mjs): cross-platform local bootstrap script using the `kalam` CLI
- [tests/chat.spec.mjs](tests/chat.spec.mjs): browser end-to-end test

## Windows Notes

The example scripts are Node-based and avoid bash-only helpers, so `npm install`, `npm run setup`, `npm run agent`, `npm run dev`, and `npm test` work on Windows as long as Node.js, npm, and Playwright's browser dependencies are installed.

## Credentials

`npm run setup` creates or reuses this local account and writes `.env.local`:

- user: `admin`
- password: `kalamdb123`

The browser and the worker both use that account for the local demo. The worker uses `EXECUTE AS USER` when it writes agent events and assistant replies for the message sender. In production, a service account can do the same when its role is authorized for that target user.

## Test

With KalamDB running and after `npm run setup`:

```bash
npm test
```

The test starts the agent, opens browser tabs, sends an ask message, and verifies that both tabs receive the user message and the assistant reply.

## Troubleshooting

If setup cannot reach KalamDB, check the server health endpoint:

```bash
curl http://127.0.0.1:8080/health
```

If the agent is running but no reply appears, rerun setup so the topic route exists:

```bash
npm run setup
```

If port 5173 is busy, Vite prints the next available local URL. Use the URL from the terminal output.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See [../../LICENSE.txt](../../LICENSE.txt) and [../../NOTICE](../../NOTICE).
