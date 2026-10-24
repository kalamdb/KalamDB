# Contract: PostgreSQL type and routine surface over native values and V8

**Review**: 2026-10-13, working tree based on `9cf03307`. This contract supplements [type-system.md](type-system.md) and [functions-v8.md](functions-v8.md). It defines implementation acceptance, not current support. Follow-up work is T125–T138; existing completion marks are not runtime evidence for these additional cases.

## Public surface and execution language

Use PostgreSQL names, quoting, type references, signature resolution, DDL, expressions and `CALL` semantics through both HTTP SQL and pgwire. A JS/TS routine is a first-class SQL function or procedure with the same catalog identity and binder as other languages. V8 is its execution backend; an SDK or JSON invocation envelope must not be required to call it from SQL.

`LANGUAGE javascript` and `LANGUAGE typescript` are Kalam language extensions within the PostgreSQL routine declaration shape. They are not built-in PostgreSQL languages, and Kalam's `input`/`ctx` API is not a claim of PLV8 compatibility. Normalize unquoted language identifiers using the shared SQL identifier rules, including any retained `js`/`ts` aliases at one registration boundary. Quoted language names remain case-sensitive. Unsupported handlers fail explicitly. [PostgreSQL procedure languages](https://www.postgresql.org/docs/18/sql-createprocedure.html).

SQL and PL/pgSQL body syntax retains its declared meaning. Never send an SQL body to a JS compiler, or relabel JS as `LANGUAGE sql` for apparent portability. SQL bodies use DataFusion and the shared routine/transaction owners; PL/pgSQL requires its own tested execution contract. Missing runtime coverage stays a blocking compatibility gap under T110. This does not delay the independent JS typed-value path.

## Parsing, names and quotes

Consume the shared token stream and typed AST. Ordinary `CREATE TYPE`, `CREATE FUNCTION`, `CREATE PROCEDURE`, `CALL`, `ALTER`, `DROP`, `COMMENT` and `GRANT` must not enter a language-word scan or a handwritten string splitter first. Extend the shared parser cursor only where the locked upstream AST cannot express a required construct. Preserve body source spans and diagnostics; no text rewrite, tokenization retry, AST printing/reparse, or separate parser for each DDL owner.

Fold only unquoted identifiers; retain quoted case, doubled double quotes, embedded dots, Unicode and reserved words. Apply PostgreSQL's configured identifier byte limit without splitting UTF-8, including truncation collisions and notices. Test each component of schema-qualified names, routine names, argument names, type names and composite fields. Keep component boundaries in typed names; never use `split('.')` to recover identity from a display string. [PostgreSQL lexical rules](https://www.postgresql.org/docs/18/sql-syntax-lexical.html).

Support ordinary and escape string literals, `$$...$$` and matching case-sensitive `$tag$...$tag$` delimiters. Body semicolons, comments, backslashes, JS template strings and inner dollar quotes with different tags must survive SQL batching unchanged. Respect `standard_conforming_strings`; dollar bodies are not SQL-unescaped internally. `AS`/`LANGUAGE` order must not alter classification. Preserve quoted type names until resolution, so a user type named `"INT"` is not treated as the builtin `int`.

## Type declarations and values

Composite and enum creation use PostgreSQL grammar, including empty definitions and composite attribute `COLLATE` where legal. Enum labels preserve case and whitespace; empty labels and colons are legal. Enforce the reference server's label byte limit, not a character count. Duplicate labels compare their actual values. Keep range, base, shell and multirange-related coverage inventoried by T124. [CREATE TYPE](https://www.postgresql.org/docs/18/sql-createtype.html), [enum implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/pg_enum.c).

`CREATE TYPE IF NOT EXISTS`, `CREATE OR REPLACE TYPE`, `FROM TABLE`, inline `COMMENT`, and field/argument `NOT NULL` or `NONEMPTY` are not portable PostgreSQL forms. Canonical generated SQL uses `COMMENT ON`, table row types and `CREATE DOMAIN` constraints as appropriate. Existing extensions require an explicit migration or a clearly bounded legacy adapter; never silently keep them as the canonical grammar.

