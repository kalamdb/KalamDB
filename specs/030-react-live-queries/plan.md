# Implementation Plan: @kalamdb/react LiveQuery & LiveQueries

**Branch**: `[030-react-live-queries]` | **Date**: 2026-05-06 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/030-react-live-queries/spec.md`

## Summary

Create a new `@kalamdb/react` package that exposes declarative `LiveQuery` and `LiveQueries` primitives plus hook-first composition APIs for React while moving reusable live-query orchestration into `@kalamdb/client` and reusing typed schema mapping from `@kalamdb/orm`. The implementation will cover typed single-query mode, raw-SQL single-query mode for the live-compatible SQL subset, typed multi-query composition, assistant-workflow ergonomics for typing/presence/tooling/approvals, mutation-state tracking, docs/examples, a proof-of-use path inside the Admin UI, and a full validation app in `examples/react-ai-chat`.

## Technical Context

**Language/Version**: TypeScript 6.0.x, React 19.2, Node.js 18+ for package build/test  
**Primary Dependencies**: React 19, React DOM 19, `@kalamdb/client`, `@kalamdb/orm`, `drizzle-orm`, Vitest, React Testing Library  
**Storage**: Existing KalamDB HTTP/WebSocket APIs via `@kalamdb/client`; no new persistent storage  
**Testing**: `node --test` for shared client and ORM utilities, `vitest` + React Testing Library for `@kalamdb/react` and Admin UI integration, Playwright plus agent/example tests for `examples/react-ai-chat`, TypeScript compile checks per package  
**Target Platform**: Browser-based React apps and the KalamDB Admin UI  
**Project Type**: TypeScript SDK library package with React bindings plus Admin UI integration  
**Performance Goals**: First live render completes after one initial fetch plus one live-subscription handshake; committed row changes surface in the UI within the 2-second spec target; multi-query screens reuse the existing client instead of creating duplicate connections; high-churn assistant datasets such as typing and presence do not require effect-driven mirror state  
**Constraints**: Direct live SQL currently supports only `SELECT ... FROM ... WHERE ...`; `ORDER BY` and `LIMIT` must be reapplied client-side; max 100 subscriptions per connection; initial live snapshot ceiling is 10,000 rows; shared logic must stay UI-agnostic inside `@kalamdb/client`; React surface should prefer hook-first composition and provider-based wiring for complex screens; SDK changes must update repo docs and KalamSite docs  
**Scale/Scope**: One new `link/sdks/typescript/react` package, shared client live-controller additions, optional ORM descriptor helpers, docs/examples including an assistant workflow, one Admin UI consumer path, and one standalone `examples/react-ai-chat` validation app; initial multi-query release is typed-query only

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Gate | Status | Notes |
|------|--------|-------|
| Performance-first execution | PASS | Shared live-controller logic stays in `@kalamdb/client`, preventing duplicate framework implementations and redundant subscription orchestration. |
| Boundary ownership before convenience | PASS | The design keeps shared live-query behavior in `@kalamdb/client`, Drizzle-specific compilation in `@kalamdb/orm`, and React concerns in `@kalamdb/react` rather than splitting the same responsibility across packages. |
| Minimal dependency expansion | PASS | New React-only dependencies stay inside `@kalamdb/react`; shared packages avoid frontend-only dependencies. |
| Composable, low-boilerplate APIs | PASS | Subscription lifecycle, resume, projection, ordering/capping behavior, and assistant-workflow orchestration primitives are planned in client-core, while React screens use hooks plus provider/context composition with thin wrapper components. |
| Validation and documentation ship together | PASS | README, examples, Admin UI usage, standalone example coverage, focused tests, and KalamSite documentation updates are explicitly in scope. |

**Gate Result (pre-design)**: PASS. The feature intent fits the ratified repo-specific constitution in `.specify/memory/constitution.md` by keeping shared logic reusable, limiting dependency spread, and requiring validation plus docs from the start.

**Gate Result (post-design)**: PASS. The design keeps framework-agnostic live orchestration in `@kalamdb/client`, limits React-only concerns to the new package, preserves typed compilation in `@kalamdb/orm`, and includes the required docs and test surfaces required by the constitution.

## Project Structure

### Documentation (this feature)

```text
specs/030-react-live-queries/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── react-sdk.md
└── tasks.md
```

### Source Code (repository root)

```text
link/sdks/typescript/
├── client/
│   ├── src/
│   │   ├── client.ts
│   │   ├── helpers/subscription_helpers.ts
│   │   └── live/
│   │       ├── controller.ts          # NEW shared live-query controller primitives
│   │       ├── descriptor.ts          # NEW live-query descriptor validation/types
│   │       └── projection.ts          # NEW client-side order/limit/projection helpers
│   ├── tests/
│   └── README.md
├── orm/
│   ├── src/
│   │   ├── live.ts
│   │   ├── sql.ts
│   │   └── index.ts
│   ├── tests/
│   └── README.md
├── react/
│   ├── package.json
│   ├── tsconfig.json
│   ├── README.md
│   ├── src/
│   │   ├── index.ts
│   │   ├── types.ts
│   │   ├── context.tsx                # provider/client wiring for hooks and wrappers
│   │   ├── hooks/
│   │   │   ├── useLiveQuery.ts
│   │   │   ├── useLiveQueries.ts
│   │   │   ├── useMutationState.ts
│   │   │   └── useLiveSelection.ts    # NEW derived-view helper for low-boilerplate screens
│   │   └── components/
│   │       ├── LiveQuery.tsx
│   │       └── LiveQueries.tsx
│   ├── tests/
│   │   ├── live-query.test.tsx
│   │   ├── live-queries.test.tsx
│   │   ├── sql-mode.test.tsx
│   │   └── assistant-workflow.test.tsx
│   └── example/
└── README.md

