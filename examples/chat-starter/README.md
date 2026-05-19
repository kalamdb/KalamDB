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

1. The sidebar dropdown picks a demo user (`alice`, `bob`, `carol`, or "Admin (default)"). The choice is persisted in `localStorage`.
2. Browser calls `POST /api/auth/token` with `{ user }` in the body (proxied through Vite to `server/index.ts`).
3. The backend looks the user up against its in-memory password map, logs into KalamDB as that user, and returns a short-lived access token.
4. The browser uses that token for live queries and SQL via `Auth.jwt(...)` in the `@kalamdb/client` SDK.
5. Every USER-table SQL the agent issues on behalf of that user is wrapped in `EXECUTE AS USER '<user>'`, so KalamDB's per-user table partitioning enforces isolation end-to-end.

Open two browser windows, pick a different demo user in each, and verify your conversations are isolated.

CSRF defense:

- `POST /api/auth/token` rejects requests whose `Content-Type` isn't `application/json` (blocks "simple request" form-style forgery).
- It also rejects cross-origin requests whose `Origin` header isn't in the `ALLOWED_ORIGINS` env list. Same-origin script POSTs don't send `Origin` and are permitted.

### Plugging in real auth

The dropdown is intentionally fake — passwords live server-side and anyone who picks "alice" becomes alice. That's safe for a local demo, fatal on the public internet. The starter's production fence (`ALLOW_UNAUTHENTICATED_TOKENS`) refuses to boot under `NODE_ENV=production` so this never ships by accident.

The swap is a single file: **`server/index.ts`**. The `/api/auth/token` handler is the seam. Replace the demo-user lookup with the real flow:

1. **Validate the caller** using whatever your real auth is — session cookie, OAuth, your IdP, etc. Reject unauthenticated callers with 401 before touching KalamDB. (If you use cookies, KEEP the existing CSRF gates and add SameSite + CSRF tokens if your auth crosses sites.)
2. **Resolve the caller to a KalamDB user identifier** — either map your auth's user-id to a `chat.users`-shaped table, or auto-provision a KalamDB user on first login.
3. **Mint a KalamDB token scoped to that user** by calling KalamDB's `/v1/api/auth/login` with that user's credentials (kept in a secret manager, not the source). The existing `makeTokenFetcher` already caches tokens per-username and refreshes near expiry — keep that logic, just feed it the resolved username.
4. **Remove the demo seeds** from `chat-app.sql` (`CREATE USER alice/bob/carol`) and the `DEMO_USERS` map in `server/index.ts`. The frontend `UserPicker` is only useful for the demo — either remove it from `src/components/Sidebar.tsx` or repurpose it to show the signed-in user's name.

Everything downstream of `/api/auth/token` — the agent's `EXECUTE AS USER`, the per-user live subscriptions, the `chat.*` table partitioning — keeps working unchanged. The boundary is the token-issuance step; the data plane is already real.

## Deployment checklist

This is a starter. Before running it anywhere real, the operator MUST address each item below. The starter has fences in code (refusing to start with unsafe defaults under `NODE_ENV=production`) but they are deliberately overridable so they can't be ignored silently.

**Security boundary**

- [ ] **Replace `/api/auth/token` with real caller auth.** The default implementation mints a root-equivalent KalamDB token to any caller. In `NODE_ENV=production`, the server refuses to start unless `ALLOW_UNAUTHENTICATED_TOKENS=true` is set — that env is a fence the operator must deliberately step over, not a feature flag. The right fix is to validate the user's session in `server/index.ts` (cookie / OAuth / your IdP) before minting a per-user KalamDB token. See `server/index.ts:assertProductionFence`.
- [ ] **Set `TRUST_PROXY=1` only behind a known reverse proxy.** Without it, the rate limiter and request logging fall back to the socket address. With it, `X-Forwarded-For` is honored — which is mandatory if you terminate TLS at a proxy, but a credential firehose if you don't (an attacker rotates the header and bypasses per-IP limits). If your load balancer doesn't strip or replace untrusted `X-Forwarded-For`, do NOT set this.
- [ ] **Replace `KALAMDB_JWT_SECRET`.** The default value in `docker-compose.yml` is in the repo — anyone reading this README knows it. Set via env / Docker secret / your secret manager.
- [ ] **Replace `KALAMDB_ROOT_PASSWORD`.** Same.
- [ ] **Tighten `KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS`** from `*` to your actual frontend origin.
- [ ] **Set `KALAMDB_ALLOW_REMOTE_SETUP=false`** — provision schemas with migrations, not the bundled `npm run setup`.

**Reliability & scaling**

- [ ] Run multiple agent replicas behind the same `KALAMDB_GROUP`. For Docker Compose: `docker compose -f docker-compose.prod.yml up -d --scale agent=N`. For Docker Swarm or Kubernetes, set the appropriate replica count on the agent deployment. `deploy.replicas` in the compose file is intentionally unset (Swarm-only field, silently ignored by plain Compose).
- [ ] Pin the KalamDB image tag in `docker-compose.yml` / `docker-compose.prod.yml` to whatever version you've tested against — `:latest` will change under you.
- [ ] Re-seed the RAG knowledge base (`npm run seed-docs`) whenever the corpus changes; the seed script wipes any rows tagged `starter/seed:%` before re-inserting so renamed doc ids don't leave orphans.

**Observability**

- [ ] The agent and backend use pino with credential-path redaction. Route stdout to whatever log system you have.
- [ ] `/api/health` deep-probes upstream KalamDB and caches the answer for `HEALTH_CACHE_MS` (default 2s) to avoid amplifying probes into upstream traffic. Wire it up to `compose healthcheck` / k8s `readinessProbe`.

**Test it**

- [ ] After every change to `chat-app.sql`, `src/agent/`, `server/`, or `src/lib/llm/`, run `npm run typecheck && npm run lint && npm test:unit && npm run test:component`. The e2e (`npm test`) needs the dev stack running.

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