Creating a table also creates its composite row type. Tables, types and domains therefore participate in the corresponding name conflicts. Do not copy the table schema into a second independent type when binding `schema.table`. A row value outside its table does not inherit table constraints; a domain may impose reusable value constraints. Preserve a NULL composite separately from a non-NULL composite whose fields are all NULL. Exercise `ROW(...)::schema.type`, whole-row arguments, field access and composite arrays. [Composite types](https://www.postgresql.org/docs/18/rowtypes.html).

Keep context-specific typmods: a composite attribute can retain `varchar(10)` or `numeric(12,2)`, but a function signature must discard parenthesized type modifiers. Do not remove constraints from a referenced domain. Verify the corresponding procedure and `%TYPE` behavior against the pinned server instead of inferring it from the column codec. [CREATE FUNCTION](https://www.postgresql.org/docs/18/sql-createfunction.html).

The parsed AST may retain syntactic modifiers and source spans. Discarding routine typmods is a semantic binding rule, not a SQL rewrite or a requirement to alter the upstream parse tree. Keep parser parity and effective signature tests separate.

## Routine identity and lifecycle

Use a stable internal RoutineId and stable catalog OID, separate from names and revisions. The overload key is schema identity, resolved name and ordered input type identities, including INOUT and the variadic array type. OUT-only types, argument names, defaults, return type and typmods do not distinguish overloads. Function/procedure kind is metadata, not an extra uniqueness discriminator: the same schema/name/input signature cannot identify one of each. [pg_proc](https://www.postgresql.org/docs/18/catalog-pg-proc.html), [CREATE PROCEDURE](https://www.postgresql.org/docs/18/sql-createprocedure.html).

Resolve aliases through the canonical registry, including array identity and domains. `OR REPLACE` must obey identity/result/name restrictions and preserve ownership, grants and stable identity; changing input types creates a different overload. Rename/schema move preserves identity. `ALTER`, signature-qualified `DROP`, `COMMENT`, `GRANT` and `REVOKE` share the resolver, with ambiguity errors for signatures omitted where PostgreSQL requires disambiguation. Consume all trailing tokens and honor RESTRICT/CASCADE; no drop-by-name shortcut. [ALTER FUNCTION](https://www.postgresql.org/docs/18/sql-alterfunction.html), [DROP FUNCTION](https://www.postgresql.org/docs/18/sql-dropfunction.html).

Publication of catalog metadata, executable artifact and ABI descriptor is atomic. A failed replacement leaves the previous executable callable. Revision-pinned calls may finish against their original descriptor; new calls see the committed revision. Invalidate prepared plans and code caches by dependencies, including type/layout, schema, language/ABI, grants and settings. Drop/recreate must not reuse a retired identity while calls or plans can observe it.

## Binding and result semantics

| Surface | Required behavior |
| --- | --- |
| Declaration arguments | Named or unnamed; IN, OUT, INOUT and VARIADIC; typed defaults using `DEFAULT` or `=`. Enforce PostgreSQL ordering and mode restrictions. |
| Invocation | Positional, named `=>` (and supported PostgreSQL legacy `:=`), mixed with positional arguments first; reject duplicates, unknown names and positional-after-named forms. |
| Overload resolution | Search path, actual input types, explicit/implicit casts, unknown literals, domains, default expansion and variadic candidates; ambiguity must fail rather than choose the first registry entry. |
| Defaults | Bind to the selected signature and evaluate when omitted, in the proper call context. Explicit NULL is not an omitted argument. Never cache a volatile default's value in the descriptor. |
| Functions | Invoke in expressions/SELECT; scalar, void, named composite, OUT/INOUT-derived record, SETOF and RETURNS TABLE have distinct result descriptors and cardinalities. |
| Procedures | Invoke with CALL; no RETURNS clause. OUT/INOUT produce the declared output row. SQL CALL supplies OUT placeholders where required; OUT expressions are not evaluated. |
| Composite argument | One typed composite/JSON value stays one argument even when its field names happen to equal parameter names. No object-shape guessing. |

The call binder owns these rules once, for SQL and typed host calls. Do not force PostgreSQL callers to send a JSON object instead of normal arguments. [Calling functions](https://www.postgresql.org/docs/18/sql-syntax-calling-funcs.html), [function resolution](https://www.postgresql.org/docs/18/typeconv-func.html), [SQL function results](https://www.postgresql.org/docs/18/xfunc-sql.html), [CALL](https://www.postgresql.org/docs/18/sql-call.html).

Negative fixtures include duplicate parameter names, invalid mode/default ordering, incompatible RETURNS and output declarations, OUT/INOUT combined with RETURNS TABLE, misplaced VARIADIC, contradictory/duplicate clauses and unsupported pseudo-types. Each accepted mode/type/result combination needs an executable case; a generic parse-success fixture is insufficient.

Resolve typed call expressions to the canonical routine descriptor before execution. Reuse DataFusion planning and array inputs for scalar/set/table invocation, with bounded V8 dispatch. T130 must prove the locked DataFusion integration supports the required async host work without blocking executor workers, per-row isolate creation, unbounded queued futures or locks held over guest awaits. SQL scalar invocation is still scalar semantics; batching transport cannot change function invocation count, volatility, errors or results. Include composite `.*` expansion versus LATERAL in the oracle; do not cache volatile result values to speed them up.

## Attributes, permissions and transactions

Default function behavior is VOLATILE and CALLED ON NULL INPUT. STRICT must skip the body for SQL-null input according to PostgreSQL rules, including variadic arrays; it is not an argument `NOT NULL` constraint. Map volatility to planner behavior and statement snapshots before allowing constant folding or caching. Implement or explicitly reject unsupported attributes such as PARALLEL, COST, ROWS, LEAKPROOF, SUPPORT and TRANSFORM, including privilege checks; do not accept and ignore them. [CREATE FUNCTION](https://www.postgresql.org/docs/18/sql-createfunction.html), [volatility](https://www.postgresql.org/docs/18/xfunc-volatility.html).

Invocations check EXECUTE, schema/type/language privileges and invoker/definer rules through the existing auth owner. Catalog grants and default privileges must match the advertised policy. Routine SET settings and search_path have scoped restoration on normal return and errors; SQL SET versus SET LOCAL effects follow PostgreSQL. A pooled isolate cannot retain the previous caller's settings or capabilities. Test malicious schema shadowing and nested invoker/definer calls.

Functions cannot commit/rollback their caller's transaction. Procedures may control transactions only in PostgreSQL-legal invocation contexts; explicit transaction blocks, definer routines and SET clauses impose restrictions. Missing legal transaction-control behavior remains an explicit gap, not silent autocommit. SQL invoked from JS uses the same transaction, principal frame, deadline and cancellation state as the caller. Test nested CALL stacks, exceptions/subtransactions, failed transactions and cancel-after-effect behavior. [CALL](https://www.postgresql.org/docs/18/sql-call.html), [procedure settings](https://www.postgresql.org/docs/18/sql-createprocedure.html), [transaction management](https://www.postgresql.org/docs/18/plpgsql-transactions.html).

## In-memory request and response contract

The runtime call is a typed internal message: descriptor/revision, explicit binding form, argument owner references and execution context. The reply is a typed value/set/output tuple or typed error. In-process channels must not encode these messages as JSON, KOBJ, FlexBuffers, protobuf or pgwire and decode them again. Channel movement may clone Arc handles; payload sharing remains subject to authorization and memory leases.

Define positional and named bindings explicitly. SQL argument notation and generated host metadata select the form; object keys do not. The ABI exposes an indexed argument view for unnamed parameters and a named `input` view for named parameters, both backed by the same bound slots. Legal SQL names such as `"a.b"`, `"default"` or `"__proto__"` are data properties accessed with brackets, never interpolated into generated JS identifiers or prototypes. A zero-argument call remains zero slots. Any legacy named-input heuristic lives only in the old ABI adapter with a removal milestone.

Bind native/Arrow values once against the selected signature, then retain their immutable owners through V8 views, nested calls and returned values. An ordinary JS object is validated and converted directly into the canonical typed representation once when it crosses the trust boundary; no JSON request/response envelope or stringify/parse roundtrip. Compatible returned host views reuse the owner after descriptor/projection checks. Output bindings distinguish a function's one composite result from a procedure's OUT tuple and a function's set of rows. New JS objects may need allocation/copy; views do not make arbitrary JS heaps shareable across isolates.

Plain-object snapshots must handle getters/proxies, cycles, mutation during async work, missing versus undefined fields, exact numeric/temporal values and bounded depth. Error mapping must preserve a useful SQLSTATE and located diagnostics; failed conversions never become zero or NULL. Reuse the existing lifecycle, realm isolation, native/external/heap budgets and buffer-transfer rules in [functions-v8.md](functions-v8.md). Actual SQL JSON operations, network output and storage codecs still encode/decode where semantically required.

For the new ABI, procedure JS completion supplies a descriptor-checked object for declared OUT/INOUT fields; SQL exposes only that tuple. A procedure with no outputs has no SQL result payload. Document and test missing/extra fields and void completion before implementation. Scalar/composite functions return their declared value; SETOF/TABLE use the separately specified bounded row iterator. Retain old JS body conventions only behind the explicit migration adapter.

## Acceptance examples and migration

The following declarations/calls are PostgreSQL reference fixtures, not a claim that they execute today:

```sql
CREATE SCHEMA "Api";
CREATE TYPE "Api"."Message" AS ("Text" text, amount numeric(12,2));
CREATE TYPE "Api"."State" AS ENUM ('', 'ready:now', 'Ready');
CREATE FUNCTION "Api"."Echo"(item "Api"."Message")
RETURNS "Api"."Message" LANGUAGE sql
AS $body$ SELECT item $body$;
SELECT "Api"."Echo"(item => ROW('hello', 1.25)::"Api"."Message");
CREATE PROCEDURE "Api"."Bump"(INOUT n integer)
LANGUAGE plpgsql AS $body$ BEGIN n := n + 1; END; $body$;
CALL "Api"."Bump"(1);
```

A paired Kalam fixture uses the same public type, signature and call shapes with different names, `LANGUAGE javascript`, and bodies such as `return input.item;` and `return { n: input.n + 1 };`. Those bodies follow the proposed new ABI. Compare SQL-visible results/metadata/errors against equivalent PG SQL/PLpgSQL routines; do not submit JS to stock PostgreSQL or mark the JS host API PostgreSQL-compatible.

Legacy procedure-with-RETURNS definitions require an inventory and reviewed migration to functions plus SELECT/expression callers, or to genuine procedures with output parameters plus CALL callers. Include schedules, triggers, grants, generated SDKs, stored definitions and artifact bindings. Keep a bounded, versioned migration adapter only where needed; never turn existing CALL into SELECT silently. Bodyless project-backed declarations remain an explicit Kalam deployment extension and do not redefine ordinary PostgreSQL DDL.

The full diff/watch/apply/build/activate journey is specified in [cli-development.md](cli-development.md), T139–T153. T137's client migration alone cannot close development lifecycle acceptance. Schema/type/routine changes must be ordered together and generated against the same captured source revision that the server commits.

## Required evidence

T136 compares syntax acceptance/rejection, SQLSTATE, values, types/OIDs, output names/cardinality, permissions and transaction outcomes through HTTP SQL and prepared pgwire. Cover quote/delimiter errors, Unicode byte limits, overload ambiguity, defaults/variadics, replacement, rollback, restart, dependency invalidation and legacy migration. SQL/PLpgSQL fixtures run unchanged on PG; JS counterparts test observable SQL semantics, not body-language portability.

T138 extends T112/T122 with separate counters for invocation-envelope encodes/decodes, payload copies, field materialization, conversion failures and live owner bytes. Compatible query→function→nested-call→return must have zero intermediate envelope encode/decode operations and zero payload-byte copies. Measure sparse and full reads, small scalars, large rows, composite arrays, overload/default binding, SQL scalar-per-row calls, warm/cold compilation and high concurrency. Report seconds, CPU, allocation and tail/retained-memory evidence against the previous KalamDB design and equivalent PG workloads under the existing budgets. Ordinary object creation, wire codecs and durable writes are measured separately, never hidden in a zero-copy claim.

## Implementation Evidence (2026-10-14)

- **Routine DDL, binding, and multi-language execution (`T125`–`T132`, `T135`–`T138`)**:
  - `kalamdb-dialect` and `kalamdb-handlers-ddl` support `CREATE [OR REPLACE] FUNCTION` and `CREATE [OR REPLACE] PROCEDURE` with `IN`/`OUT`/`INOUT`/`VARIADIC` modes, `DEFAULT` / `=` defaults, positional (`$1`) and named (`=>` and `:=`) invocation, `IMMUTABLE`/`STABLE`/`VOLATILE` volatility enforcement, `STRICT` / `CALLED ON NULL INPUT`, `SECURITY INVOKER` / `SECURITY DEFINER`, and routine `SET` options.
  - `SQL`, `PLPGSQL` (`DECLARE`, `IF ... THEN ... ELSE ... END IF`, `WHILE ... LOOP ... END LOOP`, `RETURN`, `RETURN QUERY`, `PERFORM`), and `JAVASCRIPT`/`TYPESCRIPT` routines execute over typed `RoutineValue` envelopes without JSON request/response packing.
  - Verified across 6 integration test suites (`pg18_routine_ddl`, `pg18_routine_binding`, `pg18_js_sql_calls`, `pg18_routine_context`, `pg18_routine_catalog`, `pg18_routines`) — **14/14 PASS in 0.20s**.
