# TypeScript first-run audit — 2026-10-06

Tested the local developer journey using disposable `kalam init` projects, the
current dev-profile CLI/server builds, npm, and the repository's TypeScript SDK.
This is a local workflow audit, not a claim that every supported platform or
published package combination has been verified.

## Fixes from the walkthrough

1. **Inline JavaScript was split at internal semicolons.** Both `kalam --file`
   and schema migrations rejected valid procedures with “Unterminated
   dollar-quoted procedure body.” The CLI batch splitter now preserves dollar
   quotes, including tagged delimiters. A direct create-and-call reproduction
   succeeds after the fix. Server support for tagged procedure-body syntax was
   not changed.
2. **Generated projects silently skipped function type-checking.** There was no
   `functions/tsconfig.json`, so `kalam functions build` skipped the check.
   Generation now creates a strict, no-emit configuration when procedures exist
   and preserves a user's existing configuration.
3. **Inline checks used `input: unknown`.** Enabling the check exposed errors in
   valid `input.name` access. Inline checks now use generated request/result
   types, catching incompatible returns as well. Obsolete generated inline
   checks are removed when their procedures disappear or become project-backed.

## Exercised paths

| Path | Result |
| --- | --- |
| `init --yes --languages typescript --package-manager npm` | Files, dependencies, `.env`, schema and models created; next command printed |
| `up`, `status`, `down`, restart | Local instance managed successfully; data persisted |
| Second app on occupied port | Ownership error prevented accidental reuse; `up --port 2910` provided a separate instance |
| `dev start`, `dev status`, `dev stop` | Background development loop operated successfully |
| INSERT, SELECT, UPDATE, DELETE | Verified resulting data, not only successful command exits |
| Add column/table via `schema.sql` | Watcher migrated schema and regenerated models; existing row survived |
| Remove column/table via `schema.sql` | Agent mode paused with named destructive objects and `dev --force` guidance; intentional removal succeeded |
| Generated models after removal | Removed column/table disappeared from generated TypeScript; `npm run check` passed |
| Live TypeScript starter | Received rows and updates; repeated startup handled existing demo row |
| Inline procedure | `greet('Ada')` returned `Hello Ada`, including semicolon in body |
| Project TypeScript procedure | Scaffolded unimplemented handler, implemented it, rebuilt and activated without server restart; doubling 3 returned 6 |
| Generated `createKalam` client | Assertions passed for greeting and doubling 7 to 14 |
| HTTP function API | Authenticated `POST /v1/functions/verified_app/double_number` with `{ "value": 9 }` returned 18 |
| Missing table/column | Nonzero exit; missing-column error listed valid fields |
| Missing required value | NOT NULL error identified `email` and the row |
| Unimplemented procedure | Reported `procedure not implemented` with qualified name |
| Wrong function return type | TypeScript diagnostic identified source location and incompatible return type |
| Offline server | Explained URL/port/connectivity possibilities and supplied a healthcheck command |
| Logs and migration status | Available; migration status listed applied history |

## Remaining findings

These are not fixed by this patch.

### High priority

- **Unqualified procedure namespace mismatch.** In a project whose namespace is
  `first_app`, `CREATE PROCEDURE greet(...)` generated a client calling
  `first_app.greet`, but the migration contained `CREATE OR REPLACE PROCEDURE
  public.greet`. The schema-diff compiler uses `public` as its default. Use an
  explicit namespace in procedure declarations until generation and migration
  namespace resolution agree.
- **Missing TypeScript executable still skips checking.** `functions build`
  returns successfully when it cannot find a local `tsc`. The normal npm starter
  installs TypeScript, but missing dependencies need an actionable failure or an
  explicit “not checked” status.
- **Readiness does not mean the app process is healthy.** With dependencies
  unavailable, the loop emitted `KALAM_READY` and then `KALAM_APP_EXITED code=127`.
  Distinguish database readiness, function activation, and application health.

### Onboarding and diagnostics

- Each SQL invocation repeated expired saved-login warnings before successfully
  using project credentials. At the second project's port it instead suggested
  OIDC login for an unrelated saved server. Prefer the resolved project's
  credentials and suppress irrelevant recovery advice on successful fallback.
- NOT NULL errors are buried under `Raft error: Internal error: Provider error:
  Execution error`. Lead with the constraint, column and suggested correction;
  retain internal causes in verbose output.
- Default status and function-log tables truncate most column names and values
  under captured/narrow output. A vertical or compact summary would be clearer.
- `logs --local-file --no-color` retained ANSI escape sequences from the stored
  server log.
- A sandbox-denied socket bind appeared as “could not find a free HTTP port.”
  This was environment-specific; distinguish bind permission failures from
  genuinely occupied ports.
- A failed migration showed a recovery heading and error in agent mode without
  a concrete retry/repair command in that output.

## Verification and boundaries

- Focused CLI unit tests cover splitting, generation, function scaffolds and real
  `tsc` checks for accepted/rejected handler types.
- Focused server-backed smoke tests passed: inline JavaScript validation and
  replacement (1.745 seconds), ALTER TABLE with existing data (2.057 seconds).
- Initial smoke runs failed authentication; direct login and authenticated SQL
  succeeded. The successful rerun isolated the test token cache and explicitly
  pinned server routing, so this was not counted as a product fix.
- No release build, publication, deployment or commit was performed. The normal
  `kalam` installed on PATH was not replaced. Init used local SDK `file:` links;
  npm release installation still needs a separate clean-machine check.
- The walkthrough did not validate remote deployment, browser UI, every SQL
  datatype, every function error, or every authentication role. Test data lives
  only in the disposable projects under `/tmp`.
