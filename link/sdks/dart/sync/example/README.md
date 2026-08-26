# Offline-first chat example

Advanced demo: `kalam_sync` + generated actions + a small Dart REST engine.

**New to KalamDB?** Use the Flutter starter instead:

```bash
kalam init --yes --languages dart --template simple-live
kalam schema gen --languages dart
kalam dev
```

Docs: [Kalam Sync](https://kalamdb.org/docs/dart-sdk/sync).

This example shows extra pieces the starter does not:

- **conversations** — `KalamSyncMode.bidirectional` (local insert + generic DML)
- **messages** — `replicaOnly` (generated `sendMessage` / `markMsgRead`, backend owns the row)
- A REST engine that writes user/assistant/receipt rows into KalamDB

Action source: [`../../generator/example`](../../generator/example).

## Run

Need a **running** KalamDB (`kalam init` + `kalam dev`, or the [server quick start](https://kalamdb.org/docs/server/getting-started)). Defaults: `http://localhost:2900`, user `root` / `kalamdb123`.

If your server uses `admin` instead of `root`, export `KALAMDB_USER` / `KALAMDB_PASSWORD`.

```bash
# 1. Generate actions (first time, or after editing annotations)
cd link/sdks/dart/generator/example
flutter pub get
dart run build_runner build --delete-conflicting-outputs

# 2. REST engine (applies kalam/schema.sql, namespace chat)
cd ../../sync/example/backend
dart pub get
KALAMDB_URL=http://localhost:2900 \
KALAMDB_USER=root \
KALAMDB_PASSWORD=kalamdb123 \
dart run bin/server.dart

# 3. Flutter app (once: flutter create . --project-name kalam_sync_example --platforms=macos)
cd ..
flutter pub get
KALAMDB_URL=http://localhost:2900 \
KALAM_CHAT_URL=http://127.0.0.1:8787 \
KALAMDB_USER=root \
KALAMDB_PASSWORD=kalamdb123 \
flutter run -d macos
```

In the app: FAB creates a conversation, send works online or after the cloud icon (offline). Offline sends stay in the outbox; reconnect flushes REST and an AI demo reply appears. Tap an assistant bubble to enqueue `markMsgRead`.

## E2E tests (contributors)

These talk to a real KalamDB, the generated action module, and the REST engine:

```bash
cd link/sdks/dart/sync
flutter pub get
KALAM_INTEGRATION_TEST=1 \
KALAMDB_URL=http://localhost:2900 \
KALAMDB_USER=root \
KALAMDB_PASSWORD=kalamdb123 \
flutter test test/e2e/chat_sync_e2e_test.dart
```
