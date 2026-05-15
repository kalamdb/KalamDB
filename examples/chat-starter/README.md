# KalamDB Chat Starter

A production-shaped AI chat skeleton built on **KalamDB live queries**, **topic consumers**, **React**, and a **pluggable LLM adapter**. Every real-time behavior in the app — token streaming, human-in-the-loop approvals, mid-stream cancellation — is driven by KalamDB primitives. No WebSocket-glue, no polling, no manual event bus.

This is a starter you can read end-to-end in an hour and lift into your own product. It is not a finished app — see [Deployment checklist](#deployment-checklist) for what to harden first.

## Features

- **Streaming tokens** as rows in a `typing_tokens` table. The client subscribes; the agent INSERTs in batches. Frontend never opens an SSE/WebSocket of its own.
- **Approvals** (human-in-the-loop) as rows in an `approvals` table. The agent inserts; the UI shows; the user clicks; the agent's own live query wakes up.
- **Stop button** is just an `UPDATE` on the `tasks` table. The agent subscribes to its own task row and aborts mid-stream when `is_cancelled` flips to true. Live-query-as-control-plane.
- **Topic-based work queue.** New tasks flow through a `chat.task_events` topic (sourced from `chat.tasks ON INSERT`); the agent uses `runConsumer` for at-least-once delivery and group-based load balancing across replicas.
- **Pluggable LLM** with three adapters out of the box: OpenAI, Anthropic, and a mock for first-run demos without an API key.

## Quick start

```bash
# 1. Boot KalamDB locally (port 8080)
docker compose up -d

# 2. Configure (then optionally drop in an OPENAI_API_KEY or ANTHROPIC_API_KEY)
cp .env.example .env

# 3. Install + create the schema
npm install
npm run setup

# 4. Run the full dev stack (vite + backend + agent)
npm run dev
```

Open <http://127.0.0.1:5173>, click **New chat**, send a message. Watch the assistant reply stream in token-by-token. While it's streaming, hit **Stop** — the reply truncates and is marked `(stopped)`.

To trigger an approval flow, say something destructive ("delete my old account data"). The agent emits a `request_approval` tool call; an approval card appears; click Approve or Reject.

Without `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` set, the agent uses a **mock adapter** with canned replies that still exercises the streaming, approval, and cancellation paths.

## Architecture

```
                                  topic
       +-----------+   /api/auth/token    +---------+
       |           |  ----------------->  |         |
       |  Browser  |                       | Backend |
       |  (React)  |  <-----------------   |  (Node) |
       |           |       JWT             |         |
       +-----+-----+                       +----+----+
             |                                  |
             | live SQL / WebSocket             | login as root,
             | (JWT-authenticated)              | mint per-user token
             v                                  v
       +------------------------ KalamDB -----------------+
       |  chat.conversations   chat.messages              |
       |  chat.typing_tokens   chat.approvals             |
       |  chat.tasks  ─── ON INSERT ──>  chat.task_events |
       +-----^------+--------------------------------+----+
             |      | live                        topic
             |      v                                |
       +-----+---------+                  +---------+----+
       | Agent worker  |  <-- runConsumer | Agent worker |
       | (per-task     |                  | (more replicas|
       |  abort sub)   |                  |  load-balance)|
       +---------------+                  +--------------+
```

Three Node processes run in dev (all started by `npm run dev`):

1. **Vite** — serves the React app on `:5173`. Proxies `/api` → backend, `/kdb` → KalamDB.
2. **Backend** (`server/`) — owns the KalamDB root credentials. Exposes `/api/auth/token`, which logs into KalamDB and returns a short-lived access token to the browser. In your real app this is where you authenticate the user (session cookie, OAuth, etc.) before minting the KalamDB token.
3. **Agent worker** (`src/agent/`) — consumes the `chat.task_events` topic, drives the LLM, writes typing tokens and the assistant message back to KalamDB. Subscribes to each task's row to receive cancellation signals.

## Tables

| Table                      | Purpose                                                                                                                                                                |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `chat.conversations`       | Top-level chat threads (sidebar).                                                                                                                                      |
| `chat.messages`            | User + assistant turns. `status`: `pending` → `streaming` → `final` / `cancelled` / `error`. The agent inserts the assistant row idempotently when it picks up a task. |
| `chat.typing_tokens`       | One row per LLM delta batch. Cleared when the message reaches a terminal status.                                                                                       |
| `chat.approvals`           | Human-in-the-loop checkpoints. Status flips to `approved` / `rejected` when the user clicks.                                                                           |
| `chat.tasks`               | In-flight agent work. `WITH (TYPE = 'USER')` — required for the ON-INSERT-sourced topic to carry user metadata. The agent subscribes to its own row for cancellation.  |
| `chat.task_events` (topic) | Sourced from `chat.tasks ON INSERT`. Agents consume via `runConsumer`.                                                                                                 |

## LLM providers

The pluggable adapter lives at `src/lib/llm/`. Three implementations ship:

| Provider  | When it's selected                                                                               | Env vars                                                                     |
| --------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| OpenAI    | `LLM_PROVIDER=openai`, or `OPENAI_API_KEY` set with no provider override                         | `OPENAI_API_KEY`, `OPENAI_MODEL` (default `gpt-4o-mini`)                     |
| Anthropic | `LLM_PROVIDER=anthropic`, or `ANTHROPIC_API_KEY` set with no provider override and no OpenAI key | `ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL` (default `claude-haiku-4-5-20251001`) |
| Mock      | `LLM_PROVIDER=mock`, or no API keys set                                                          | none                                                                         |

To add another (Bedrock, local Ollama, etc.), implement the `LlmAdapter` interface in a new file under `src/lib/llm/` and wire it into `getLlmAdapter()` in `src/lib/llm/index.ts`:

```ts
export interface LlmAdapter {
  readonly name: string;
  stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined>;
}
```

The stream yields `LlmStreamEvent` values: `{type: "text", delta}`, `{type: "tool_call", call}`, `{type: "done", reason}`. This lets the agent interleave token deltas, tool calls, and termination signals naturally.

## Tests

`npm test` runs the Playwright suite (`tests/chat-flow.spec.ts`) — three deterministic tests covering streaming, approvals, and cancellation. They run against the mock adapter so they don't need API keys. Boot the dev stack first (`npm run dev`), then run tests in another terminal.

## How auth works in this starter

The browser holds **no** KalamDB credentials. The flow is:

1. Browser calls `POST /api/auth/token` (proxied through Vite to `server/index.ts`).
2. The backend logs into KalamDB with the bundled root credentials (env-only, never bundled into the frontend) and returns the resulting access token to the browser.
3. The browser uses that token for live queries and SQL via `Auth.jwt(...)` in the `@kalamdb/client` SDK.

This keeps the secret on the server. **What it does NOT do** is mint per-user tokens — every browser session gets a token with the same (admin) authority. That's fine for a starter; replace `/api/auth/token` with logic that:

- Validates the caller's session (cookie, OAuth, whatever your auth is)
- Mints a KalamDB token scoped to that user's data
- Caches/refreshes it server-side

See `server/index.ts` for the boundary; that's the file to modify.

## Deployment checklist

This is a starter. Before running it anywhere real:

- [ ] Replace the bundled `kalamdb-dev-password` and `KALAMDB_JWT_SECRET` in `docker-compose.yml` (use shell env substitution, not committed values).
- [ ] Tighten `KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS` from `*` to your actual frontend origin.
- [ ] Set `KALAMDB_ALLOW_REMOTE_SETUP=false` — provision schemas with migrations, not the bundled `npm run setup`.
- [ ] Implement real user auth in `server/index.ts:/api/auth/token` and mint scoped per-user tokens, not the shared root token.
- [ ] Add retry on transient LLM errors in `src/agent/index.ts` — the current code surfaces any failure as `status='error'` immediately. For a real product, retry rate-limit / transient network failures with backoff.
- [ ] Add proper observability (the agent currently `console.log`s; replace with structured logging + your tracing system).
- [ ] Run multiple agent replicas behind the same `KALAMDB_GROUP` — runConsumer load-balances automatically.
- [ ] Pin the Docker image tag in `docker-compose.yml` to whatever version you've actually tested against.

## Scripts

| Command                             | Effect                                                                              |
| ----------------------------------- | ----------------------------------------------------------------------------------- |
| `npm run dev`                       | Boot vite + backend + agent under one process (via `concurrently`).                 |
| `npm run vite` / `server` / `agent` | Run each individually.                                                              |
| `npm run setup`                     | (Re)create the schema in `chat-app.sql`. Destructive: drops and recreates `chat.*`. |
| `npm run build`                     | Type-check + build the Vite app for production.                                     |
| `npm test`                          | Playwright e2e tests (requires the dev stack to be running).                        |
| `npm run typecheck`                 | Strict TS check across both app + agent configs.                                    |

## License

Apache-2.0 (matches KalamDB).
