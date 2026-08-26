# kalam_link

Official Dart and Flutter client for [KalamDB](https://kalamdb.org): SQL over HTTP, live rows over WebSocket, and JWT auth.

For a local-first Flutter app (offline cache, optimistic writes), use [`kalam_sync`](https://pub.dev/packages/kalam_sync) instead.

→ **[Docs](https://kalamdb.org/docs/dart-sdk)** · [Setup](https://kalamdb.org/docs/dart-sdk/setup) · [Kalam Sync](https://kalamdb.org/docs/dart-sdk/sync) · [GitHub](https://github.com/kalamdb/KalamDB)

## Install

```bash
flutter pub add kalam_link
```

Point the client at a running KalamDB. Server install and first-user setup are in the [server quick start](https://kalamdb.org/docs/server/getting-started) and [`kalam init`](https://kalamdb.org/docs/cli/init).

## Get started

```dart
import 'package:flutter/widgets.dart';
import 'package:kalam_link/kalam_link.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await KalamClient.init();
  runApp(const MyApp());
}
```

Connect after the first frame — not before `runApp()`:

```dart
final client = await KalamClient.connect(
  url: 'http://localhost:2900',
  authProvider: () async => Auth.basic('root', 'kalamdb123'),
);

final result = await client.query(
  r'SELECT id, title FROM app.tasks WHERE done = $1',
  params: [false],
);

for (final row in result.rows) {
  print('${row['id']?.asInt()} ${row['title']?.asString()}');
}

final tasks = client.liveTable<Map<String, KalamCellValue>>('app.tasks');
await for (final rows in tasks) {
  print('rows=${rows.length}');
  break;
}

await client.dispose();
```

Live SQL must stay `SELECT ... FROM ... WHERE ...`. Prefer `live()` / `liveTable()` for UI lists; use `liveEvents()` when you need raw insert/update/delete events.

## Auth

```dart
authProvider: () async => Auth.jwt(await tokens.freshAccessToken()),
```

`Auth.basic(user, password)` exchanges credentials for a JWT on connect. Call `refreshAuth()` when the token rotates. See [Authentication](https://kalamdb.org/docs/dart-sdk/auth).

## Local-first Flutter

```dart
import 'package:kalam_sync/kalam_sync.dart';

final kalam = await Kalam.open(
  url: 'http://localhost:2900',
  subject: signedInUser.id,
  authProvider: () async => Auth.jwt(await tokens.freshAccessToken()),
);

runApp(KalamScope(kalam: kalam, child: const App()));
```

Docs: [Kalam Sync](https://kalamdb.org/docs/dart-sdk/sync).

## Examples

Protocol demos (need a running server; defaults `root` / `kalamdb123`):

```bash
dart run example/simple-events/main.dart
dart run example/chat-app/main.dart
```

New Flutter apps: `kalam init --languages dart`. See [`example/README.md`](example/README.md).

## License

Apache-2.0. See [LICENSE.txt](LICENSE.txt).
