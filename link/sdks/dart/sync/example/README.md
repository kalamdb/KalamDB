# Offline-first chat example

This Flutter app uses `kalam_sync`, generated `kalam_sync_generator` actions, and
`kalam_link` live events against a real KalamDB server.

- **conversations** are `KalamSyncMode.bidirectional` — creating a chat writes
  locally and queues generic DML.
- **messages** are `replicaOnly` — sends and read receipts go through generated
  durable actions with optimistic overlays. The row becomes `synced` only after
  the backend writes to KalamDB and the live event is committed locally.
- The REST engine writes authoritative user/assistant/receipt rows into KalamDB.
  After `sendMessage`, it inserts a canned AI reply.

Action source of truth: [`../../generator/example`](../../generator/example).

## Ports and credentials

| Service | Default |
| --- | --- |
| KalamDB | `http://localhost:2900` |
| Chat REST engine | `http://127.0.0.1:8787` |
| User | `admin` |
| Password | `kalamdb123` |

These match the Dart SDK e2e helpers in `link/sdks/dart/link/test/e2e/helpers.dart`.
If your server only has `root`, export `KALAMDB_USER=root` and
`KALAMDB_PASSWORD` / `KALAMDB_ROOT_PASSWORD`.

The dart SDK `test.sh` creates the `admin` DBA user when it can log in as root.

## Run

From the repository root:

### 1. Start KalamDB

```bash
cd backend
KALAMDB_ROOT_PASSWORD=kalamdb123 \
KALAMDB_SERVER_PORT=2900 \
KALAMDB_JWT_SECRET=kalamdb-e2e-test-jwt-secret-key!! \
cargo run --bin kalamdb-server
```

If `cargo run` fails to link locally, use an existing debug binary:

```bash
KALAMDB_ROOT_PASSWORD=kalamdb123 \
KALAMDB_SERVER_PORT=2900 \
KALAMDB_JWT_SECRET=kalamdb-e2e-test-jwt-secret-key!! \
./target/debug/kalamdb-server
```

Health check: `curl -sf http://localhost:2900/v1/api/healthcheck`

A fresh server creates `root`, not `admin`. The Dart e2e helpers default to
`admin` / `kalamdb123`. Create that user once:

```sql
CREATE USER 'admin' WITH PASSWORD 'kalamdb123' ROLE dba
```

Or export `KALAMDB_USER=root` and `KALAMDB_PASSWORD=kalamdb123` for every
command below.

### 2. Generate actions (first time, or after editing annotations)

```bash
cd link/sdks/dart/generator/example
flutter pub get
dart run build_runner build --delete-conflicting-outputs
```

### 3. Start the REST engine

The engine applies `kalam/schema.sql` (namespace `chat`) on startup.

```bash
cd link/sdks/dart/sync/example/backend
dart pub get
KALAMDB_URL=http://localhost:2900 \
KALAMDB_USER=admin \
KALAMDB_PASSWORD=kalamdb123 \
dart run bin/server.dart
```

Optional: `KALAM_CHAT_NAMESPACE=chat` `KALAM_CHAT_PORT=8787`

You can also apply the schema yourself:

```bash
curl -u admin:kalamdb123 -X POST http://localhost:2900/v1/api/sql \
  -H 'Content-Type: application/json' \
  --data-binary @link/sdks/dart/sync/example/kalam/schema.sql
```

`POST /v1/api/sql` expects JSON `{"sql":"..."}` for a single statement, so the
engine applies the statements one at a time. Prefer starting `bin/server.dart`.

### 4. Run the Flutter app

```bash
cd link/sdks/dart/sync/example
flutter pub get
flutter create . --project-name kalam_sync_example --platforms=macos
KALAMDB_URL=http://localhost:2900 \
KALAM_CHAT_URL=http://127.0.0.1:8787 \
KALAMDB_USER=admin \
KALAMDB_PASSWORD=kalamdb123 \
flutter run -d macos
```

`flutter create` is only needed once to generate platform runners.

In the app:

- FAB creates a conversation (bidirectional local insert).
- Send a message while online or after tapping the cloud icon to go offline.
- Offline sends stay in the outbox with an optimistic bubble.
- Going back online flushes REST (`persist` then `deliver`) and reconciles the
  live echo to `synced`. An AI demo reply appears on the same conversation.
- Tap an assistant bubble to enqueue `markMsgRead`.

## E2E tests

These talk to a real KalamDB (HTTP + live websocket), the generated action
module, and the REST engine:

```bash
cd link/sdks/dart/sync
flutter pub get
KALAM_INTEGRATION_TEST=1 \
KALAMDB_URL=http://localhost:2900 \
KALAMDB_USER=admin \
KALAMDB_PASSWORD=kalamdb123 \
flutter test test/e2e/chat_sync_e2e_test.dart
```

Use-case tests:

1. `offlineEnqueueThenReconnect` — send while paused; optimistic row; reconnect
   writes through REST and live echo becomes `synced`.
2. `replicaOnlyRejectsDirectMutations` — `messages.insert` throws; send/read go
   through generated actions; server owns delivered/seen.
3. `optimisticUiThenServerEcho` — pending overlay, REST success, then echo to
   `synced` (including delayed echo as `awaitingServerEcho`).
4. `readReceiptsOnlineAndOffline` — mark-read online and offline; a second
   local cache sees `read`; retries are idempotent.
5. `catchUpAfterGapAndBackpressure` — 40 rows while paused; resume from the
   SQLite checkpoint with `batchSize: 10`; no duplicate seqs; acks after commit.

## Layout

```
example/
  lib/main.dart          Flutter chat UI
  backend/               Dart REST engine (writes to KalamDB)
  kalam/schema.sql       conversations + messages USER tables
  README.md
```
