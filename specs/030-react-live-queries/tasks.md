# Tasks: @kalamdb/react LiveQuery & LiveQueries

**Input**: Design documents from `/specs/030-react-live-queries/`  
**Prerequisites**: plan.md ✅, spec.md ✅, research.md ✅, data-model.md ✅, contracts/ ✅, quickstart.md ✅  
**Last Updated**: 2026-05-07

**Tests**: Required. Repo guidance requires SDK changes under `link/sdks/**` to ship with test coverage, so each user story includes focused package or Admin UI validation tasks.

**Organization**: Tasks are grouped by user story so each story remains independently implementable and testable after the shared live-query foundation is complete.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks in the same phase)
- **[Story]**: Which user story this belongs to (`US1`, `US2`, `US3`, `US4`, `US5`)
- Every task includes exact file paths

## Path Conventions

- Shared client SDK: `link/sdks/typescript/client/`
- Drizzle ORM SDK: `link/sdks/typescript/orm/`
- React SDK package: `link/sdks/typescript/react/`
- Admin UI integration: `ui/src/`
- Standalone example app: `examples/react-ai-chat/`
- Repo SDK docs: `docs/sdk/sdk.md`
- External SDK docs sync: `../KalamSite/content/sdk/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the new React SDK package surface and local repo wiring needed by every story.

- [X] T001 Create the new React package metadata in `link/sdks/typescript/react/package.json`, `link/sdks/typescript/react/tsconfig.json`, and `link/sdks/typescript/react/README.md`
- [X] T002 Create the React package scaffold in `link/sdks/typescript/react/src/index.ts`, `link/sdks/typescript/react/src/types.ts`, `link/sdks/typescript/react/tests/test-utils.tsx`, and `link/sdks/typescript/react/example/.gitkeep`
- [X] T003 [P] Add `@kalamdb/react` to the SDK package overview in `link/sdks/typescript/README.md`
- [X] T004 [P] Wire local `@kalamdb/react` consumption and build-path resolution in `ui/package.json`, `ui/tsconfig.json`, and `ui/vite.config.ts`

**Checkpoint**: The repo recognizes `@kalamdb/react` as a first-class local package and the Admin UI can consume it during development.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish the shared client, ORM, and React infrastructure that all user stories depend on.

**⚠️ CRITICAL**: No user story work should begin until this phase is complete.

- [X] T005 Create shared live descriptor and projection contracts in `link/sdks/typescript/client/src/live/descriptor.ts`, `link/sdks/typescript/client/src/live/projection.ts`, and `link/sdks/typescript/client/src/index.ts`
- [X] T006 Integrate shared live controller lifecycle, reconnect, and refetch support in `link/sdks/typescript/client/src/live/controller.ts`, `link/sdks/typescript/client/src/client.ts`, `link/sdks/typescript/client/src/types.ts`, and `link/sdks/typescript/client/src/helpers/subscription_helpers.ts`
- [X] T007 [P] Extend Drizzle live descriptor compilation and typed row-key extraction in `link/sdks/typescript/orm/src/live.ts`, `link/sdks/typescript/orm/src/sql.ts`, and `link/sdks/typescript/orm/src/index.ts`
- [X] T008 [P] Create shared React provider and mutation plumbing in `link/sdks/typescript/react/src/context.tsx`, `link/sdks/typescript/react/src/hooks/useMutationState.ts`, `link/sdks/typescript/react/src/types.ts`, and `link/sdks/typescript/react/src/index.ts`
- [X] T009 [P] Add foundational shared-core coverage in `link/sdks/typescript/client/tests/live-controller.test.mjs`, `link/sdks/typescript/orm/tests/live-descriptor.test.mjs`, and `link/sdks/typescript/react/tests/test-utils.tsx`

**Checkpoint**: Shared live-query controller, descriptor compilation, provider wiring, and base mutation state are ready for story-level API work.

---

## Phase 3: User Story 1 - Build a Single Live Screen (Priority: P1) 🎯 MVP

**Goal**: Deliver typed single-query React screens with loading, reconnect, refetch, and mutation state while keeping the API low-boilerplate.

**Independent Test**: Render one typed live query, observe initial load and reconnect behavior, then perform insert/update/delete actions and verify mutation state and rows update without manual subscription management.

### Tests for User Story 1

- [X] T010 [P] [US1] Add typed single-query hook and component tests in `link/sdks/typescript/react/tests/live-query.test.tsx`
- [X] T011 [P] [US1] Add typed single-query shared-core coverage in `link/sdks/typescript/client/tests/live-query-typed.test.mjs` and `link/sdks/typescript/orm/tests/live-typed-query.test.mjs`

### Implementation for User Story 1

- [X] T012 [US1] Implement typed `useLiveQuery` loading, reconnect, and refetch behavior in `link/sdks/typescript/react/src/hooks/useLiveQuery.ts`
- [X] T013 [US1] Implement the `LiveQuery` wrapper and typed render-context exports in `link/sdks/typescript/react/src/components/LiveQuery.tsx` and `link/sdks/typescript/react/src/index.ts`
- [X] T014 [US1] Add typed mutation action builders and row-key tracking in `link/sdks/typescript/react/src/hooks/useLiveQuery.ts` and `link/sdks/typescript/react/src/hooks/useMutationState.ts`
- [X] T015 [US1] Create the typed single-query example in `link/sdks/typescript/react/example/messages-pane.tsx`

**Checkpoint**: Typed single-query screens are independently usable and form the MVP for the new package.

---

## Phase 4: User Story 2 - Build a Live Screen From SQL (Priority: P2)

**Goal**: Add raw SQL single-query mode for the supported live-query subset, including validation, projection normalization, and mutation support.

**Independent Test**: Render one SQL-based live query, confirm live-compatible SQL is normalized correctly, and verify mutation/error state without manual merge logic.

### Tests for User Story 2

- [X] T016 [P] [US2] Add raw-SQL hook and component tests in `link/sdks/typescript/react/tests/sql-mode.test.tsx`
- [X] T017 [P] [US2] Add raw-SQL descriptor normalization coverage in `link/sdks/typescript/client/tests/live-sql-descriptor.test.mjs`

### Implementation for User Story 2

- [X] T018 [US2] Implement raw-SQL live query normalization and query-shape validation in `link/sdks/typescript/client/src/live/descriptor.ts` and `link/sdks/typescript/client/src/live/projection.ts`
- [X] T019 [US2] Implement raw-SQL `useLiveQuery` support in `link/sdks/typescript/react/src/hooks/useLiveQuery.ts` and `link/sdks/typescript/react/src/types.ts`
- [X] T020 [US2] Add raw-SQL key-strategy and mutation routing behavior in `link/sdks/typescript/react/src/hooks/useLiveQuery.ts` and `link/sdks/typescript/react/src/hooks/useMutationState.ts`
- [X] T021 [US2] Create the raw-SQL example in `link/sdks/typescript/react/example/sql-messages-pane.tsx`

**Checkpoint**: SQL-first single-query screens work for the v1 live-compatible subset without breaking the typed single-query API.

---

## Phase 5: User Story 3 - Compose Multiple Live Datasets (Priority: P3)

**Goal**: Deliver `useLiveQueries` and `LiveQueries` so one screen can combine multiple named datasets with aggregate state and target-aware mutations.

**Independent Test**: Render a screen with at least two typed live datasets, observe aggregate loading and error state, and mutate one dataset without disrupting the others.

### Tests for User Story 3

- [X] T022 [P] [US3] Add multi-query hook and component tests in `link/sdks/typescript/react/tests/live-queries.test.tsx`
- [X] T023 [P] [US3] Add Admin UI multi-query integration coverage in `ui/src/components/live-data/ReactLiveQueryDemo.test.tsx`

### Implementation for User Story 3

- [X] T024 [US3] Implement `useLiveQueries` controller orchestration in `link/sdks/typescript/react/src/hooks/useLiveQueries.ts`
- [X] T025 [US3] Implement `LiveQueries` and typed multi-query context exports in `link/sdks/typescript/react/src/components/LiveQueries.tsx`, `link/sdks/typescript/react/src/types.ts`, and `link/sdks/typescript/react/src/index.ts`
- [X] T026 [US3] Add aggregate loading/connection/error state and target-aware mutation routing in `link/sdks/typescript/react/src/hooks/useLiveQueries.ts` and `link/sdks/typescript/react/src/hooks/useMutationState.ts`
- [X] T027 [US3] Create the generic Admin UI live-data pilot in `ui/src/components/live-data/ReactLiveQueryDemo.tsx` and integrate it with `ui/src/pages/LiveQueries.tsx`

**Checkpoint**: Multi-query composition is independently usable for collaborative screens and the Admin UI has a real consumer path.

---

## Phase 6: User Story 4 - Build an AI Assistant Workspace (Priority: P3)

**Goal**: Make assistant-style screens ergonomic by supporting derived selections, tool activity, typing/presence, and human approval flows with minimal React boilerplate.

**Independent Test**: Render an assistant workspace that combines messages, tool calls, tool results, typing or presence, and approvals; derive pending approvals and active tool calls without effect-driven mirror state; approve or reject a pending step without resetting the other datasets.

### Tests for User Story 4

- [X] T028 [P] [US4] Add assistant workflow and derived-selection tests in `link/sdks/typescript/react/tests/assistant-workflow.test.tsx`
- [X] T029 [P] [US4] Add Admin UI assistant workflow coverage in `ui/src/components/assistant/AssistantWorkflowDemo.test.tsx`

### Implementation for User Story 4

- [X] T030 [US4] Implement derived selection support in `link/sdks/typescript/react/src/hooks/useLiveSelection.ts`, `link/sdks/typescript/react/src/hooks/useLiveQueries.ts`, and `link/sdks/typescript/react/src/types.ts`
- [X] T031 [US4] Export selection helpers and assistant-friendly hook typings in `link/sdks/typescript/react/src/index.ts` and `link/sdks/typescript/react/src/types.ts`
- [X] T032 [US4] Create the assistant workflow Admin UI demo in `ui/src/components/assistant/AssistantWorkflowDemo.tsx` and surface it from `ui/src/pages/LiveQueries.tsx`
- [X] T033 [US4] Create the assistant workflow example in `link/sdks/typescript/react/example/assistant-workspace.tsx`

**Checkpoint**: Assistant-style multi-query screens are independently usable with low-boilerplate derived state and human-in-the-loop approvals.

---

## Phase 7: User Story 5 - Validate the package with a full chat app example (Priority: P3)

**Goal**: Ship a standalone `examples/react-ai-chat` app that proves `@kalamdb/react` covers conversation navigation, history loading, typing, file-backed messages, tool activity, streamed replies, and message edit/cancel flows.

**Independent Test**: Start the example app and its agent worker, create or select a conversation from the sidebar, load the selected conversation history, send a multi-file message, observe typing and streamed reply/tool activity, and edit or cancel a user message without breaking the rest of the conversation state.

### Tests for User Story 5

- [X] T034 [P] [US5] Add standalone example validation coverage in `examples/react-ai-chat/tests/agent.test.ts` and `examples/react-ai-chat/tests/chat.spec.mjs`

### Implementation for User Story 5

- [X] T035 [US5] Create the example package and bootstrap surface in `examples/react-ai-chat/package.json`, `examples/react-ai-chat/README.md`, `examples/react-ai-chat/setup.sh`, `examples/react-ai-chat/vite.config.ts`, `examples/react-ai-chat/tsconfig.json`, and `examples/react-ai-chat/scripts/ensure-sdk.sh`
- [X] T036 [US5] Define the example schema, USER tables, topic routes, and schema generation in `examples/react-ai-chat/chat-app.sql` and `examples/react-ai-chat/scripts/generate-schema.sh`
- [X] T037 [US5] Build the conversation shell and sidebar navigation in `examples/react-ai-chat/src/App.tsx`, `examples/react-ai-chat/src/main.tsx`, `examples/react-ai-chat/src/styles.css`, and `examples/react-ai-chat/src/components/ConversationSidebar.tsx`
- [X] T038 [US5] Implement conversation history loading, multi-file message send, typing indicator, and message edit/cancel actions in `examples/react-ai-chat/src/components/ConversationHistory.tsx`, `examples/react-ai-chat/src/components/ChatComposer.tsx`, `examples/react-ai-chat/src/components/TypingIndicator.tsx`, and `examples/react-ai-chat/src/components/MessageActions.tsx`
- [X] T039 [US5] Implement streamed replies and tool-call activity views with `@kalamdb/react` in `examples/react-ai-chat/src/components/StreamingReply.tsx`, `examples/react-ai-chat/src/components/ToolCallTimeline.tsx`, and `examples/react-ai-chat/src/App.tsx`
- [X] T040 [US5] Implement the topic-consuming agent worker and conversation-scoped tool flow in `examples/react-ai-chat/src/agent.ts` and `examples/react-ai-chat/src/App.tsx`

**Checkpoint**: The standalone example proves the package can support a real chat app with conversation navigation, files, typing, streamed replies, and tool activity.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Finish docs, examples, external SDK sync, and focused validation across the package and Admin UI surfaces.

- [X] T041 [P] Update SDK package docs in `link/sdks/typescript/react/README.md`, `link/sdks/typescript/client/README.md`, `link/sdks/typescript/orm/README.md`, and `link/sdks/typescript/README.md`
- [X] T042 [P] Update repo SDK docs and quickstart references in `docs/sdk/sdk.md` and `specs/030-react-live-queries/quickstart.md`
- [X] T043 [P] Sync external SDK docs in `../KalamSite/content/sdk/react-live-query.md`, `../KalamSite/content/sdk/react-assistant-workflow.md`, and `../KalamSite/content/sdk/react-ai-chat-example.md`
- [X] T044 Run focused package validation for `link/sdks/typescript/client`, `link/sdks/typescript/orm`, and `link/sdks/typescript/react`, then fix touched files until the package test/build commands pass and the supported raw-SQL subset behavior is exercised explicitly
- [X] T045 Run Admin UI and `examples/react-ai-chat` validation for `ui/package.json` and `examples/react-ai-chat/package.json`, record whether committed updates stay within the 2-second target and whether mutation-state indicators remain correct across the tested create/update/delete flows, then fix touched files until those checks pass
- [X] T046 [P] Time a docs-first single-query onboarding run from `link/sdks/typescript/react/example/`, `examples/react-ai-chat/`, `ui/src/lib/kalam-client.ts`, and `specs/030-react-live-queries/quickstart.md`, and confirm the published examples cover typed single-query, SQL subset, multi-query, assistant workflow, and standalone chat-app validation flows

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: Can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion and blocks all user stories
- **User Story 1 (Phase 3)**: Depends on Phase 2 and delivers the MVP single-query surface
- **User Story 2 (Phase 4)**: Depends on Phase 2 and extends the single-query API with raw SQL support
- **User Story 3 (Phase 5)**: Depends on Phase 2 and builds the multi-query surface on top of the shared controller and mutation primitives
- **User Story 4 (Phase 6)**: Depends on Phase 5 because assistant workflows require the multi-query and selection surfaces to be in place
- **User Story 5 (Phase 7)**: Depends on US4 because the standalone example should validate the final multi-query and assistant workflow surface
- **Polish (Phase 8)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1 (P1)**: No story dependency after Foundational; establishes the MVP single-query contract
- **US2 (P2)**: Can begin after Foundational, but should land after the `useLiveQuery` contract from US1 is stable
- **US3 (P3)**: Can begin after Foundational, but relies on the single-query contract and mutation primitives established by US1
- **US4 (P3)**: Depends on US3 because assistant workflows require multi-query orchestration and derived selections
- **US5 (P3)**: Depends on US4 because the example must validate the finished assistant-friendly package surface in a chat app

### Within Each User Story

- Tests must be written before or alongside implementation and should fail against the missing behavior first
- Hook and shared-state logic should land before the component wrappers that consume it
- Mutation routing should land before examples or Admin UI demos that depend on it
- Examples and demos should follow the core story implementation so the docs reflect the actual API

### Parallel Opportunities

- **Setup**: `T003` and `T004` can run in parallel after the package scaffold exists
- **Foundational**: `T007`, `T008`, and `T009` can run in parallel once `T005` and `T006` define the shared controller contract
- **US1**: `T010` and `T011` can run in parallel because they touch different test surfaces
- **US2**: `T016` and `T017` can run in parallel before implementation begins
- **US3**: `T022` and `T023` can run in parallel; `T024`–`T026` should precede `T027`
- **US4**: `T028` and `T029` can run in parallel; `T030` and `T031` should precede the Admin UI demo and example
- **US5**: `T034` can start first; `T035` and `T036` establish the example surface before `T037`–`T040`
- **Polish**: `T041`, `T042`, `T043`, and `T046` can run in parallel before the focused validation tasks `T044` and `T045`

---

## Parallel Example: Foundational Phase

```bash
# After T005-T006 define the shared controller contract, these can run in parallel:
Task: "Extend Drizzle live descriptor compilation and typed row-key extraction in link/sdks/typescript/orm/src/live.ts, link/sdks/typescript/orm/src/sql.ts, and link/sdks/typescript/orm/src/index.ts"
Task: "Create shared React provider and mutation plumbing in link/sdks/typescript/react/src/context.tsx, link/sdks/typescript/react/src/hooks/useMutationState.ts, link/sdks/typescript/react/src/types.ts, and link/sdks/typescript/react/src/index.ts"
Task: "Add foundational shared-core coverage in link/sdks/typescript/client/tests/live-controller.test.mjs, link/sdks/typescript/orm/tests/live-descriptor.test.mjs, and link/sdks/typescript/react/tests/test-utils.tsx"
```

## Parallel Example: User Story 3

```bash
# These test tasks can run together before the multi-query implementation work:
Task: "Add multi-query hook and component tests in link/sdks/typescript/react/tests/live-queries.test.tsx"
Task: "Add Admin UI multi-query integration coverage in ui/src/components/live-data/ReactLiveQueryDemo.test.tsx"
```

## Parallel Example: User Story 5

```bash
# After T035-T036 establish the example package and schema, these can run in parallel:
Task: "Build the conversation shell and sidebar navigation in examples/react-ai-chat/src/App.tsx, examples/react-ai-chat/src/main.tsx, examples/react-ai-chat/src/styles.css, and examples/react-ai-chat/src/components/ConversationSidebar.tsx"
Task: "Implement conversation history loading, multi-file message send, typing indicator, and message edit/cancel actions in examples/react-ai-chat/src/components/ConversationHistory.tsx, examples/react-ai-chat/src/components/ChatComposer.tsx, examples/react-ai-chat/src/components/TypingIndicator.tsx, and examples/react-ai-chat/src/components/MessageActions.tsx"
```

## Parallel Example: Polish Phase

```bash
# These documentation tasks can run in parallel before final validation:
Task: "Update SDK package docs in link/sdks/typescript/react/README.md, link/sdks/typescript/client/README.md, link/sdks/typescript/orm/README.md, and link/sdks/typescript/README.md"
Task: "Update repo SDK docs and quickstart references in docs/sdk/sdk.md and specs/030-react-live-queries/quickstart.md"
Task: "Sync external SDK docs in ../KalamSite/content/sdk/react-live-query.md, ../KalamSite/content/sdk/react-assistant-workflow.md, and ../KalamSite/content/sdk/react-ai-chat-example.md"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Confirm typed single-query screens load, reconnect, and mutate with low boilerplate
5. Demo the new single-query React surface before expanding scope

