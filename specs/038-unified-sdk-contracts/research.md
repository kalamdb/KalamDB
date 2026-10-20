# SDK parity research and decisions

Source review: 2026-10-18, checkout baseline `af274f6f`, with pre-existing uncommitted SDK changes. Findings below describe inspected source, not runtime verification.

## Current alignment

| Surface | TypeScript today | Dart today | Decision |
| --- | --- | --- | --- |
| Transport | `createClient`, `KalamDBClient`, shared Rust/WASM | `KalamClient`, shared Rust/FRB | Keep implementations; align typed executor and lifecycle contracts |
| Raw SQL | `query`, `queryOne`, `queryAll`, `queryRows` | `query`, typed cell wrappers | Keep compatibility; add shared descriptor execution and cardinality |
| Live rows | `live`, `liveTable`, descriptors, controller | `live`, `liveTable`, streams | Extend current descriptor/controller design into both |
| Framework | React provider/hooks and mutation state | Flutter `KalamScope` in sync | Thin bindings over shared snapshots; no core React/Drift dependency |
| ORM | Drizzle adapter and generated `kTable` schemas | Table specs/local bindings; no corresponding remote table builder | Keep Drizzle in TS; add thin remote table builder in `kalam_link` (no new package) |
| Generated functions | `createKalam(client).namespace.method(input)` | `KalamFunctions(client).namespace.method(input)` | One generated facade vocabulary with compatibility aliases |
| Generation source | ContractSnapshot and ProcedureCatalog | Same source for CLI, separate action annotations | Share semantic metadata/codecs; annotations only for custom application actions |
| Persistence | No equivalent app-sync package inspected | `kalam_sync` cache, outbox, checkpoints in its own Drift file | Deliberate Dart-only extension; new sessions use one Drift database, managed or attached |
| Topic workers | `@kalamdb/consumer` | Not equivalent to sync | Keep topic workers separate from app mirrors/inboxes |

## Evidence and changes required

1. [TS descriptor](../../link/sdks/typescript/client/src/live/descriptor.ts) already separates subscription SQL and local projection. Its regex normalization rewrites whitespace and detects SQL keywords without SQL lexical context. Replace the new generated path with structured plans and use the shared parser for raw validation; do not copy the regex into Dart.
2. [TS controller](../../link/sdks/typescript/client/src/live/controller.ts) has useful snapshot/lifecycle behavior but defaults to a cast when no decoder exists. [Dart client](../../link/sdks/dart/link/lib/src/kalam_client.dart) similarly permits a generic live type without a mapper. Typed new APIs must require generated/explicit codecs.
3. [TS live ORM adapter](../../link/sdks/typescript/orm/src/live.ts) and [React hook](../../link/sdks/typescript/react/src/hooks/useLiveQuery.ts) already provide low-boilerplate paths. Preserve these and add descriptor overloads; do not create a competing React state engine.
4. [TS driver](../../link/sdks/typescript/orm/src/driver.ts) strips quotes/defaults and rewrites file placeholders. Remove obsolete transformations only after semantic fixtures prove server/binder support. Quote preservation and DEFAULT behavior are acceptance gates.
5. [CLI TS generation](../../cli/src/workflow/schema/typescript/schema.rs) emits `.primaryKey()` for `field.primary_key || field.name == "id"`; Dart [table generation](../../cli/src/workflow/schema/dart/tables.rs) chooses `id` or the first field. The shared [contract snapshot](../../backend/crates/kalamdb-dialect/src/contracts/snapshot.rs) already contains field primary-key/default flags, but needs explicit ordered composite keys and richer default/identity/cardinality metadata.
6. [CLI type mappings](../../cli/src/workflow/schema/types.rs) and [Dart codecs](../../cli/src/workflow/schema/dart/rows.rs) differ for numeric, JSON, temporal, file, and array values. Type declarations and encoders/decoders must share one semantic type tree.
7. [TS function template](../../cli/templates/schema-gen/typescript/src/generated/kalam.ts.template) casts the extracted result; [Dart template](../../cli/templates/schema-gen/dart/lib/generated/kalam.dart.template) decodes but collapses errors and imports sync. Split output and use shared typed result/error rules.
8. [TS schema generation](../../cli/src/workflow/schema/typescript/schema.rs) and [ORM introspection generation](../../link/sdks/typescript/orm/src/generate.ts) are distinct emitters. Route project generation through one semantic manifest; keep introspection as a manifest input mode, not a second type policy.
9. [OpenAPI export](../../cli/src/workflow/schema/api_exports/openapi.rs) currently emits named composite schemas, generally string scalar mappings, and no operation paths. The real [function endpoint](../../backend/crates/kalamdb-api/src/http/functions.rs) exists separately. Full operation exports need the same routine/validation contract.
10. [Dart bridge config](../../link/kalam-link-dart/src/models.rs) sets native `version_domain` to `None`; TS exposes a domain option. Both public checkpoint types still expose a last version rather than a complete durable replay contract. Preserve actual server metadata end to end before promising replay.
11. [Sync table binding](../../link/sdks/dart/sync/lib/src/tables/kalam_table_binding.dart) mixes external callbacks with default cached-row cleanup and infers snapshot state from rows. [Transport](../../link/sdks/dart/sync/lib/src/transport/kalam_link_transport.dart) drops batch completion metadata. Use explicit generations and projection adapters.
12. [Sync store](../../link/sdks/dart/sync/lib/src/store/kalam_sync_store.dart) watches historical actions and loads eligible candidates into lists; interrupted-work recovery can reclaim running work without a persistent owner lease. Add bounded indexed scheduling, status aggregates, and leases.
13. [Action generator](../../link/sdks/dart/generator/lib/src/kalam_action_generator.dart) returns action records; [payload generator](../../link/sdks/dart/generator/lib/src/kalam_action_payload_generator.dart) has an independent small type set. Reuse generated operation codecs and typed result handles.
14. Masky uses local path overrides to these Dart packages. Its `lib/services/sync/kalam_sync_service.dart`, `kalam_sync/masky_kalam_session.dart`, `background_batch_sync.dart`, and `kalam_sync/projection_recovery.dart` are migration targets. Message decoding, missing-conversation fetches, and mailbox handlers can return successfully after errors; source callbacks and SDK progress need an explicit retry/rejection contract.