examples/
└── react-ai-chat/
	├── package.json
	├── README.md
	├── setup.sh
	├── chat-app.sql
	├── vite.config.ts
	├── tsconfig.json
	├── scripts/
	│   ├── ensure-sdk.sh
	│   └── generate-schema.sh
	├── src/
	│   ├── App.tsx
	│   ├── main.tsx
	│   ├── agent.ts
	│   ├── styles.css
	│   └── components/
	│       ├── ConversationSidebar.tsx
	│       ├── ConversationHistory.tsx
	│       ├── ChatComposer.tsx
	│       ├── TypingIndicator.tsx
	│       ├── ToolCallTimeline.tsx
	│       └── MessageActions.tsx
	└── tests/
		├── agent.test.ts
		└── chat.spec.mjs

ui/
├── package.json
├── src/lib/kalam-client.ts
├── src/components/live-data/
│   └── ReactLiveQueryDemo.tsx         # NEW Admin UI consumer/pilot
├── src/components/assistant/
│   └── AssistantWorkflowDemo.tsx      # NEW assistant-style validation surface
├── src/pages/
│   └── LiveQueries.tsx                # optional integration point for the demo/pilot
└── src/**/*.test.tsx

docs/
└── sdk/                               # repo-side SDK usage docs updated in parallel
```

**Structure Decision**: Keep all framework-agnostic live-query orchestration in the existing TypeScript client package, keep Drizzle-specific query-descriptor compilation in the ORM package, and add `@kalamdb/react` as a sibling package under `link/sdks/typescript/` following the current SDK publishing layout. The React package will be hook-first for complex screens and expose thin component wrappers for declarative usage. The Admin UI consumes the new package through its existing client wiring in `ui/src/lib/kalam-client.ts` rather than duplicating live-query state logic locally, and `examples/react-ai-chat` serves as the standalone end-to-end validation surface for chat-app coverage.

## Complexity Tracking

No constitution exceptions require justification. The only intentional complexity is a shared live-controller layer in `@kalamdb/client`, which is necessary to avoid re-implementing subscription, resume, client-side projection behavior, and workflow-friendly orchestration in each future UI framework package.
