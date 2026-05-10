# React AI Chat Example

This example is a chat-application validation surface for `@kalamdb/react`. It keeps the browser code in [src/app](src/app) and the topic worker in [src/agent](src/agent), then demonstrates conversations, websocket-confirmed sends, streamed typing tokens, final assistant inserts, file attachments, and approval actions with small, readable components.

The example setup is intentionally simple: one SQL file and one setup script. The setup script uses the installed `kalam` CLI, clears the two example topics if they already exist, imports [chat-app.sql](chat-app.sql), then writes `.env.local` for the browser and agent.

## Quick Start

```bash
npm install
npm run setup
npm run agent
npm run dev
```

Open the Vite URL, usually `http://127.0.0.1:5176`.

`npm run setup` clears the example topics if they already exist, runs `kalam --file chat-app.sql`, creates the tables and topics, seeds the default conversation, and writes `.env.local` with `VITE_KALAMDB_DEMO_MODE=false`.

If you need non-default credentials, set `KALAMDB_URL`, `KALAMDB_USER`, and `KALAMDB_PASSWORD` before running the script.

Send a message like:

```text
customer refund needs approval
```

The agent will create approval rows, stream typing tokens, and continue after approval actions.

If you want browser-only demo mode instead of the server-backed flow, skip `npm run setup` and set `VITE_KALAMDB_DEMO_MODE=true` in `.env.local`.

## Files Worth Reading

- [src/app/App.tsx](src/app/App.tsx): `KalamProvider` and `LiveQueries` orchestration.
- [src/app/components/Aside.tsx](src/app/components/Aside.tsx): conversation creation and selection.
- [src/app/components/Conversation.tsx](src/app/components/Conversation.tsx): sticky header, scrollable messages, sticky composer.
- [src/app/components/ChatComposer.tsx](src/app/components/ChatComposer.tsx): attach-file and enter-to-send composer.
- [src/agent/index.ts](src/agent/index.ts): deterministic topic-agent worker.
- [chat-app.sql](chat-app.sql): full namespace reset, table/topic creation, and seed data.

## Validation

```bash
npm run build
npm test
```

The package scripts build the local TypeScript SDKs first if their `dist` folders are missing.