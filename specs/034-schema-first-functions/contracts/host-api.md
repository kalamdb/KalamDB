# Host API (shared `runtime.d.ts`)

The host surface is specified once in `kalamdb-functions-host` (`HOST_METHODS`, `ASYNC_OPS`, `NATIVE_FNS`). Emitters write:

- V8 bootstrap (`emit_js_bootstrap()` → `__kalamMakeCtx`)
- TypeScript `runtime.d.ts` (`emit_typescript()` + schema-typed `FunctionsHost` from `ContractSnapshot`)
- Generated `procedure.d.ts` / `procedure.js` builders (`emit_procedure_builders`)

CLI `kalam schema gen` concatenates the TS host types with per-procedure methods. Inline SQL and bundled TS share the same frozen `ctx` at runtime. Native dispatch lives in `kalamdb-functions` (V8 bind / `kalamAsyncOp`); `kalamdb-core` `CoreFunctionHost` only implements AppContext adapters.

SQL table/procedure types and Drizzle `kTable` objects are generated once to
`src/generated/schema.ts` (sibling of `createKalam` in `src/generated/kalam.ts`).
`functions/src/generated/contracts.ts` re-exports that module plus `procedure`
from `procedure.js`. `runtime.js` exports `wrapProcedure` so generated builders
attach `ctx.orm` (Drizzle over `ctx.db`, with `ctx.orm.as(user)` for
`EXECUTE AS`). Never emit a sibling `runtime.ts` or `procedure.ts`, which would
shadow the declarations and type `ctx` / `input` as `any`. These files are
written to `functions/src/generated/` so the editor can resolve them (a gitignored
`.kalam/` folder is invisible to TypeScript).

Do not import `@kalamdb/client` into a procedure; the HTTP driver is not valid
inside V8. `ctx.orm` is attached by `procedure` builders, not by the V8 host
bootstrap — inline SQL bodies keep using `ctx.db`.

```ts
import type { ProcedureOrm } from "@kalamdb/orm";

export interface Actor {
  readonly id: string;
  readonly role: string;
}

export interface ProcedureContext {
  readonly actor: Actor;
  readonly principal: Actor;
  readonly source: { readonly kind: string };
  readonly parent: string | null;
  readonly db: DbHost;
  readonly orm: ProcedureOrm;
  readonly functions: FunctionsHost;
  readonly topics: TopicsHost;
  readonly log: LogHost;
  readonly http: HttpHost | null;
  sleep(ms: number): Promise<void>;
}

export interface DbHost {
  query(sql: string, params?: unknown[]): Promise<unknown>;
  execute(sql: string, params?: unknown[]): Promise<unknown>;
}

export interface FunctionsHost {
  call(name: string, args?: unknown): Promise<unknown>;
  // Schema methods, e.g. chat.createMessage(input): Promise<…>
  // still dispatch through kalamAsyncOp("call", routineId, …).
}

export interface TopicsHost {
  publish(topic: string, payload: unknown): Promise<void>;
}

export interface LogHost {
  debug(message: string, ...args: unknown[]): void;
  info(message: string, ...args: unknown[]): void;
  warn(message: string, ...args: unknown[]): void;
  error(message: string, ...args: unknown[]): void;
  error(error: unknown, message?: string, ...args: unknown[]): void;
}

export interface HttpHost {
  readonly request: {
    readonly method: string;
    readonly path: string;
    headers: { get(name: string): string | null };
    query: { get(name: string): string | null };
  };
  readonly response: {
    status(code: number): void;
    header(name: string, value: string): void;
    contentType(value: string): void;
  };
}

export type ProcedureHandler<TInput, TOutput> = (
  ctx: ProcedureContext,
  input: TInput,
) => TOutput | Promise<TOutput>;

export interface ProcedureBuilder<TInput, TOutput> {
  (handler: ProcedureHandler<TInput, TOutput>): ProcedureHandler<TInput, TOutput>;
  unimplemented(): ProcedureHandler<TInput, TOutput>;
}

export declare const procedure: {
  chat: {
    createMessage: ProcedureBuilder<ChatCreateMessageRequest, ChatCreateMessageResult>;
  };
};
```

Authoring:

```ts
import { procedure } from "../generated/contracts";

export const createMessage = procedure.chat.createMessage(async (ctx, input) => {
  return input;
});
```

Generation writes one `functions/src/<schema>/<procedure>.ts` per identity.
Local export names are arbitrary. Procedure identity comes from the generated
builder (`procedure.chat.createMessage` → `chat.create_message`).
`.unimplemented()` typechecks and scaffolds, but is omitted from the compiled
registry so CALL resolves to inline fallback or `PROCEDURE_NOT_IMPLEMENTED`.

## Rules

- `actor` / `principal` are read-only; JS cannot assign them.
- `ctx.sleep(ms)` pauses the isolate (capped at 60s and the remaining deadline).
- `http` is `null` for non-HTTP roots. Nested procedures may read `request`; `response` mutation throws.
- `functions.call` and typed `ctx.functions.<schema>.<method>` always go through host ACL/security — not a JS import of another `functions/src` file.
- CALL / REST payloads are not logged. `ctx.log` and isolate `console` share the process logger (`target: kalamdb::functions`). Records include `channel=ctx.log` or `channel=console`, plus message, optional error, extra JSON args, and invocation metadata (`request_id`, `routine`, `actor`, `principal`, `namespace`). `console.log` maps to info. Prefer `ctx.log` for structured procedure logs.

## Core adapters (edit `CoreFunctionHost` only for these)

These need `AppContext` / catalog / SQL / sessions. Do not add portable helpers here.

- `sql` / `query` / `execute`
- nested `call` / `call_async` / `prepare_nested_call`
- `publish` / `publish_async`
- HTTP request/response
- `metadata`
- `routine_js_map` (catalog → `{ "api": { "createOrder": "api.create_order" } }`)
- `procedure_stack`, `max_log_bytes`

## Adding a portable method (one spec entry)

Example: `ctx.now()` later. **Do not** add `now` to `CoreFunctionHost`.

1. Add a row to `HOST_METHODS` in `backend/crates/kalamdb-functions-host/src/spec.rs` (JS path, TS signature, `Native` or `AsyncOp`).
2. If it is a new native or async kind, add it to `NATIVE_FNS` or `ASYNC_OPS`. V8 bind iterates `NATIVE_FNS` and fails the isolate install if a name is missing.
3. Implement the behavior as a **default method** on `FunctionHost` in `kalamdb-functions` (clock, structured log, etc.). Tests and V8 use the default; core does not copy it.
4. If the method needs SQL, nested CALL, publish, or HTTP, call the existing adapters (`self.sql`, `self.call`, …) instead of adding a new Core method per feature.
5. Snapshot tests on `emit_typescript()` / `emit_js_bootstrap()` catch emitter drift. CLI goldens cover typed `FunctionsHost` methods and `procedure` builders.

## Inline shim

Dollar-quoted SQL cannot import. Build emits `functions/src/generated/inline/<schema>_<proc>.ts` wrapping the body with `ctx: ProcedureContext` for typecheck only.
