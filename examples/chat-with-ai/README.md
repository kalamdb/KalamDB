# Chat With AI

Realtime React chat on KalamDB. SHARED rooms with `CREATE POLICY`, a personal USER inbox, STREAM copilot progress, and a **topic trigger** that runs inside the database. There is no separate agent process.

```bash
kalam init --yes --template chat-with-ai --languages typescript --package-manager npm
kalam dev
```

Use `--template react-ai-chat` when you want a personal assistant with approvals.

## Quick Start

From this folder, use the **0.7+ CLI from this repo**. A 0.6 `kalam` on PATH
(`~/.kalam/bin/kalam`) applies `CREATE PROCEDURE` without activating the JS
module, so `CALL` fails with `procedure not implemented` and overwrites
`src/generated/kalam.ts` (the app then fails with
`does not provide an export named 'createKalam'`).

```bash
npm install
npm run kalam:schema
npm run kalam:dev
```

`kalam dev` starts a local server, applies `kalam/schema.sql`, generates `src/generated/kalam.ts`, builds and **activates** the TypeScript procedures, and runs Vite. You should see `activated function module backend`. If activation fails, the CLI prints an error; the procedures stay `missing` until it succeeds.

```toml
[dev.processes]
app = "npm run dev"
```

Open the Vite URL, usually `http://127.0.0.1:5174`. Sign-in defaults are `root` / `kalamdb123` at `http://127.0.0.1:2900`. Set `VITE_KALAM_USER` to chat as someone else.

```bash
kalam db reset
npm run kalam:dev
```

```bash
npm run kalam:deploy
```

## What It Does

1. The browser calls `chat_demo.join_room`.
2. Sending calls `chat_demo.send_message` (`room` → SHARED transcript, `direct` → USER inbox).
3. Topic `chat_demo.ai_inbox` fans those INSERTs to `chat_demo.on_user_message`.
4. The procedure writes STREAM rows, then one assistant reply.
5. `liveTable` keeps every tab in sync.

The reply is deterministic so the example runs without an external AI API.

## The Write Path

```ts
await api.chatDemo.joinRoom({ room_id: ROOM });
await api.chatDemo.sendMessage({ target: 'room', target_id: ROOM, content });
```

Personal inbox:

```ts
await api.chatDemo.sendMessage({
  target: 'direct',
  target_id: CHAT_USERNAME,
  content,
});
```

## Files Worth Reading

- `kalam/schema.sql` — tables, policies, procedures, trigger
- `functions/src/chat_demo/` — `join_room`, `send_message`, `on_user_message`
- `src/App.tsx` — `liveTable` + the send form
- `src/db.ts` — `createClient` + `createKalam(client)`

## Tests

With a KalamDB server available (and functions activated via `npm run kalam:dev` or `npm run kalam:deploy`):

```bash
npm test
```
