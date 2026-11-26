# Masky app findings: Dart SDK review

Reviewed 2026-11-08 against `../masky-v3/KALAM-NOTES.md` (February 2026).
Checked items describe inspected implementation or checks added here, not a
claim that a published SDK or live server was validated.

## Addressed in the current SDK

- [x] First-class resume: `liveEvents` and `liveEventsWithAck` accept
  `from: VersionId`; use the SDK checkpoint rather than adding a `seq_id` column.
  Existing reconnect/resume integration tests cover this path.
- [x] Subscription parameters are exposed as `params` on both raw event APIs.
  Caveat: these currently use client-side SQL literal substitution; see the open
  safety item below. Ordinary `query(params: ...)` sends parameters separately.
- [x] Readiness is observable through `AckEvent`. Attach a listener before writes;
  wait for the ack when testing live-only changes. Snapshot completion is a separate
  signal (`InitialDataBatch.hasMore == false`). Neither alone promises durable
  application processing or exactly-once delivery.
- [x] Durable local processing can use `liveEventsWithAck`: persist local changes
  before acknowledging a delivery. Prefer `kalam_sync` for a local-first app.
- [x] Android and iOS plugin folders exist; packaging tests check declared folders,
  both required Android binaries, their ELF architecture, and iOS archive size.
- [x] CI's SDK runner now executes every top-level offline test (excluding the
  opt-in `live_server_test.dart`), rather than only `models_test.dart`.
- [x] Publishing now blocks on offline failures; it no longer swallows test errors.
- [x] README documents the supported mobile platforms and hosted-version cache
  recovery after publishing.

## Remaining SDK work

- [ ] Replace subscription SQL substitution with typed parameters across the
  Dart bridge/shared client/server protocol. `bindSqlParams` currently uses repeated
  string replacement: placeholder-looking text in literals/comments or parameter
  values can be changed. Add regression cases for these, dollar-quoted SQL,
  missing parameters, non-finite numbers, and unsupported value types when fixing
  it. Do not treat the current helper as equivalent to server parameter binding.
- [ ] Validate package archive contents, then build/install representative Android
  and iOS apps in release CI. File/ELF checks do not prove Flutter linking or APK
  contents. This review did not run device builds.
- [ ] macOS/desktop/web: add complete platform support and runtime smoke coverage
  before advertising support; current plugin declaration is Android/iOS only.
- [ ] Reproduce the historical execute-as-user parameter loss with the current
  server/SDK contract before designing a Dart wrapper or multi-principal batch API.
- [ ] Verify large-ID handling across generated app models, JSON and web runtimes;
  preserve cursor types and avoid JavaScript-number conversion for int64 IDs.

## Server, TypeScript, or app-owned findings

These are not Dart-only fixes and remain unverified by this review:

- [ ] TypeScript Node WASM auto-resolution and executeAsUser parameter forwarding.
- [ ] Server insert/update classification and snapshot/replay boundary guarantees.
- [ ] User partition creation/repair and non-destructive app table migrations.
- [ ] Batched impersonated fanout, partial-failure reporting, retries/dead letters.
- [ ] `_seq` projection aliases, namespace qualification errors and guidance.
- [ ] App OpenAPI parity, media/action validation, capability manifests and embeddings.

App-specific fanout, OpenAPI models, Docker/bootstrap choices, and semantic search
belong in the app/backend. They should not be hidden behind SDK success responses.

## Verification for this change

- All 104 offline tests passed across 11 suites, including packaging checks.
- `flutter analyze --no-pub` reported no issues.
- Shell syntax and runner selection/failure propagation checks passed, including
  nonzero exit propagation through machine-output capture.
- Canonical skill mirrors regenerated and verified; website documentation corpus
  regenerated. No server integration, device build, or publication was run.
