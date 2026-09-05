# KalamDB Quick Start

Run a realtime React chat app with a topic worker, then open it in two tabs to see changes arrive live. The worker uses a simulated reply, so you do not need an external AI key.

## 1. Install the CLI

Use a current Node.js LTS release with npm. The native CLI supports macOS Apple Silicon, Linux x86-64 and ARM64, and Windows x86-64. Initial setup downloads the CLI, starter, and dependencies, so it needs internet access.

```bash
npm install -g @kalamdb/cli
kalam version
```

## 2. Create and start your app

```bash
mkdir my-app && cd my-app
kalam init --yes --template chat-with-ai --languages typescript --package-manager npm
kalam dev
```

`kalam init` downloads the chat starter and installs its dependencies. `kalam dev` starts or reuses a local KalamDB server, applies `kalam/schema.sql`, generates TypeScript tables, and runs the browser app and topic worker.

## 3. See a change travel through the app

Open the app URL printed in your terminal in two browser tabs. Send a message such as:

```text
latency spike after deploy
```

You should see the message in both tabs, live worker progress, and a saved reply. The reply is deterministic demo logic; you can replace it with your own model call in `src/agent.ts`.

The starter uses local demo credentials (`root` / `kalamdb123`) and connects to `http://127.0.0.1:2900` by default. These privileged demo sessions are for exploring the data flow. Use ordinary user sessions and the starter's SQL policies when building your application's access controls.

## 4. Make it yours

- Edit `src/App.tsx` to change the interface.
- Read `kalam/schema.sql` for shared rooms, membership policies, STREAM progress events, and the topic route.
- Edit `src/agent.ts` to change how the worker responds to new messages.
- Keep `kalam dev` running to apply schema changes and regenerate types as you work. The demo schema includes reset statements; inspect it before adapting it to data you want to keep.

The [chat example guide](../../examples/chat-with-ai/README.md) explains the complete flow. To start smaller, run `kalam init` in a new empty folder and choose a minimal template. To inspect available starters, run `kalam init --list-templates`.

## Next steps

- [CLI workflow](cli.md) — schema generation, migrations, environments, and deployment.
- [SQL reference](../reference/sql.md) — USER, SHARED, and STREAM tables, policies, and topics.
- [API reference](../api/api-reference.md) — SQL requests from your own client.
- [WebSocket protocol](../api/websocket-protocol.md) — live subscriptions.
- [Development setup](../development/development-setup.md) — build KalamDB from source and contribute.
