# Generated SDK contract check

This disposable-project check uses actual SDK packages, not generator snapshots or
mock ORM declarations. It checks generated TypeScript row/Drizzle compatibility,
negative TypeScript types, Dart array/enum/composite/date round trips, and negative
Dart filter types. It leaves generated files in a temporary directory only.

Prerequisites: built CLI (`cargo build -p kalam-cli --bin kalam`), built TS client
and ORM, installed TypeScript dependencies, and resolved Dart sync dependencies.

```sh
python3 link/tests/sdk-contracts/check_generated.py --flutter /path/to/flutter/bin/flutter
```

Use `--node-modules` for another installed TS dependency tree and `--kalam` for
another development CLI binary. The default TS dependencies are the React SDK's
local dependency tree. This check does not establish server/platform/soak readiness.
