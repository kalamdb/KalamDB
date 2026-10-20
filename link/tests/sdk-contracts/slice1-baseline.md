# Slice 1 baseline

Recorded before the Slice 1 helpers landed. These entry points stay. Slice 1 adds the Dart table builder and the session helpers below; it does not add generator API v2, live replay cursors, a new sync schema, or server idempotency.

## Public SDKs that must keep compiling

- `@kalamdb/client`: `createClient`, `query`, `live`, `liveEvents`, `LiveQueryController`
- `@kalamdb/orm`: `kTable`, `kalamDriver`, `liveTable`, `kalamFile`, `executeAsUser`, `queryWithFiles`
- `@kalamdb/react`: `useLiveQuery`, `useLiveQueries`, `useMutationState`
- `kalam_link`: `KalamClient.init`, `KalamClient.connect`, `query`, `queryWithFiles`, `live`, `liveTable`, `liveEventsWithAck`
- `kalam_sync`: `Kalam.open`, `Kalam.catchUp`, `KalamTableSpec`, `KalamEventConsumer.consume`, `KalamActionContext.step`

## Masky glue Slice 1 replaces

- `app/lib/services/sync/kalam_sync/masky_kalam_specs.dart` — qualified subscription SQL
- `app/lib/services/sync/kalam_sync/kalam_decode.dart` — cell map and partial update flag
- `app/lib/services/sync/kalam_sync/projection_recovery.dart` — checkpoint and mailbox reset
- `app/lib/services/sync/kalam_sync/masky_kalam_session.dart` — checkpoint seed
- `app/lib/services/sync/kalam_sync/kalam_runtime.dart` — init timeout and platform check
- `app/lib/services/sync/kalam_sync_service.dart` — session plan, auth recovery, connection debounce