## Lessons from other SDKs

These are design inputs, not claims that their guarantees equal KalamDB's.

| Source | Useful pattern | Application here |
| --- | --- | --- |
| [Drizzle select](https://orm.drizzle.team/docs/select) and [insert](https://orm.drizzle.team/docs/insert) | Typed composable queries and separate insert inference | Preserve Drizzle; give Dart the same small table-operation vocabulary without cloning the entire library |
| [Drift managers](https://drift.simonbinder.eu/dart_api/manager/) | Generated filters, `get`, `watch`, typed rows | Generate Dart managers and optional local adapters; keep remote PostgreSQL and local SQLite execution distinct |
| [Drift SQL files](https://drift.simonbinder.eu/sql_api/drift_files/) and [sqlc generation](https://docs.sqlc.dev/en/latest/howto/generate.html) | SQL analysis generates typed parameters/results | Compile named queries through Kalam's authoritative SQL pipeline and emit both languages |
| [Convex function references](https://docs.convex.dev/api/modules/server) | Generated references carry arguments and return types | Use operation descriptors for immediate calls and eligible durable calls |
| [tRPC input/output validators](https://trpc.io/docs/server/validators) | Validation and type inference share procedure definitions | Generate codecs and handler boundaries from contracts; static assertions are insufficient |
| [Firestore snapshot metadata](https://firebase.google.com/docs/reference/js/firestore.snapshotmetadata) | Cached-source and pending-write metadata | Add completeness/freshness metadata to live and persisted snapshots |
| [Supabase Dart streams](https://supabase.com/docs/reference/dart/stream) | Simple initial-data-plus-change subscription | Keep subscription setup small; explicitly test filter exits, which its docs identify as a limitation |

## Chosen tradeoffs

- Canonical facade: `createKalam(client)` in both languages, with `functions`, `queries`, and `operations` namespaces (plus `tables` in Dart). Existing TS direct namespace methods and Dart `KalamFunctions` delegate during migration.
- TypeScript table queries stay Drizzle, including the schema `kalam dev` generates. Dart's thin remote table builder lives in `kalam_link` (no separate `kalam_orm` package, no Drift or persistence in `kalam_link`), and no second TypeScript builder is added beside Drizzle. Do not add SeaORM, Diesel, SQLx, or another ORM to `link-common`. The shared Rust client keeps transport, execution, and live materialization. `kalam_sync` uses Drift for the one local SQLite database, its migrations, and local queries.
- One SQLite connection per Dart account session. Default: Kalam opens the Drift database and returns it so the app can add tables and migrations through its own Drift schema on that connection. Alternative: the app passes an already opened Drift database that includes the sync tables. Foreground and background share that connection. A second SQLite file is reserved for a destination that cannot use this connection.
- Match semantic types exactly, use language-native representations where lossless, and named SDK scalar types where native types lose information.
- One shared Rust live/materialization implementation where possible; persistent mirrors own durable membership and transactions. Do not run two full materializers for one mirror.
- Durability is opt-in and visible. The default TS client does not gain SQLite/IndexedDB/outbox dependencies.
- New API layout uses generator API version 2. Existing projects retain their declared version until migration; compatibility is temporary, with one runtime implementation.
- A full server transaction API, arbitrary offline relational engine, automatic business conflict resolution, and arbitrary live joins are outside this feature.
