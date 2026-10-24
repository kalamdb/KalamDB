# kalam_link

Official Dart and Flutter client for [KalamDB](https://kalamdb.org): SQL over HTTP, live rows over WebSocket, and JWT/OIDC auth.

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

`KalamClient.init(timeout: ...)` retries after a failed or timed-out load. `KalamClient.isSupportedPlatform` is true on Android and iOS.

Remote table reads use the same `query` and `live` calls. Bind a `KalamRemoteTable`, then `query().where(...).limit(...).get()`. `live()` sends the filter only. TypeScript table access stays on Drizzle.

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

`Auth.basic(user, password)` exchanges credentials for a JWT on connect. Call `refreshAuth()` when the token rotates.

For Keycloak or any KalamDB-configured OIDC issuer, obtain an ID token in the app (for example with `flutter_appauth`), then exchange it for KalamDB tokens:

```dart
final session = await client.exchangeOidcToken(idToken);
```

The client then uses the returned KalamDB access token for SQL, files, and live rows. See [Authentication](https://kalamdb.org/docs/dart-sdk/auth) and [Keycloak](https://kalamdb.org/docs/server/integrations/keycloak).

## Procedures

Do not call procedures through a generic string API on `KalamClient`. Run `kalam schema gen --languages dart` and use the generated typed client, which wraps `query('CALL ...')`:

```dart
final db = KalamFunctions(client);
final user = await db.chat.createMessage(
  ChatCreateMessageRequest(userId: 'u1', body: 'hello'),
);
```

Edit `schema.sql`, regenerate, and keep using the generated types. REST `POST /v1/functions/...` is a server HTTP path, not a Dart SDK method.

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

## Generated typed remote queries

After `kalam schema gen --languages dart`, reuse the generated table codecs and
primary key directly:

```dart
final inventory = KalamTables.inventory.bind(client);
final query = inventory.query()
    .filter(InventoryColumns.sku.eq('sku-1'))
    .limit(20);
final rows = await query.get();
final updates = query.live();
```

`InventoryColumns.sku` is `KalamColumn<String>`, so passing a number fails Dart
analysis. Existing `.where((c) => c.col('sku').eq(value))` remains available for
dynamic queries. `eq(null)` and `neq(null)` produce `IS NULL` and `IS NOT NULL`
when the column type permits null. Values use bound parameters.

`orderBy` on a remote live query sorts the matching rows locally before applying
`limit`. The limit controls the displayed rows, not retained memory. Use a narrow
filter for large datasets. Scalar string ordering is local and does not promise
server collation equivalence. Use `kalam_sync` for durable local projections and
offline actions; `bind(client)` uses remote transport and does not add persistence.

Generated Dart table specs require one declared primary key. Keyless tables still
get row codecs, but no sync spec or guessed key. Composite primary keys are rejected
by schema compilation to match current server support. Array codecs preserve null
arrays and encode/decode each supported scalar, enum, composite, and timestamp
element; null array elements are not represented by the current contract.
