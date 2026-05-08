# Research: @kalamdb/react LiveQuery & LiveQueries

## Decision: Add `@kalamdb/react` as a sibling SDK package under `link/sdks/typescript/`

- **Decision**: Create a new package at `link/sdks/typescript/react-old` with its own `package.json`, `tsconfig.json`, `README.md`, `src/`, and `tests/`, following the same publish/build layout used by `client/`, `orm/`, and `consumer/`.
- **Rationale**: The repo already organizes TypeScript deliverables as independent npm packages. A sibling package keeps React-specific dependencies and test tooling out of the shared client and preserves a clean public surface for framework bindings.
- **Alternatives considered**:
  - Put React exports inside `@kalamdb/client`: rejected because it would force React dependencies into the shared client and make future Angular/Vue packaging harder.
  - Put React exports inside `@kalamdb/orm`: rejected because raw SQL mode must work without Drizzle and the package would become an awkward mix of ORM and framework code.

## Decision: Move reusable live-query orchestration into `@kalamdb/client`

- **Decision**: Add a framework-agnostic live-controller layer in `@kalamdb/client` to own subscription lifecycle, materialized row updates, resume checkpoints, connection state, refetch support, and client-side projection helpers.
- **Rationale**: The existing client already owns WebSocket lifecycle, row materialization, and resume semantics. Extending it with reusable controller primitives preserves a single source of truth for live behavior and lets future UI packages reuse the same contract.
- **Alternatives considered**:
  - Rebuild subscription orchestration inside `@kalamdb/react`: rejected because it would duplicate logic that future non-React packages also need.
  - Push everything into `@kalamdb/orm`: rejected because raw SQL mode and future framework reuse do not belong in the Drizzle bridge.

## Decision: Make hooks the primary composition surface and keep `LiveQuery`/`LiveQueries` as thin wrappers

- **Decision**: Expose `useLiveQuery` and `useLiveQueries` as the primary ergonomic API for advanced screens, with `LiveQuery` and `LiveQueries` remaining convenience wrappers over those hooks.
- **Rationale**: Complex assistant screens that combine messages, tool activity, typing, presence, and approval queues become awkward if consumers must nest multiple render-prop wrappers. Hook-first composition aligns better with current React practice and keeps screen-level composition readable.
- **Alternatives considered**:
  - Keep only render-prop components: rejected because it increases boilerplate and fights React composition patterns on complex screens.
  - Expose only hooks and drop components entirely: rejected because the original feature request explicitly calls for component primitives and simple declarative entry points.

## Decision: Support pure derived selections for screen-ready assistant state

- **Decision**: Let `useLiveQuery` and `useLiveQueries` support derived selections or companion selection helpers so screens can compute values such as active typing users, open tool calls, and pending approvals directly from live state without mirroring rows in `useEffect`.
- **Rationale**: React best practice is to derive view state during render rather than copying source state into parallel local state. Assistant UIs amplify this need because several high-churn live datasets must be combined into screen-ready slices.
- **Alternatives considered**:
  - Require consumers to build derived mirrors with `useEffect`: rejected because it increases boilerplate and introduces avoidable consistency bugs.
  - Ship assistant-specific selectors in the package: rejected because the package should stay generic even while making assistant workflows ergonomic.

## Decision: Keep Drizzle-specific descriptor compilation in `@kalamdb/orm`

- **Decision**: Extend `@kalamdb/orm` with typed live-query descriptor helpers that compile table-based definitions into live-safe SQL and typed row mapping metadata for the shared client controller.
- **Rationale**: `@kalamdb/orm` already knows how to compile Drizzle SQL and normalize rows back into typed models. Reusing that package avoids duplicating Drizzle internals in the React layer.
- **Alternatives considered**:
  - Compile Drizzle definitions directly inside `@kalamdb/react`: rejected because it would duplicate type-mapping logic already present in `orm/src/live.ts` and make React the owner of ORM behavior.
  - Treat typed mode as raw SQL only: rejected because it would lose the type-safe developer experience requested in the spec.

## Decision: Support raw SQL live mode only for the current live-compatible SQL subset in v1

- **Decision**: The initial release of raw SQL live mode will support single-query SQL that can be expressed against KalamDB's current live-subscription contract. `ORDER BY` and `LIMIT` will be reapplied client-side when possible, while unsupported query shapes such as aggregate/grouped/compound live SQL remain out of scope for v1 and must fail with a clear validation error.
- **Rationale**: Current SDK docs and live APIs explicitly require `SELECT ... FROM ... WHERE ...` shape for direct live subscriptions and reject `ORDER BY`/`LIMIT` in subscription SQL. Trying to promise arbitrary live SQL in v1 would create a mismatch between the package contract and the server's actual capabilities.
- **Alternatives considered**:
  - Pass the raw SQL string directly to `client.live()` regardless of shape: rejected because existing live validation rejects ordered/limited queries.
  - Rewrite arbitrary SQL into a live-safe query automatically: rejected for v1 because generalized AST rewriting for joins, aggregates, and projections is high risk and not required to deliver the first useful version.
  - Poll unsupported SQL instead of subscribing: rejected because the feature is explicitly real-time and should not silently degrade to polling.

## Decision: Track mutation progress locally but let live updates remain the source of row truth

