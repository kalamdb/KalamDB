# Feature Specification: @kalamdb/react LiveQuery & LiveQueries

**Feature Branch**: `[030-react-live-queries]`  
**Created**: 2026-05-06  
**Status**: Draft  
**Input**: User description: "Feature Request: `@kalamdb/react` – LiveQuery & LiveQueries (Real-time React SDK)"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Build a single live screen (Priority: P1)

As a React developer building a real-time screen, I want to declare one live query and render directly from rows plus mutation state so I do not need to separately fetch data, subscribe for changes, merge updates, and manage loading state by hand.

**Why this priority**: Single-query live screens are the core value proposition of the package and the smallest independently useful slice of the feature.

**Independent Test**: Create a screen backed by one dataset, confirm initial rows load, confirm committed changes appear automatically, and trigger create, update, and delete actions while observing loading and mutation state.

**Acceptance Scenarios**:

1. **Given** a screen configured with one typed live query, **When** the screen loads, **Then** the current matching rows are rendered and loading state clears after the initial result arrives.
2. **Given** a single-query live screen is already open, **When** a matching row is inserted, updated, or deleted, **Then** the rendered rows update automatically without a manual refresh.
3. **Given** a user triggers create, update, or delete from the live query render context, **When** the operation is in progress, **Then** the UI can detect the corresponding mutation state and render feedback for that action.

---

### User Story 2 - Build a live screen from SQL (Priority: P2)

As a React developer who prefers SQL-first workflows, I want to declare a live screen from a SQL query string for the supported live-compatible subset so I can keep a SQL-first workflow without giving up the same real-time behavior and mutation experience.

**Why this priority**: Raw SQL mode is a key differentiator for KalamDB and broadens adoption beyond schema-first ORM users.

**Independent Test**: Configure a screen from a supported live-compatible SQL query string, verify the initial result set appears, verify live changes flow into the UI, and verify mutation state is surfaced during successful and failed operations.

**Acceptance Scenarios**:

1. **Given** a live screen configured from a supported live-compatible SQL query string, **When** the screen loads, **Then** the current result set is exposed through the same render-context shape used by the typed single-query mode where the capability applies.
2. **Given** a SQL-driven live screen is open, **When** underlying matching data changes, **Then** the visible result set stays synchronized automatically.
3. **Given** a SQL-driven live screen performs a mutation, **When** the mutation succeeds or fails, **Then** the UI receives updated mutation and error state without breaking the current live result set.

---

### User Story 3 - Compose multiple live datasets (Priority: P3)

As a React developer building collaborative or multi-panel screens, I want multiple live queries in one component so I can combine datasets such as messages, typing events, presence, or notifications without manually coordinating subscriptions.

**Why this priority**: Multi-query composition turns the single-query primitive into a complete screen-building pattern for real applications.

**Independent Test**: Build a screen with at least two named live datasets, confirm that each dataset renders independently, confirm aggregate state is exposed for the full screen, and verify that mutations against one dataset do not disrupt the others.

**Acceptance Scenarios**:

1. **Given** a component declares two named live queries, **When** both initial result sets load, **Then** the render context exposes each dataset separately and provides aggregate loading and connection state for the whole screen.
2. **Given** one query in a multi-query screen encounters an error, **When** the other queries remain healthy, **Then** the failing query state and the aggregate error state are both visible without hiding successful query results.
3. **Given** a multi-query screen includes mutation actions, **When** the user performs a mutation against one dataset, **Then** the correct dataset updates and the remaining datasets continue streaming normally.

---

### User Story 4 - Build an AI assistant workspace (Priority: P3)

As a React developer building an AI assistant UI, I want to compose messages, tool calls, tool outputs, typing indicators, presence, and human-approval queues through one ergonomic live-data API so I can build collaborative assistant experiences without manually wiring multiple subscriptions, mirror state, or effect-based merges.

