![KalamDB](docs/images/kalamdb_logo.png)

# Postgres-compatible. Realtime built in. One binary.

**KalamDB is an open-source backend in a single Rust binary.** It combines a PostgreSQL-compatible SQL engine with live queries, JavaScript/TypeScript server functions, durable pub/sub, authentication, and built-in Raft clustering.

Use it as a **Postgres alternative** behind your existing backend, or connect your web, mobile, and AI apps directly through HTTP, WebSocket, and typed SDKs. Start with one process on your laptop. Deploy on your own infrastructure.

![CI](https://github.com/kalamdb/KalamDB/actions/workflows/ci.yml/badge.svg)
![Release](https://img.shields.io/github/v/release/kalamdb/KalamDB?display_name=tag)
![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)
![Docker Pulls](https://img.shields.io/docker/pulls/kalamdb/kalamdb)

[Get started](#get-started) · [Connect with Postgres tools](#connect-with-postgres-tools) · [Live queries](#query-once-stay-up-to-date) · [Documentation](https://kalamdb.org/docs) · [Releases](https://github.com/kalamdb/KalamDB/releases)

## Your backend, built in

| Capability | What you can do |
| --- | --- |
| **PostgreSQL syntax + PGWire** | Use familiar SQL, `psql`, DBeaver, and PostgreSQL drivers, including prepared queries and transactions. |
| **Live queries** | Subscribe to SQL results over WebSocket and keep your UI current as matching rows change. |
| **Server functions** | Run JavaScript and compiled TypeScript inside the embedded V8 runtime. Invoke backend logic through SQL, HTTP, or generated clients. |
| **Durable pub/sub** | Turn table changes into events for workers, integrations, and agents, with consumer groups, acknowledgements, and retries. |
| **Auth + access control** | Use authentication and row-level policies for shared data. Manage your backend through the included admin UI. |
| **Tables partitioned by user** | Create `USER` tables so each person's rows live in their own partition. Use `SHARED` tables for data everyone reads, and `STREAM` tables for events that expire. |
| **Tiered storage** | Keep recent rows in RocksDB. Flush older user and shared data to compressed Parquet on disk or object storage, and query both tiers as one table. |
| **Typed app APIs** | Generate TypeScript and Dart/Flutter table and function contracts from your SQL schema. |
| **Raft clustering** | Run multiple nodes with replicated data, leader election, and live subscriptions served by each node. |

Build chat, collaborative apps, live dashboards, and event-driven backends with the same database and runtime.

## Get started

Run the server with a [native binary](https://github.com/kalamdb/KalamDB/releases) or [Docker](docker/run/single/docker-compose.yml).

To try a complete app, start the React chat demo with live messages and a server function that replies automatically. The app tooling needs a current Node.js LTS with npm:

```bash
npm install -g @kalamdb/cli
mkdir my-app && cd my-app
kalam init --yes --template chat-with-ai --languages typescript --package-manager npm
kalam dev
```

**Open the app URL printed in your terminal in two tabs and send a message.** Both tabs update live. A topic-triggered function runs inside KalamDB and saves a reply. The demo simulates the AI response, so no model API key is needed.

`kalam dev` starts the local database, applies the SQL schema, generates app types, builds and activates server functions, and runs the app.

For a smaller starting point, run `kalam init` in an empty folder and choose a minimal template. See the [quick-start guide](docs/getting-started/quick-start.md) for setup details and demo credentials.

## Connect with Postgres tools

KalamDB supports PostgreSQL syntax and the PostgreSQL wire protocol (PGWire). Connect your SQL tools and drivers directly to the engine.

Enable the listener in `server.toml` (`kalam/server/server.toml` in CLI projects), then restart the server:

```toml
[postgres_wire]
enabled = true
host = "127.0.0.1"
port = 5432
```

Connect using your KalamDB root password:

```bash
psql -h 127.0.0.1 -p 5432 -U root -d kalam -W
```

Query the chat starter's messages:

```sql
SELECT id, content
FROM chat_demo.messages
WHERE room = 'main';
```

KalamDB implements PostgreSQL compatibility on its own Rust engine, powered by DataFusion and Arrow. Migration suitability depends on the SQL features and extensions your application uses; see the [SQL reference](docs/reference/sql.md) and [release notes](https://github.com/kalamdb/KalamDB/releases).

## Query once. Stay up to date.

Use the same query as a live subscription. In the chat starter, reuse the authenticated client from `src/db.ts`:

```ts
import { client } from './db';

const stop = await client.live(
  "SELECT id, content FROM chat_demo.messages WHERE room = 'main'",
  (rows) => console.log(rows),
  { lastRows: 100, limit: 100, onError: console.error },
);

// Call await stop() when the screen closes.
```

The SDK delivers the initial rows and maintains the current result as inserts, updates, and deletes arrive. Writes through PGWire, HTTP, or server functions feed the same live-query system. See the [TypeScript SDK](link/sdks/typescript/client/) and [React bindings](link/sdks/typescript/react/).

## Put your logic next to your data

Declare tables, types, functions, and topics in SQL. Implement your server logic in JavaScript or TypeScript. KalamDB generates typed contracts and runs your deployed code in its embedded V8 runtime.

A chat message can update connected screens, publish a durable event, trigger a function, and stream the saved reply back to the app. The same building blocks work for notifications, background processing, and AI agents.

Explore the chat starter's [SQL schema](examples/chat-with-ai/kalam/schema.sql) and [server functions](examples/chat-with-ai/functions/src/chat_demo/), or read the [deployment guide](docs/getting-started/cli.md#deploy-with-migration-guardrails).

## One node to a cluster

Start with one binary and local disk. Add nodes with **Multi-Raft replication** for availability and more client connections. Clustering is part of the engine.

`USER` tables are partitioned by user, so one person's rows stay separate from everyone else's. Recent rows stay in RocksDB. Older user and shared data moves to compressed Parquet on filesystem or object storage, and the same SQL query reads both tiers. `STREAM` tables stay hot and expire by TTL.

[Download a binary](https://github.com/kalamdb/KalamDB/releases) · [Run with Docker](docker/run/single/docker-compose.yml) · [Try a three-node cluster](docker/run/cluster/docker-compose.yml) · [Cluster architecture](docs/architecture/raft-replication.md) · [Storage architecture](docs/architecture/hot-cold-storage-unification.md)

## Explore and contribute

- **SDKs:** [TypeScript / JavaScript](link/sdks/typescript/client/) · [React](link/sdks/typescript/react/) · [ORM](link/sdks/typescript/orm/) · [Dart / Flutter](link/sdks/dart/link/) · [Rust](link/sdks/rust/).
- **Examples:** [Realtime chat](examples/chat-with-ai/README.md) · [AI assistant](examples/react-ai-chat/README.md) · [Background worker](examples/summarizer-agent/README.md).
- **Get involved:** [Build from source](docs/development/development-setup.md) · [Report a bug or suggest a feature](https://github.com/kalamdb/KalamDB/issues).

KalamDB is under active development and licensed under [Apache 2.0](LICENSE.txt).

**If KalamDB is useful to you, give it a star to help others discover it.**
