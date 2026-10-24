# SDK correctness and developer-experience review

Reviewed 2026-10-19 against checkout `af274f6f` plus the current working-tree changes. This is a review, not a runtime implementation. Existing user edits were preserved. The updated specification explicitly makes the current work **Slice 1**; absent Slice 2 replay/idempotency APIs are not counted as newly introduced defects.

**Verdict: request changes before marking Slice 1 complete.** The thin Dart builder and shared application Drift database are useful additions, but reproducible query/action defects remain. TypeScript and Dart also have existing generation and live-query problems that block the broader type-safety/parity goal.

## Findings requiring fixes

### R1 — P1: Chained Dart filters lose boolean grouping — new

[Query compilation](/Users/jamal/git/KalamDB/link/sdks/dart/link/lib/src/table/kalam_remote_table.dart:216) joins rendered predicates with `AND` without enclosing each predicate. Consequently:

```text
where(owner = me OR public = true).where(deleted = false)
```

becomes `(owner = me) OR (public = true) AND deleted = false`. Owner-matching deleted rows pass the filter. The same issue affects live SQL. Bound values prevent SQL injection here, but do not preserve boolean semantics.

**Fix:** compile the predicate tree with explicit grouping at every composition boundary. Verify actual returned rows for chained/nested AND/OR expressions, not just placeholder numbering. Reproduced in `remote_probe_test.dart`.

### R2 — P1: Duplicate action IDs silently accept different work or another account — new

[Enqueue deduplication](/Users/jamal/git/KalamDB/link/sdks/dart/sync/lib/src/store/kalam_sync_store.dart:33) looks up only `id` and returns the existing record without comparing account, action definition, or payload.

Reproductions:

- Enqueue ID `stable-id` with text `one`, then text `two`: the second call succeeds with the first action; the second intent is lost.
- In one shared database, account A and account B enqueue `mailbox:1`: account B receives account A's record.

The default per-account database limits the second case, but the new attached-database API and public store do not enforce that separation. This can be fixed locally; it does not require Slice 2 server idempotency.

**Fix:** scope identity to account and require semantic equality for a duplicate, or reject collisions explicitly. Never return a different account's record. Preserve the documented replay behavior for identical work. Reproduced in `sync_probe_test.dart`.

### R3 — P1: Live ordering is wrong in both languages — new Dart path, existing TS paths

- [Dart live](/Users/jamal/git/KalamDB/link/sdks/dart/link/lib/src/table/kalam_remote_table.dart:263) drops SQL ordering for subscription compatibility but never applies `_orders` locally. Input `[1,3,2]` remains `[1,3,2]` for descending order.
- [TS direct liveTable](/Users/jamal/git/KalamDB/link/sdks/typescript/orm/src/live.ts:134) explicitly discards `orderBy`; it computes a descriptor but never executes its projection.
- [TS descriptor projection](/Users/jamal/git/KalamDB/link/sdks/typescript/orm/src/live.ts:121) contains wire name `created_at`, while its mapper returns `createdAt`. The controller sorts mapped rows by an absent property. The result again remains `[1,3,2]` instead of `[3,2,1]`.

**Fix:** make all entry points execute one projection contract with correct wire/public-name mapping. Apply exact ordering before presentation limits; keep retention limits distinct. Test delete/promotion of the next row in an ordered top-N result. Reproduced with both portable Dart and TS probes.

### R4 — P1: Generated array types and their runtime/ORM representations disagree — existing

Freshly generated both targets with the newly rebuilt CLI from `schema.sql` in this directory. It contains an ordinary `tags TEXT[]` column.

- [Dart decoder generation](/Users/jamal/git/KalamDB/cli/src/workflow/schema/dart/rows.rs:114) produces `List<String>? tags` but initializes it using `_asStringOrNull(...)`. The Dart analyzer reports `String?` cannot be assigned to `List<String>?`.
- [TS Drizzle generation](/Users/jamal/git/KalamDB/cli/src/workflow/schema/typescript/schema.rs:369) emits `tags: text("tags")`, while the generated `Inventory` type says `string[] | null`. Assigning `inventory.$inferSelect` to `Inventory` fails compilation.