**Why this priority**: AI assistant workflows are a high-value real-time use case for KalamDB and stress the package in exactly the way that exposes whether the API is truly ergonomic or only works for simple tables.

**Independent Test**: Build an assistant screen that combines at least messages, tool calls, typing, presence, and approvals; verify that rows stay synchronized, approval actions and tool-status updates are routable to the correct datasets, and the screen can derive pending approvals and active typing state without custom subscription-merging code.

**Acceptance Scenarios**:

1. **Given** a React assistant screen declares live datasets for messages, tool calls, typing, presence, and approvals, **When** the screen renders, **Then** the developer can read each dataset and aggregate state through one low-boilerplate API surface rather than manually coordinating separate subscriptions.
2. **Given** an assistant workflow needs derived UI state such as pending approvals, active tool calls, or currently typing users, **When** the developer builds the screen, **Then** the package provides a React-friendly composition path that avoids copying live rows into separate effect-managed local state.
3. **Given** a human reviewer approves or rejects a pending step while the assistant and tool calls continue running, **When** the mutation completes, **Then** the affected approval dataset updates without interrupting the rest of the live assistant screen.

---

### User Story 5 - Validate the package with a full chat app example (Priority: P3)

As a developer evaluating whether `@kalamdb/react` is sufficient for a production-style chat interface, I want a standalone `examples/react-ai-chat` example that exercises conversations, history loading, typing, tool activity, file uploads, and agent-driven replies so I can verify the package covers real chat-app workflows with low boilerplate.

**Why this priority**: A concrete example is the fastest way to prove the package is not only theoretically capable but actually ergonomic for chat applications, which are one of the primary targets for this feature.

**Independent Test**: Run the example browser app and agent worker, create or select a conversation from the sidebar, load its history, send a message with multiple files, observe typing and streamed reply updates, see tool-call progress, and edit or cancel a user message without breaking the rest of the conversation view.

**Acceptance Scenarios**:

1. **Given** the `examples/react-ai-chat` app is running, **When** the user creates or selects a conversation from the left sidebar, **Then** the conversation history loads and the live state shown on the screen switches to that conversation only.
2. **Given** a conversation is open, **When** the user sends a message with one or more files, **Then** the example persists the message, starts the agent workflow, and shows typing or streaming feedback while the assistant is generating a reply.
3. **Given** the assistant performs tool calls during a reply, **When** the tool workflow progresses, **Then** the example surfaces the tool activity in the conversation workspace without requiring custom live-subscription wiring outside the package primitives.
4. **Given** the user has already sent a message that triggered an in-progress assistant reply, **When** the user edits or cancels that message, **Then** the example demonstrates the corresponding live update path without resetting unrelated conversation state.

### Edge Cases