- **Decision**: `@kalamdb/react` will manage `inserting`, `updating`, and `deleting` state locally for UI feedback, but rows stay authoritative to the live controller and update only when confirmed query results arrive.
- **Rationale**: This gives the requested mutation-state feedback without forcing optimistic row reconciliation rules into the shared client. It also avoids conflicts between temporary optimistic rows and authoritative live updates.
- **Alternatives considered**:
  - Optimistically mutate local rows before the live stream confirms changes: rejected for v1 because it complicates reconciliation, rollback, and ordering guarantees.
  - Put mutation-state tracking in `@kalamdb/client`: rejected because in-flight UI state is framework-facing and not needed by non-UI consumers.

## Decision: `LiveQueries` orchestrates one controller per named query and aggregates state in React

- **Decision**: Implement `LiveQueries` by creating one live controller per named query definition, aggregating loading/connection/error state in React, and routing mutations to the correct target dataset while reusing the same underlying client instance.
- **Rationale**: One controller per query preserves isolation, makes partial failure visible, and matches the user-facing mental model of named datasets such as `messages`, `typing`, and `presence`.
- **Alternatives considered**:
  - Build a single composite backend query: rejected because named multi-query results map better to independent subscriptions and the server does not expose a composite multi-query live protocol.
  - Manage each live query entirely in consumer code: rejected because the entire feature exists to remove that orchestration burden from application code.

## Decision: Validate the API explicitly against assistant workflows

- **Decision**: Include an assistant-workflow example and tests that combine messages, tool calls, tool results, typing or presence, and approval rows through one `useLiveQueries` or `LiveQueries` declaration.
- **Rationale**: This is the most demanding real-time UI case in the current product direction, and it exposes whether the API truly reduces boilerplate or only works for trivial tables.
- **Alternatives considered**:
  - Treat assistant workflows as a downstream example only after v1: rejected because the user explicitly wants this case covered by the initial design.
  - Add assistant-specific UI widgets: rejected because the package should remain generic and UI-agnostic beyond React composition primitives.

## Decision: Add a standalone `examples/react-ai-chat` validation app

- **Decision**: Create a new example at `examples/react-ai-chat` that uses `@kalamdb/react` as its primary live-data integration layer, reuses the general Vite + `runAgent()` topology from `examples/chat-with-ai`, and validates a fuller chat application surface including conversation selection, history loading, multi-file messages, typing/streaming feedback, tool activity, and message edit/cancel actions.
- **Rationale**: The Admin UI demo proves local integration inside the product, but a standalone example is the clearest way to prove the package covers real chat-app development needs. Reusing the existing chat example topology reduces setup risk and keeps the new example focused on package ergonomics instead of infrastructure invention.
- **Alternatives considered**:
  - Extend `examples/chat-with-ai` directly: rejected because that example is intentionally minimal and should remain a compact ask/reply demo.
  - Use only an Admin UI demo as validation: rejected because it does not provide an isolated developer-facing example of how to build a chat app with the package.

## Decision: Keep `examples/react-ai-chat` chat state in USER tables where requested

- **Decision**: Model conversations, messages, and typing indicators in USER tables for `examples/react-ai-chat`, while using the agent and topic pattern to drive assistant work and tool activity similarly to `examples/chat-with-ai`.
- **Rationale**: The example is meant to validate app-development ergonomics around user-scoped live state, and USER tables align with the repo's product model for end-user chat data.
- **Alternatives considered**:
  - Put conversation activity in STREAM tables only: rejected because the user explicitly wants conversations/messages/typing to be USER tables and the example should validate that path.
  - Collapse all chat state into one table: rejected because separate conversation, message, typing, and tool-activity surfaces better exercise the new multi-query primitives.

## Decision: Use optional peer dependencies for Drizzle-only features

- **Decision**: `@kalamdb/react` will require `react`, `react-dom`, and `@kalamdb/client` as peer dependencies, and treat `@kalamdb/orm` plus `drizzle-orm` as optional peers used only for typed table-based mode.
- **Rationale**: Raw SQL mode should not force Drizzle installation, but typed mode must still integrate cleanly with Drizzle tables and types.
- **Alternatives considered**:
  - Hard dependency on Drizzle for all installs: rejected because it burdens SQL-only consumers.
  - Separate raw-SQL and typed React packages: rejected because it fragments the package surface without enough benefit.

## Decision: Use Vitest + React Testing Library for the React package, keep shared packages on their existing test style

- **Decision**: Test shared controller/descriptor additions with the existing package-local test styles (`node --test` in client/orm), and test `@kalamdb/react` with Vitest + React Testing Library because the package exports React hooks and components.
- **Rationale**: The repo's UI already uses Vitest and React Testing Library, which fit JSX, hooks, and DOM-oriented render assertions much better than the minimal Node runner alone.
- **Alternatives considered**:
  - Force the React package to use only `node --test`: rejected because component and hook testing become unnecessarily brittle.
  - Move all SDK tests to Vitest: rejected because the existing client/orm packages already have stable Node-runner coverage and do not need a wholesale test-runner migration.

## Decision: Prove Admin UI compatibility through the existing client singleton boundary

- **Decision**: Admin UI adoption will consume `@kalamdb/react` through the existing UI client wiring in `ui/src/lib/kalam-client.ts` rather than introducing a second connection-management path.
- **Rationale**: The Admin UI already separates query and subscription client lifecycle. Reusing that boundary reduces integration risk and validates that the React package works with the repo's real authenticated browser setup.
- **Alternatives considered**:
  - Build a second UI-only client/provider for the demo: rejected because it would test the wrong integration path and duplicate live-connection management.
  - Leave Admin UI validation out of scope: rejected because the acceptance criteria explicitly require the package to be usable inside the Admin UI.