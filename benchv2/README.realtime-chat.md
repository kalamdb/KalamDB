# Realtime chat benchmark

`chat_realtime` is a timed load scenario that imitates a real chat app (Masky-style): personal AI threads on USER tables, and 2- or 3-person rooms on SHARED tables protected by `CREATE POLICY`.

It is **not** a microbenchmark of a single SQL statement. It logs in real users, keeps live subscriptions open, writes messages, waits for delivery, occasionally reads history, and sometimes disconnects and comes back.

Run it with:

```bash
./run-chat-realtime.sh
./run-chat-realtime.sh --url http://127.0.0.1:2900 --minutes 10 --users 1000 --realtime-convs 150
./run-chat-realtime.sh --ai-convs 50 --shared-convs 50 --triple-convs 10
./run-chat-realtime.sh --messages-per-minute 0   # idle subscriptions only
```

Reports land in `results/chat-runtime/`.

## What it is modeling

A product with two conversation kinds:

| Kind | Product analog | Storage |
| --- | --- | --- |
| AI thread | User talking to an agent | USER tables: one copy per user |
| Human room | Masky / group chat with 2 or 3 people | SHARED tables + membership RLS |

Both kinds also cover everyday client behavior:

- Open a conversation and stay subscribed
- Send messages and see the other side appear live
- Scroll history *before a cutoff date*
- Close the chat, miss traffic, reconnect, replay, then reply
- Re-open a conversation later with `SELECT` (not only live push)

## Schema

Setup creates one namespace and these objects:

```text
USER    conversations_ai     personal AI threads
USER    messages_ai          user + assistant rows in those threads
SHARED  conversations        human rooms
SHARED  conversation_members who is in each room  (PK: '{user}:{conversation_id}')
SHARED  messages             room transcript
STREAM  typing_events        short-lived AI typing (TTL 30s)
TOPIC   chat_ai_inbox        every INSERT into messages_ai
```

Shared-table policies (the interesting part for human rooms):

- Anyone signed in can create a room.
- You can insert only your own membership row (`user_id = CURRENT_USER`).
- You can select a room only if you are a member.
- You can select or insert messages only for rooms you belong to.

That is the same shape as `examples/chat-with-ai`: one shared transcript, RLS decides which rows each user sees. There is **no** topic forwarder between human users. They all read the same table.

## Default mix

For `--realtime-convs 100` (the default):

| Workers | Count | What each worker is |
| --- | --- | --- |
| AI | 50 | one user + the topic agent |
| Shared pair | 40 | two users in one RLS room |
| Shared triple | 10 | three users in one RLS room |

Overrides:

| Flag / env | Meaning |
| --- | --- |
| `--users` / `KALAMDB_BENCH_CHAT_USERS` | Seeded KalamDB users (default 1000) |
| `--realtime-convs` / `KALAMDB_BENCH_CHAT_REALTIME_CONVS` | Total concurrent conversations (default 100) |
| `--ai-convs` / `KALAMDB_BENCH_CHAT_AI_CONVS` | AI workers (default half of total) |
| `--shared-convs` / `KALAMDB_BENCH_CHAT_SHARED_CONVS` | Human-room workers |
| `--triple-convs` / `KALAMDB_BENCH_CHAT_TRIPLE_CONVS` | How many of those rooms are 3-person (default 20% of shared) |
| `--minutes` / `KALAMDB_BENCH_CHAT_MINUTES` | Run length (default 5) |
| `--messages-per-minute` / `KALAMDB_BENCH_CHAT_MESSAGES_PER_MINUTE` | Target rate per conversation (default 20). `0` = subscribe and idle |
| `--mutation-every-messages` / `KALAMDB_BENCH_CHAT_MUTATION_EVERY_MESSAGES` | Alternate an UPDATE and DELETE every N inserted user messages (default 10). `0` disables mutations |

Only as many users as the mix needs are logged in (an AI room uses 1, a pair uses 2, a triple uses 3). Extra seeded users exist so logins can fail over during prewarm.

## Run sequence

1. **Setup** — create namespace, drop leftover objects from older bench versions, create tables/policies/topic, create users.
2. **Prewarm** — log in the active users (capped concurrency).
3. **Start the AI inbox agent** — one topic consumer for all AI threads.
4. **Spawn workers** — one tokio task per conversation, staggered by a few milliseconds.
5. **Run until the deadline** — each worker loops at the configured message rate, runs history/reopen queries, and alternates UPDATE/DELETE for every configured Nth message.
6. **Stop** — close subscriptions, stop the agent, print a summary, tear down schema and users.

## Path 1: AI conversation (USER tables)

This is a personal assistant thread, not two humans talking through USER-table mirroring.

