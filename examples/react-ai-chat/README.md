# React AI Chat Example

Personal AI assistant chat: **USER** conversations/messages, **STREAM** typing tokens, and a topic agent. Start from an empty folder with:

```bash
kalam init --list-templates --json
kalam init --yes --template react-ai-chat --languages typescript --package-manager npm
kalam dev start --agent
```

Use `--template chat-with-ai` instead for a **multi-user room chat** (SHARED tables + `CREATE POLICY`).

This example is a chat-application validation surface for `@kalamdb/react`. It keeps the browser code in [src/app](src/app) and the topic worker in [src/agent](src/agent), then demonstrates conversations, websocket-confirmed sends, streamed typing tokens, final assistant inserts, file attachments, and approval actions with small, readable components.

The example is a `kalam init`-style project. [kalam.toml](kalam.toml) points at [kalam/schema.sql](kalam/schema.sql), stores generated TypeScript schema types in [src/app/schema.generated.ts](src/app/schema.generated.ts), applies the initial migration in [kalam/migrations](kalam/migrations), and starts both the Vite app and topic agent through `kalam dev`.

## Quick Start

Use a current Kalam CLI. Update with `kalam self-update`, or run the workspace build: `../../target/debug/kalam dev`.

```bash
npm install
kalam dev
```

Open the Vite URL, usually `http://127.0.0.1:5176`.

`kalam init` rewrites local `file:` SDK paths to published `@kalamdb/*` packages at the CLI version. From the repo tree you can keep filesystem dependencies.

`kalam dev` starts a local KalamDB server, applies pending migrations, regenerates [src/app/schema.generated.ts](src/app/schema.generated.ts), watches [kalam/schema.sql](kalam/schema.sql), and runs both configured dev processes:

```toml
[dev.processes]
app = "npm run dev"
agent = "npm run agent"
```

The default local credentials are `root` / `kalamdb123` at `http://127.0.0.1:2900`, matching the local server managed by `kalam dev`.

While `kalam dev` is running you can access that server directly:

- **Admin UI:** [http://localhost:2900/ui](http://localhost:2900/ui) — sign in as `root` / `kalamdb123`
- **CLI:** `kalam --url http://127.0.0.1:2900 --user root --password kalamdb123`

Override them for app processes with `KALAM_URL`, `KALAM_USER`, and `KALAM_PASSWORD` for the agent, and `VITE_KALAM_URL`, `VITE_KALAM_USER`, and `VITE_KALAM_PASSWORD` for the browser.

To wipe local database state and start over, stop `kalam dev` and run `kalam db reset`, then `kalam dev` again.

If you want browser-only demo mode instead of the server-backed flow, set `VITE_KALAM_DEMO_MODE=true` in `.env.local` and run `npm run dev`.

Send a message like:

```text
customer refund needs approval
```

The agent will create approval rows, stream typing tokens, and continue after approval actions.

## Files Worth Reading

- [src/app/App.tsx](src/app/App.tsx): `KalamProvider` and `LiveQueries` orchestration.
- [src/app/components/Aside.tsx](src/app/components/Aside.tsx): conversation creation and selection.
- [src/app/components/Conversation.tsx](src/app/components/Conversation.tsx): sticky header, scrollable messages, sticky composer.
- [src/app/components/ChatComposer.tsx](src/app/components/ChatComposer.tsx): attach-file and enter-to-send composer.
- [src/agent/index.ts](src/agent/index.ts): deterministic topic-agent worker.
- [kalam/schema.sql](kalam/schema.sql): full namespace reset, table/topic creation, and seed data.
- [kalam.toml](kalam.toml): `kalam dev` project config for schema generation and dev processes.
- [kalam/migrations/0001_init.sql](kalam/migrations/0001_init.sql): initial bootstrap migration.

## Validation

```bash
npm run build
npm test
```

With `kalam dev` running in another terminal:

```bash
npm run test:e2e
```

## Windows Notes

The example scripts are Node-based and avoid bash-only helpers, so `npm install`, `npm run agent`, `npm run dev`, `npm run build`, and `npm test` work on Windows as long as Node.js and npm are installed. `kalam dev` uses the platform shell for configured processes, so the one-command workflow also works on Windows as long as the `kalam` CLI, Node.js, and npm are installed.