**Fix:** recursively derive declarations, codecs, and ORM builders from the same field shape. Generate array builders/element codecs or explicitly reject unsupported arrays. Compile emitted code and assert ORM/model agreement; current string-fragment generator tests do not catch this.

### R5 — P1: Generated keys are inferred from the name `id`, contradicting the schema — existing

For `id TEXT NOT NULL, sku TEXT PRIMARY KEY`, the fixture produces:

- Dart `keyColumn: 'id'` and a row identity based on that non-key field, from [key_column_name](/Users/jamal/git/KalamDB/cli/src/workflow/schema/dart/tables.rs:53).
- TS `.primaryKey()` on **both** `id` and `sku`, from [drizzle_column](/Users/jamal/git/KalamDB/cli/src/workflow/schema/typescript/schema.rs:389).

This is misleading metadata and can merge distinct live/cache rows or address the wrong row. Real primary-key flags already exist in the contract; single-key correctness should not wait for a whole new manifest format.

**Fix:** use catalog-declared keys, preserve composite-key order, and reject key-dependent operations when no supported key exists. Include a non-key column named `id` in the regression fixture.

### R6 — P2: TS raw-live preparation changes SQL string values — existing

[normalizeSqlText](/Users/jamal/git/KalamDB/link/sdks/typescript/client/src/live/descriptor.ts:136) collapses whitespace over the entire SQL string. `WHERE body = 'a  b'` becomes `WHERE body = 'a b'`, selecting different data.

**Fix:** preserve literals/quoted identifiers through authoritative SQL parsing or structured descriptors. Do not repair this with another broad regex rewrite. Reproduced in `ts_probe.test.mjs`.

### R7 — P2: New Dart FILE naming differs from the server — new

[Dart filename sanitation](/Users/jamal/git/KalamDB/link/sdks/dart/link/lib/src/file_ref.dart:245) truncates after collapsing dashes and lowercases before ASCII filtering. The canonical [Rust helper](/Users/jamal/git/KalamDB/backend/crates/kalamdb-commons/src/models/file_ref.rs:233) filters/truncates before collapsing dashes. Extension handling also differs.

Executed exact extracted helper functions from both sources:

| Input | Rust/server suffix | Dart suffix |
| --- | --- | --- |
| `a` + 60 spaces + `b.txt` | `a.txt` | `a-b.txt` |
| `file.` | `file.` | `file.bin` |

The same stored file can therefore produce a different download path in Dart. Removing FFI from a simple helper is useful, but shared behavior must remain exact.

**Fix:** match canonical semantics and share filename vectors across Rust/TS/Dart, including long separators, trailing dots, Unicode, and empty base names. The added full Flutter FILE probe was not run after the SDK cache became unwritable; the exact-helper comparison was executed successfully.

### R8 — P2: Checkpoint seeding is not atomic — new

[seedCheckpointIfAbsent](/Users/jamal/git/KalamDB/link/sdks/dart/sync/lib/src/store/kalam_sync_store.dart:465) checks absence outside the write transaction, then calls an operation allowed to advance an existing checkpoint. Concurrent seeds at 4 and 9 both return `true`; the second seed can overwrite a checkpoint that was absent only at the earlier read.

**Fix:** perform an insert-if-absent operation atomically and return whether it inserted. A seed must not advance an existing checkpoint. Reproduced in `sync_probe_test.dart`.

### R9 — P2: The immutable Dart query retains mutable input lists — new

[inList](/Users/jamal/git/KalamDB/link/sdks/dart/link/lib/src/table/kalam_remote_table.dart:299) captures the caller's list. Building a query with `['a']` and then appending `'b'` changes the already-built query. Post-construction mutation can also bypass the 100-element validation.

**Fix:** copy/freeze predicate inputs and validate the captured definition. Apply the same ownership policy to order/group lists and exposed compiled parameters. Reproduced in `remote_probe_test.dart`.

### R10 — P2: Old TS error/checkpoint callbacks can mutate a disposed/replaced controller — existing

The row callback checks `startToken`, but [onError/onCheckpoint](/Users/jamal/git/KalamDB/link/sdks/typescript/client/src/live/controller.ts:137) do not. A late error after disposal changes status from `disposed` to `error`; an old subscription can similarly overwrite a new generation's state.

**Fix:** capture/check the generation in every callback, and make disposal terminal for that handle. Reproduced in `ts_probe.test.mjs`.

