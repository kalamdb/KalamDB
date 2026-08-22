# kalam_sync_generator

Optional code generation for `kalam_sync` durable actions.

Add the generator and `build_runner` as development dependencies, annotate an
immutable payload with `@KalamActionPayload()`, and annotate executor methods
inside a `@KalamActionModule`. Running `dart run build_runner build` generates:

- a durable JSON codec for each supported payload;
- stable namespaced `KalamActionDefinition` values;
- a typed queue class with offline enqueue methods.

Drift remains responsible for table row and companion types. This generator
does not create duplicate create/update/delete models for every table; direct
KalamDB DML uses the runtime's single generic action envelope.

See the [`kalam_sync` chat example](../sync/example) for a complete offline-first
messaging app, REST engine, and live-server e2e suite. The canonical generated
action module lives in [`example/`](example).

```bash
cd example
flutter pub get
dart run build_runner build --delete-conflicting-outputs
```

The test suite generates multiple action modules in one library and then runs
the checked-in generated queue through a real in-memory SQLite outbox:

```bash
dart test
flutter test flutter_test/generated_action_runtime_e2e_test.dart
```