- What happens when a developer provides both a typed query definition and a raw SQL query for the same live query? The system must reject the ambiguous configuration with a clear error before streaming begins.
- How does the system handle a temporary connection loss after initial data has loaded? The UI must retain the last consistent result set, expose disconnected state, and resume live updates after connectivity returns.
- How are failed mutations surfaced? Failed create, update, and delete operations must clear their in-flight state, expose the failure, and leave previously confirmed rows unchanged.
- What happens when a live query returns no rows initially or after a change? The result must be represented as an empty collection rather than an error.
- How does multi-query behavior work when one query finishes loading later than the others? Aggregate loading must remain active until every required initial result set is ready, while each query still exposes its own status.
- How does the system handle very chatty assistant datasets such as typing and presence? The API must let developers consume those live rows without forcing effect-driven mirror state or deeply nested render-prop trees.
- How are human-approval queues handled when approvals are opened, reassigned, approved, or rejected while the same screen also streams tool activity and chat messages? The relevant rows must update independently without resetting unrelated live datasets.
- What happens when the user switches to another conversation while the assistant is still streaming a reply in the previous conversation? The example must keep each conversation's live state scoped correctly and load the selected conversation's history without leaking typing or tool activity across threads.
- How does the example handle multiple file uploads on a single user message when one file fails validation or upload? The example must surface the failure clearly without dropping the valid attachments or corrupting the conversation state.
- What happens when the user edits or cancels a message that already triggered tool activity? The example must demonstrate how related live rows update without breaking the rest of the conversation history.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide an official React package that lets developers declare real-time query-driven UI without manual subscription lifecycle management.
- **FR-002**: The system MUST support a single live query defined from a typed table-based query configuration and return the current ordered rows, query lifecycle state, mutation helpers, and a manual refetch action to the render function.
- **FR-003**: The system MUST support a single live query defined from a raw SQL query string and expose the same render-context contract used by the typed single-query mode wherever the capability is applicable.
- **FR-004**: The system MUST reject invalid single-query configurations where more than one query mode is supplied or where required query inputs are missing.
- **FR-005**: The system MUST keep a single live query result synchronized with committed database changes after the initial load without requiring the user to refresh manually.
- **FR-006**: The system MUST expose query lifecycle state that includes loading, connection status, and error information so the UI can represent first load, reconnection, and failure conditions.
- **FR-007**: The system MUST expose mutation helpers for create, update, and delete operations from a single live query context.
- **FR-008**: The system MUST track create operations with a query-level in-progress state and track update and delete operations with row-level in-progress state keyed by the affected row identity.
- **FR-009**: The system MUST clear in-progress mutation state after each operation finishes and MUST surface failures without leaving mutation state stuck.
- **FR-010**: The system MUST provide a multi-query component that can declare multiple named live queries within one screen and deliver each named result set separately to the render function.
- **FR-011**: The system MUST provide aggregate multi-query state that includes loading, connection, and error information for the overall screen in addition to per-query state.
- **FR-012**: In the initial release, the multi-query component MUST support typed table-based query definitions for each named query.
- **FR-013**: The system MUST allow mutation actions from a multi-query context and route each action to the correct target dataset without requiring custom subscription-merging logic.
- **FR-014**: The system MUST integrate with Drizzle schema types so row shapes and mutation inputs align with the developer's declared schema definitions.
- **FR-015**: The system MUST limit raw SQL live-query mode in v1 to the supported live-compatible subset, reapply ordering or limiting client-side where supported, and fail unsupported SQL shapes with a descriptive error instead of silently degrading behavior.
- **FR-016**: The system MUST be usable from KalamDB's Admin UI without requiring a separate synchronization layer for live data screens.
- **FR-017**: The release MUST include examples and usage documentation for a typed single-query flow, a SQL-based single-query flow, a multi-query flow, and mutation-state handling.
- **FR-018**: The feature MUST define a reusable live-query behavior contract that future KalamDB UI SDKs can mirror without changing the semantics of query results and mutation state.
- **FR-019**: The system MUST support assistant-style real-time workflows where multiple named datasets such as messages, tool calls, tool outputs, typing indicators, presence, and approval queues are composed into one screen without manual subscription lifecycle code.
- **FR-020**: The system MUST provide a React-friendly hook-based API in addition to component wrappers so complex screens can avoid render-prop nesting and follow normal React composition patterns.
- **FR-021**: The system MUST support deriving assistant-screen view state such as pending approvals, active tool calls, or currently typing users directly from live query state without requiring effect-based copying of rows into separate local state.
- **FR-022**: The release MUST include at least one documented assistant-workflow example that demonstrates tool activity, typing or presence, and human-in-the-loop approval handling with low boilerplate.
- **FR-023**: The release MUST include a standalone example at `examples/react-ai-chat` that uses `@kalamdb/react` as the primary live-data integration surface for a chat-style application.
- **FR-024**: The `examples/react-ai-chat` application MUST provide a left sidebar that lets the user create a new conversation or select an existing conversation and then load the selected conversation's history.
- **FR-025**: The `examples/react-ai-chat` application MUST support sending messages with multiple file attachments in a single user action.
- **FR-026**: The `examples/react-ai-chat` application MUST run an agent worker similar to `examples/chat-with-ai` that consumes a topic and produces assistant activity through KalamDB.
- **FR-027**: The `examples/react-ai-chat` application MUST use USER tables for conversations, messages, and typing indicators.
- **FR-028**: The `examples/react-ai-chat` application MUST display typing feedback and streamed assistant response progress while the AI is generating a reply.
- **FR-029**: The `examples/react-ai-chat` application MUST let the user edit or cancel a previously sent message from the message list while demonstrating the corresponding live update behavior.
- **FR-030**: The `examples/react-ai-chat` application MUST surface tool-calling activity during AI replies so the package is validated against tool-driven chat workflows.