### R11 — P2: liveSql uses subscription identity as the table name — new, source-confirmed

[KalamTableSpec.liveSql](/Users/jamal/git/KalamDB/link/sdks/dart/sync/lib/src/tables/kalam_table_spec.dart:26) uses `subscriptionId`. The public constructor/copyWith intentionally allow that identifier to differ from `tableId`. A spec for `public.messages` with subscription ID `unread-messages` therefore generates `SELECT * FROM unread-messages`.

**Fix:** build quoted SQL from table identity; keep subscription/checkpoint identity separate. Add a custom-subscription-ID and quoted-identifier test. This finding was verified from the direct expression and constructor contract, not a runtime server test.

## Developer experience: improvements with the highest return

### 1. Generate the Dart builder instead of making users define it

The new builder currently types only the decoded result. `col(String)`, `eq(Object?)`, `get(Object?)`, and `insert/update(Map<String,Object?>)` accept misspelled columns, wrong key types, and invalid values. Applications still define table/key/decoder metadata themselves. The generator emits `KalamTableSpec`, not a typed remote builder.

Generate typed columns plus separate row/key/insert/patch types, reuse the existing row codecs, and expose a generated binding method. Keep string/map entry points as explicit escape hatches. A wrong column name/value should fail analysis; omission and explicit null must remain distinct.

**Proposed convenience, not implemented today:**

```dart
final messages = KalamTables.messages.bind(client);
final recent = messages.query()
    .where((m) => m.conversationId.eq(conversationId))
    .orderBy((m) => m.createdAt.desc())
    .limit(50);
final rows = await recent.get();
final updates = recent.live();
```

Generate this binding around the existing runtime; do not require a handwritten decoder or a replacement TS ORM. Preserve normal TS Drizzle queries and adapt them to the same live semantics.

### 2. Keep Drizzle idiomatic; align outcomes and shared definitions

Drizzle already infers selected rows/nullability and parameterizes its filters. Drift's generated managers demonstrate how Dart can expose typed filtering and observation. Borrow those patterns without copying the full ORM or pretending SQLite and remote PostgreSQL are interchangeable. Sources: [Drizzle select](https://orm.drizzle.team/docs/select), [Drift managers](https://drift.simonbinder.eu/dart_api/manager/).

The important parity is: one generated schema, matching value/null/default/key semantics, parameterized queries, typed results/errors, consistent ordered live state, and deterministic disposal. Promise/Future, Drizzle operators/Dart typed lambdas, and callback/Stream differences are reasonable.

### 3. Use one query definition for one-shot, live, and optional persistent views

Avoid rebuilding the same filter as ORM expressions, raw SQL, and a separate sync spec. Introduce a prepared typed-query descriptor underneath existing public entry points. Shared metadata must preserve public/wire names and key/codec identity. UI hooks and the eventual Dart mirror consume that definition.