```text
user tab                         topic agent
--------                         -----------
INSERT conversations_ai
SUBSCRIBE messages_ai
SUBSCRIBE typing_events
INSERT messages_ai role=user  →  chat_ai_inbox
                                 ignore assistant rows
                                 INSERT typing_events AS USER  (×2)
                                 INSERT messages_ai role=assistant AS USER
live INSERT (typing + reply)  ←
```

Each cycle:

1. The user inserts a `role='user'` row into **their** `messages_ai`.
2. The agent consumes that INSERT from `chat_ai_inbox`.
3. It writes two STREAM typing rows as that user, then an assistant reply into the same user’s `messages_ai` via `EXECUTE AS USER`.
4. The user’s live subscription must see the assistant row (and typing, except on reconnect cycles).

The agent never talks to SHARED tables. Assistant INSERTs are skipped so it does not reply to itself.

The inbox consumer processes replies with bounded concurrency, matching a service that can handle
multiple independent AI conversations at once while preventing an unbounded task backlog.

## Path 2: Human room (SHARED + RLS)

This is the Masky path: 2 or 3 real users, one shared transcript.

```text
user A                         user B (+ optional user C)
------                         --------------------------
INSERT conversations
INSERT conversation_members    INSERT conversation_members
SUBSCRIBE messages             SUBSCRIBE messages
                               WHERE conversation_id = this room

INSERT messages                live INSERT (RLS: only this room)
                               INSERT messages
live INSERT
```

Each cycle, members take turns sending. After A writes, B (and C) must see that row on their live subscription before the next turn. There is no copy into another user’s table: RLS filters the same `messages` table.

A non-member would see zero rows even with `OR 1=1` in their own `WHERE` clause. The bench does not try to bypass that; it only drives authorized members.

## History, reconnect, and “open this chat later”

Every worker, on both paths:

| Every N cycles | What happens | Why |
| --- | --- | --- |
| 4 | `SELECT … WHERE conversation_id = ? AND created_at_ms < cutoff ORDER BY created_at_ms DESC LIMIT 50` | Scroll history before a date (cutoff is the midpoint between room creation and now) |
| 6 | Close live subs → send or receive while gone → wait 2s → resubscribe **with initial snapshot** → `SELECT` the conversation + latest messages | Leave the chat, come back, replay, then continue |

AI reconnect specifically: the user unsubscribes, sends a message while offline, the agent still replies into the USER table, then the user resubscribes with initial data so the missed assistant row arrives in the snapshot.

Shared reconnect specifically: member 0 unsubscribes, member 1 posts into the shared table, member 0 resubscribes with snapshot, must see the missed row, then replies.

Reconnect closes **that conversation’s subscriptions**, not the shared WebSocket for the whole user. One person can be in several rooms on the same login.

`--messages-per-minute 0` skips sending. Workers only hold subscriptions for the run duration (capacity / idle-fanout check).

## What “success” looks like

Workers keep running if a live wait times out; timeouts are counted instead of failing the whole bench. The process fails on hard errors (login, DDL, subscribe, agent decode, join panics) and when the six equal-time run stages show rising SQL latency or RSS after warm-up rather than a flat line.

The printed summary is the useful result:

- **Chat mix** — how many AI / pair / triple workers ran
- **Rates** — completed cycles, user messages, typing, SQL, live change rows
- **Path split** — AI user messages vs AI replies vs shared-room messages
- **SQL breakdown** — run-total select/insert/update/delete rates and latency
- **Run stages** — six equal slices of the run. Stage 1 is warm-up. Each later stage shows query counts, sel/ins/upd/del rates, live/history/reconnect activity, RSS, and whether sql/latency/live/rss rose (`↑`), held (`→`), or fell (`↓`) versus the previous stage
- **App behaviors** — historic SELECTs, conversation reopens, reconnect replays
- **Delivery diagnostics** — timed-out waits and AI reply gap (`user messages − agent replies`)
- **Timing percentiles** — subscribe open, create conversation, insert/update/delete message, insert typing, historic select, reconnect
- **Managed server RSS** — only when the runner started the server (`KALAMDB_BENCH_MANAGED_SERVER=1`)

A healthy AI path has a small reply gap. A healthy shared path delivers peer messages without a topic mirror. Historic and reconnect counters should be non-zero on a multi-minute run.

## Code map

| File | Role |
| --- | --- |
| `run-chat-realtime.sh` | CLI wrapper |
| `src/chat_runtime/chat_realtime_bench.rs` | Benchmark trait, schema, worker spawn |
| `src/chat_runtime/ai.rs` | Topic agent + AI worker |
| `src/chat_runtime/shared.rs` | 2- and 3-user RLS rooms |
| `src/chat_runtime/common.rs` | Settings, user pool, subscriptions, stats |

Suite name is `chat-runtime`; the benchmark name is `chat_realtime`.