### Key Entities *(include if feature involves data)*

- **Live Query Definition**: A declarative description of one real-time dataset, including the data source, any filters, ordering, and the mode used to describe the query.
- **Live Query Result**: The current ordered rows for a single query plus the query lifecycle state, mutation state, and actions exposed to the UI.
- **Mutation Action**: A create, update, or delete request initiated from the UI and tracked until it either succeeds or fails.
- **Multi-Query Collection**: A named set of live query definitions whose results are rendered together and share aggregate screen-level state.
- **Derived Live View**: A React-facing projection derived from one or more live query results, used for screen-ready state such as active typing users, pending approvals, or open tool calls without duplicating live rows into separate local state.
- **Conversation**: A chat thread that groups messages, typing rows, tool activity, and any other assistant workflow state for a single user-visible discussion.
- **Conversation Message**: A user or assistant message within a conversation, including edit/cancel lifecycle and any attached files.
- **Message Attachment**: A file linked to a user message and transmitted to the assistant workflow as part of a single send action.
- **Tool Activity**: Live status emitted while the agent performs tool calls for a conversation, used to explain what the AI is doing before the final reply completes.

## Assumptions

- Host applications already provide authenticated access to KalamDB through the standard client configuration used across the product.
- The first release targets React web applications and KalamDB's Admin UI; equivalent packages for other UI frameworks are out of scope for this feature.
- The first release of the multi-query component supports typed table-based query definitions; expanding multi-query support to raw SQL is a future enhancement.
- Live screens preserve the most recently confirmed data during brief connectivity interruptions rather than clearing the UI immediately.
- The package remains domain-agnostic: it will not ship assistant-specific UI widgets, but it will provide generic live composition primitives that make assistant workflows ergonomic.
- The standalone `examples/react-ai-chat` example is intentionally more feature-complete than `examples/chat-with-ai` and acts as a coverage check for package ergonomics rather than a minimal demo.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can build a working single-query real-time screen from the published documentation in 15 minutes or less without writing custom subscription coordination code.
- **SC-002**: In supported demo and admin UI scenarios, committed inserts, updates, and deletes appear in affected live screens within 2 seconds without a manual page refresh.
- **SC-003**: All published example flows cover the six core scenarios of the release: typed single query, SQL-based single query for the live-compatible subset, multi-query composition, mutation-state-driven UI feedback, assistant-workflow composition, and the standalone `examples/react-ai-chat` validation app.
- **SC-004**: A collaborative screen that combines at least two live datasets can be implemented without custom code to merge or coordinate separate subscriptions.
- **SC-005**: In release validation, mutation-state indicators accurately reflect operation progress and completion for 100% of tested create, update, and delete interactions.
- **SC-006**: A documented AI assistant screen combining messages, tool activity, typing or presence, and human approvals can be implemented without custom subscription-merging logic or effect-driven mirror state.
- **SC-007**: A developer can run `examples/react-ai-chat`, create or select a conversation, send a multi-file message, observe streamed AI replies and tool activity, and edit or cancel a user message without writing custom live synchronization code.
