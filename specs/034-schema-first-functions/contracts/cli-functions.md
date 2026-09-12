# CLI: functions and deploy

## `kalam functions build`

Offline. Must:

1. Compile local SQL `ContractSnapshot`
2. Generate `src/generated/contracts.ts`, `runtime.d.ts`, `runtime.js`, `procedure.d.ts`, `procedure.js`, and `registry.ts`; scaffold each missing identity once as `procedure.<schema>.<method>.unimplemented()` at `functions/src/<schema>/<proc>.ts`. Do not emit `runtime.ts` or `procedure.ts` (they shadow the declarations).
3. Discover top-level named `export const` bindings to generated `procedure` builders via a TypeScript/JavaScript AST parse (not regex). Helper exports are ignored.
4. Typecheck project against generated `.d.ts`
5. Typecheck inline TS/JS via generated shims (when project exists)
6. Bundle the generated registry as the single esbuild entry and append `kalamInvoke`
7. Write artifact + [manifest](build-manifest.md) with `module` | `inline` | `missing`
8. Validate bindings vs catalog/snapshot (unknown, duplicate, malformed)

Must **not** require a live server. Must **not** only call schema generate.

Fails on: unknown or duplicate procedure identities, malformed bindings, Node builtin / native addon, bundle requiring `eval`. Does **not** fail solely because a bodyless routine is `.unimplemented()` or unbound; that is recorded as `missing` (or `inline` when SQL has a body).

## `kalam functions override <schema.proc>`

Scaffolds a named `procedure.<schema>.<method>(handler)` export at `functions/src/<schema>/<proc>.ts` when that identity is not already bound anywhere. May seed from inline body. Does not run on ordinary generate for inline procedures.

## `kalam functions status`

Active module, revision, runtime health (not only `SELECT` from `system.routines`).

## `kalam functions revisions`

```text
MODULE    REVISION   IS_CURRENT  CONTRACT     CREATED
backend   43         true       a83f...       ...
backend   42         false      92da...       ...
```

## `kalam functions rollback <revision>`

CAS active pointer after: artifact exists, ABI supported, contract compatible with current schema. Does not rebuild. Not `CREATE OR REPLACE` of procedure bodies.

## `kalam functions logs [procedure]`

Structured procedure invocation, V8 `console.*`/`ctx.log.*`, and error records from `system.procedure_logs` (not only `system.trigger_attempts`). Omit secrets/bodies.

## `kalam functions runtime`

Resident isolates (`system.module_instances`), memory reservations from
`system.stats`, and in-flight roots (`system.active_procedure_runs`).

## `kalam deploy`

Compile snapshot → generate → schema diff → **functions build** → validate manifest → upload artifact → apply catalog → immutable revision → CAS active → optional warm/health-check. Activation sends only `module` procedure IDs. An empty export set means no project-backed procedures.

## `kalam deploy --dry-run`

All of the local non-mutating steps above (parse, diff, generate, typecheck, build, hashes, planned revision). **No** migrate, upload, catalog write, or activation.

## `kalam dev`

Watch `schema/**/*.sql`, `functions/src/**`, `package.json`, lockfile, `tsconfig.json`. Schema change: snapshot + generate + scaffold missing identities + typecheck. Function change: incremental bundle + local revision + activate. Do not restart the database.
