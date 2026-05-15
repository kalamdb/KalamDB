# KalamDB Chat Starter

A production-ready AI chat skeleton built on **KalamDB live queries**, **React**, and a **pluggable LLM adapter**. Every real-time behavior in the app  typing indicators, message streaming, human-in-the-loop approvals, even the **Stop** button  is driven by `<LiveQuery>` subscriptions. There is no WebSocket-glue code, no polling, no manual event bus.

## Why this is interesting

- **Streaming tokens** are rows in a `typing_tokens` table. The client subscribes; the server worker INSERTs. No SSE/WebSocket plumbing on top.
- **Approvals** (human-in-the-loop) are rows in an `approvals` table. The agent inserts a row, the UI sees it live, the user clicks Approve, the agent's own subscription wakes up.
- **Stop button** is just an UPDATE on the `tasks` table. The agent subscribes to its OWN task row and aborts mid-stream when `is_cancelled` flips to true. This is the live-query-as-control-plane pattern that's hard to demo in any other stack.

## Quick start

```bash
cp .env.example .env       # add your OPENAI_API_KEY
docker compose up -d       # boots a local KalamDB on :8080
npm install
npm run setup              # creates the schema
npm run agent              # starts the server-side worker
npm run dev                # starts the Vite app on :5173 (separate terminal)
```

Open the Vite URL, click **New chat**, send a message. Watch the assistant reply stream in token-by-token. While it's streaming, hit **Stop**  the reply truncates immediately, marked `(stopped)`.

## Architecture

```
+---------+         live subscriptions          +-----------+
|         | <----------------------------------> |           |
|  React  |    conversations / messages /        |  KalamDB  |
|   UI    |    typing_tokens / approvals /       |           |
|         |    tasks                             +-----^-----+
+---------+                                            |
                                                       | live
                                                       |
                                                +------+------+
                                                |  Node agent |
                                                |  worker     |
                                                +-------------+
```

The agent worker is just a Node process that:
1. Subscribes to `tasks WHERE is_cancelled = false AND finished_at IS NULL` (the work queue)
2. For each task, also subscribes to its own row (the cancellation channel)
3. Streams from the LLM, INSERTs each token as a `typing_tokens` row
4. On stream end OR cancel: UPDATEs the message body + status, deletes the streamed tokens

## Tables

| Table | Purpose |
|---|---|
| `chat.conversations` | Top-level chat threads (sidebar) |
| `chat.messages` | User + assistant turns; `status` tracks `pending` -> `streaming` -> `final` / `cancelled` / `error` |
| `chat.typing_tokens` | One row per LLM delta. Cleared when the message is finalised. |
| `chat.approvals` | Human-in-the-loop checkpoints. Status flips when the user clicks Approve/Reject. |
| `chat.tasks` | In-flight agent work. The agent subscribes to its own row for cancellation. |

## LLM providers

Pluggable adapter at `src/lib/llm/`. Ships with **OpenAI** and **Anthropic** implementations. To add another (Bedrock, local Ollama, etc.): implement the `LlmAdapter` interface in a new file and wire it in `src/lib/llm/index.ts`.

```ts
export interface LlmAdapter {
  readonly name: string;
  stream(messages: LlmMessage[], signal: AbortSignal): AsyncGenerator<LlmStreamChunk>;
}
```

Streaming returns `AsyncGenerator` so the agent can interleave token deltas with cancellation checks naturally.

## Production checklist

This starter ships with permissive defaults so you can run it locally in seconds. Before going to prod:

- [ ] Replace the hardcoded `kalamdb-dev-password` with a real credential, ideally per-user via your auth system
- [ ] Tighten `KALAMDB_SECURITY_CORS_ALLOWED_ORIGINS` to your actual origin
- [ ] Run multiple agent worker instances (the task table can be claimed safely with `is_cancelled = false` filtering; add a `claimed_by` column if you want stricter ownership)
- [ ] Add a `system_prompt` column on `conversations` if you want per-thread personalities
- [ ] Add error-recovery on the agent: retry transient LLM errors, surface non-retryable ones as `status='error'` messages

## License

Apache-2.0 (matches KalamDB).