### Incremental Delivery

1. Finish Setup + Foundational so the repo has one reusable live-query controller and React package shell
2. Deliver US1 to establish the typed single-query API and mutation state contract
3. Deliver US2 to add SQL-first single-query support
4. Deliver US3 to add named multi-query composition and Admin UI pilot usage
5. Deliver US4 to validate assistant-style workflows and derived selections
6. Deliver US5 to validate the package with the standalone `examples/react-ai-chat` app
7. Finish docs sync and focused validation

### Parallel Team Strategy

With multiple developers:

1. One developer completes the shared client/controller work in Phase 2
2. One developer prepares the React provider/test harness in parallel during Phase 2
3. After US1 stabilizes the single-query contract:
   - Developer A: US2 raw SQL mode
   - Developer B: US3 multi-query composition
4. Once US3 is stable, Developer C can complete US4 assistant workflows and Admin UI demos
5. After US4 stabilizes the package surface, Developer D can build `examples/react-ai-chat`

### Suggested MVP Scope

**MVP = Phase 1 + Phase 2 + Phase 3 (User Story 1)**

This delivers:

- Typed React live-query screens
- Loading, reconnect, refetch, and mutation state
- Provider-based client wiring
- A concrete example developers can copy into an app

---

## Notes

- `[P]` tasks are safe to parallelize because they touch different files or begin after an earlier contract-defining task in the same phase
- The React package should stay hook-first for advanced screens and treat component wrappers as convenience APIs
- `select` and derived selection helpers should remain pure projections over authoritative live state, not a second store
- SDK documentation changes under `link/sdks/**` must also sync to `../KalamSite/content/sdk/` per repo guidance
- `examples/react-ai-chat` should reuse the proven agent topology from `examples/chat-with-ai` and keep browser-side streaming/history wiring serialized enough to avoid overlapping SDK refresh/setup races
- Total tasks: 46
- User story task counts: US1 = 6, US2 = 6, US3 = 6, US4 = 6, US5 = 7