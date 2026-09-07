# CLI: functions and deploy

## `kalam functions build`

Offline. Must:

1. Compile local SQL `ContractSnapshot`
2. Generate `contracts.ts`, `runtime.d.ts`, `registry.ts`; scaffold missing **non-inline** procedures once
3. Typecheck project against generated `.d.ts`
4. Typecheck inline TS/JS via generated shims (when project exists)
5. Run configured build/bundle
6. Write artifact + [manifest](build-manifest.md)
7. Validate exports vs catalog/snapshot

Must **not** require a live server. Must **not** only call schema generate.

Fails on: missing project-backed implementations, unknown exports, Node builtin / native addon, bundle requiring `eval`.

## `kalam functions override <schema.proc>`

Scaffolds `functions/src/<schema>/<proc>.ts`. May seed from inline body. Does not run on ordinary generate for inline procedures.

## `kalam functions status`

Active module, revision, runtime health (not only `SELECT` from `system.routines`).

## `kalam functions revisions`

```text
MODULE    REVISION   STATUS    CONTRACT     CREATED
backend   43         active    a83f...       ...
backend   42         ready     92da...       ...
```

## `kalam functions rollback <revision>`

CAS active pointer after: artifact exists, ABI supported, contract compatible with current schema. Does not rebuild. Not `CREATE OR REPLACE` of procedure bodies.

## `kalam functions logs [procedure]`

Structured function errors (not only `system.trigger_attempts`). Omit secrets/bodies.

## `kalam deploy`

Compile snapshot → generate → schema diff → **functions build** → validate manifest → upload artifact → apply catalog → immutable revision → CAS active → optional warm/health-check.

## `kalam deploy --dry-run`

All of the local non-mutating steps above (parse, diff, generate, typecheck, build, hashes, planned revision). **No** migrate, upload, catalog write, or activation.

## `kalam dev`

Watch `schema/**/*.sql`, `functions/src/**`, `package.json`, lockfile, `tsconfig.json`. Schema change: snapshot + generate + scaffold missing non-inline + typecheck. Function change: incremental bundle + local revision + activate. Do not restart the database.
