# Host API (shared `runtime.d.ts`)

The host surface is specified once in `kalamdb-functions-host` (`HOST_METHODS`, `ASYNC_OPS`, `NATIVE_FNS`). Emitters write:

- V8 bootstrap (`emit_js_bootstrap()` → `__kalamMakeCtx`)
- TypeScript `runtime.d.ts` (`emit_typescript()` + schema-typed `FunctionsHost` from `ContractSnapshot`)

CLI `kalam schema gen` concatenates the TS host types with per-procedure methods. Inline SQL and bundled TS share the same frozen `ctx` at runtime. Native dispatch lives in `kalamdb-functions` (V8 bind / `kalamAsyncOp`); `kalamdb-core` `CoreFunctionHost` only implements AppContext adapters.

SQL input/output types stay in `contracts.ts`. `defineProcedure` lives in `runtime.d.ts` (contracts re-export it).

```ts
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
  readonly functions: FunctionsHost;
  readonly topics: TopicsHost;
  readonly log: LogHost;
  readonly http: HttpHost | null;
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

export function defineProcedure<C extends { input: unknown; output: unknown }>(
  handler: (ctx: ProcedureContext, input: C["input"]) => Promise<C["output"]> | C["output"],
): (ctx: ProcedureContext, input: C["input"]) => Promise<C["output"]> | C["output"];
```

## Rules

- `actor` / `principal` are read-only; JS cannot assign them.
- `http` is `null` for non-HTTP roots. Nested procedures may read `request`; `response` mutation throws.
- `functions.call` and typed `ctx.functions.<schema>.<method>` always go through host ACL/security — not a JS import of another `functions/src` file.
- CALL / REST payloads are not logged. `ctx.log` records message, optional error, extra JSON args, and invocation metadata (`request_id`, `routine`, `actor`, `principal`, `namespace`) on the process logger (`target: kalamdb::functions`).

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
5. Snapshot tests on `emit_typescript()` / `emit_js_bootstrap()` catch emitter drift. CLI goldens cover typed `FunctionsHost` methods.

## Inline shim

Dollar-quoted SQL cannot import. Build emits `.kalam/generated/inline/<schema>_<proc>.ts` wrapping the body with `ctx: ProcedureContext` for typecheck only.