Simple streaming setup is achievable elsewhere too: Supabase exposes initial rows and changes through a small stream API. Its documented filter-exit limitations also show why line count alone is an insufficient benchmark. Source: [Supabase Dart stream](https://supabase.com/docs/reference/dart/stream).

### 4. Finish moving session ownership into the SDK

The new `planKalamSession` is an enum decision helper; it does not implement the coalesced `reconcileSession` behavior named in checked task T038. Masky still owns `_sessionSyncFuture`, `_sessionSyncQueued`, `_drainSessionChanges`, `_reconcileSessionState`, open/replace/dispose sequencing, and state listeners. The inspected service/session files contain 362 and 110 lines, respectively; these totals include business integration and are **not** a measured generic-glue reduction.

Add a session owner that accepts the desired account identity/auth state, coalesces transitions, fences old callbacks, and owns teardown. Compare full account/server/namespace identity, not just user ID. Keep domain consumers and business handling in Masky. This can use the existing storage/protocol and need not pull Slice 2 into Slice 1.

### 5. Make common failure and state handling predictable

The Dart builder currently reduces structured errors to `StateError(message)` and `live()` returns only row lists. Add a shared structured error model and a small live-state handle exposing rows, connection, loading/completeness where supported, error, refetch, and disposal. Preserve lightweight stream/callback conveniences. Do not claim snapshot completeness from connectivity or fake fields the current protocol cannot establish.

### 6. Measure efficiency and boilerplate with equivalent tasks

No performance or comparative line-count benchmark was executed in this review, so there is no evidence yet for “faster” or “less code than other SDKs.” Use a fixed acceptance matrix:

| Workflow | Boilerplate target | Correctness/efficiency gate |
| --- | --- | --- |
| Typed filtered read | One generated binding + one query expression | No manual decoder/string column names; one request |
| Ordered live list | Reuse query + one observation | Correct order/top-N under update/delete; bounded retained state |
| Insert/patch | One typed call | Defaults/omission/null correct; no map-key strings |
| Routine call | One generated method | Checked input/output, no SQL/serialization in application |
| Persistent Dart view | One mirror binding when Slice 2 exists | Cached-first state, explicit gap/reset behavior |
| Account transition | One SDK reconcile call | No app-owned transition queue or duplicate subscriptions |

Count handwritten setup, models, adapters, cleanup, and error handling—not just the final query expression. Benchmark request/subscription counts, retained memory, decoded/copied rows per change, and startup/reconnect runtime. Report runtimes in seconds using equivalent workloads. Generated adapters should avoid repeated per-row schema discovery and whole-history action scans.

## Verification performed

**140 existing focused tests passed:**

| Suite | Passed | Scope |
| --- | --- | --- |
| CLI schema/codegen | 46 | `cargo nextest run --offline -p kalam-cli --lib -E 'test(workflow::schema)'`; test execution 0.324 seconds |
| Dart link | 6 | Remote table builder and FILE unit tests |
| Dart sync | 17 | Session, store, and transport unit tests |
| TS client | 7 | Build/typecheck plus controller, typed descriptor, raw descriptor, README tests |
| TS ORM | 49 | Build plus selected schema, table, live descriptor, SQL/default/temporal tests |
| TS React | 15 | Typecheck plus five Vitest files; final test run 1.09 seconds |

The CLI binary was rebuilt from the current sources before fresh generation. **Ten additional behavioral probes failed**, identifying R1/R2/R3/R6/R8/R9/R10. Fresh generated Dart analysis and TS ORM-to-model assignment both failed for R4; emitted key metadata demonstrated R5. Standalone exact-source Dart/Rust filename comparisons demonstrated R7. R11 is source-confirmed.

Initially TS ORM lacked its local Drizzle dependency. For validation only, its missing dependency was temporarily linked to the already-installed React package copy; no downloads or manifest changes were needed. The review-created link was removed afterward. This environment issue is distinct from the code defects above.

After the initial Flutter suites/probes passed or reproduced their failures, later Flutter invocations were blocked by write access to the external SDK cache. The direct Dart analyzer still produced the array diagnostic, followed by a telemetry-file permission error. Neither tool-environment failure is counted as an SDK defect. No real-server, Android/iOS, browser E2E, full Masky acceptance, performance, TLS, or binary packaging validation is claimed.

## Reproducing and implementing

The three probe files intentionally assert the desired behavior and currently fail. Run after building the local TS client/ORM and preparing the normal package dependencies:

```sh
# From the repository root:
node --test specs/038-unified-sdk-contracts/review-2026-10-19/ts_probe.test.mjs

# From link/sdks/dart/link:
flutter test --no-pub ../../../../specs/038-unified-sdk-contracts/review-2026-10-19/remote_probe_test.dart

# From link/sdks/dart/sync:
flutter test --no-pub ../../../../specs/038-unified-sdk-contracts/review-2026-10-19/sync_probe_test.dart
```

For codegen, copy this directory's `kalam.toml` and `schema.sql` to a disposable directory, run the rebuilt CLI's `schema gen --languages typescript,dart`, then analyze/compile both targets with the local SDK packages. Assert `Inventory` and `inventory.$inferSelect` agree; assert `sku` is the only key. Do not edit generated output to make the check pass.

Implement R1/R2/R3 and the small R7/R8/R9/R11 fixes first, then address the existing generator and TS issues. Move successful probes into the owning test suites, add real-server ordered/filter/reconnect coverage, and only then re-evaluate the Slice 1 completion gate. Keep typed Dart generation and the session owner as explicit follow-up work; do not mark deferred Slice 2 features as implemented.
