# kalam_link examples

Protocol demos against a **running** KalamDB. They create sample tables from Dart so you can inspect queries, login, and `liveEvents()`.

**New Flutter app?** Prefer `kalam init --languages dart` and [`kalam_sync`](https://kalamdb.org/docs/dart-sdk/sync). Server install is not part of this SDK — see the [server quick start](https://kalamdb.org/docs/server/getting-started).

```bash
# from link/sdks/dart/link
dart run example/simple-events/main.dart
dart run example/chat-app/main.dart
```

Defaults: `http://localhost:2900`, user `admin` / `kalamdb123`. Override with `KALAMDB_URL`, `KALAMDB_USER`, `KALAMDB_PASSWORD` (local `kalam init` servers use `root` / `kalamdb123`).
