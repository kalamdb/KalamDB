# Data Model: @kalamdb/react LiveQuery & LiveQueries

## Overview

The feature introduces a small set of runtime entities that describe live datasets, their current materialized results, the UI-visible mutation state layered on top of authoritative live rows, and derived screen-ready views for complex workflows such as AI assistants.

## Entities

### LiveQueryDescriptor

- **Purpose**: Defines one live dataset and the rules required to fetch, subscribe, project, and key its rows.
- **Fields**:
  - `mode`: `drizzle` or `sql`
  - `name`: optional stable key for multi-query collections
  - `table`: Drizzle table reference when `mode=drizzle`
  - `where`: optional typed filter expression for Drizzle mode
  - `orderBy`: optional typed or normalized ordering descriptor
  - `query`: raw SQL string when `mode=sql`
  - `limit`: optional client-side row cap after materialization
  - `getKey`: row-key strategy used for reconciliation and row-level mutation state
  - `subscriptionSql`: normalized live-safe SQL used by the shared controller
  - `projectionPlan`: client-side sort/limit/projection instructions derived from the original definition
- **Validation rules**:
  - Exactly one query mode may be supplied.
  - Drizzle mode requires `table`.
  - SQL mode requires `query`.
  - Multi-query v1 accepts only Drizzle-mode descriptors.
  - Raw SQL v1 must resolve to a live-compatible subset or fail fast with a validation error.

### LiveQuerySession

- **Purpose**: Represents the runtime state for one active live dataset.
- **Fields**:
  - `rows`: current materialized rows after projection
  - `loading`: whether the first stable result is still pending
  - `connected`: whether the underlying live controller is currently connected
  - `error`: last terminal or visible recoverable error
  - `lastSeqId`: latest resume checkpoint emitted by the controller
  - `status`: `idle`, `loading`, `live`, `reconnecting`, `error`, or `disposed`
  - `refetchNonce`: monotonic value or trigger used to force a fresh load
- **State transitions**:
  - `idle -> loading` when the component mounts or a definition changes
  - `loading -> live` after the initial result and live subscription are both ready
  - `live -> reconnecting` on connection loss after an established session
  - `reconnecting -> live` after resume succeeds
  - `loading|live|reconnecting -> error` when setup or recovery fails visibly
  - `* -> disposed` on unmount or explicit teardown

### MutationTracker

- **Purpose**: Exposes UI-facing progress for create, update, and delete operations without making local rows authoritative.
- **Fields**:
  - `inserting`: boolean flag for in-flight create operations scoped to one live context
  - `updating`: set of row keys currently being updated
  - `deleting`: set of row keys currently being deleted
  - `error`: last mutation error visible to the render context
  - `lastCompletedAt`: optional timestamp for consumers that want transient success cues
- **State transitions**:
  - `idle -> inserting` on create start
  - `idle -> updating(rowKey)` on update start
  - `idle -> deleting(rowKey)` on delete start
  - `inserting|updating|deleting -> idle` on success
  - `inserting|updating|deleting -> idle + error` on failure

### SingleQueryRenderContext

- **Purpose**: Public render payload exposed by `LiveQuery`.
- **Fields**:
  - `rows`: materialized query rows
  - `state`: live session state merged with mutation tracker state
  - `insert`: create helper
  - `update`: update helper
  - `remove`: delete helper
  - `refetch`: explicit reload helper
- **Relationships**:
  - Wraps exactly one `LiveQuerySession`
  - Owns exactly one `MutationTracker`

### LiveSelection

- **Purpose**: Represents a pure derived view built from one or more live query contexts so application code can expose screen-ready state without duplicating rows into separate local state.
- **Fields**:
  - `source`: `single` or `multi`
  - `selector`: pure projection function over current live context
  - `selectedValue`: derived screen-ready output
  - `dependencies`: implicit dependency on the current query rows and state already held by the live context
- **Validation rules**:
  - Selection must be pure and side-effect free.
  - Selection cannot become a second authoritative store for query rows.
  - Selections may combine multiple datasets for views such as `pendingApprovals`, `activeToolCalls`, `typingUsers`, or `onlineParticipants`.

### MultiQueryDefinition

- **Purpose**: Named collection of `LiveQueryDescriptor` objects for `LiveQueries`.
- **Fields**:
  - `queries`: map of query name to typed descriptor
  - `sharedClient`: shared client reference or provider context
  - `defaultMutationTarget`: optional fallback target for generic mutation helpers
  - `selection`: optional pure derived projection for screen-ready state
- **Validation rules**:
  - Query names must be unique and stable across renders.
  - Each named definition must satisfy `LiveQueryDescriptor` rules.
  - Initial release restricts entries to Drizzle mode.
  - Definitions must support assistant-style groupings such as `messages`, `toolCalls`, `toolResults`, `typing`, `presence`, and `approvals` without changing the underlying live contract.

### MultiQueryRenderContext

- **Purpose**: Public render payload exposed by `LiveQueries`.
- **Fields**:
  - `[name]`: one single-query-like result object per named dataset
  - `state.loading`: true until every required initial dataset is ready
  - `state.connected`: true only when every active dataset is connected
  - `state.error`: first aggregate-visible error, while per-query errors remain available on each dataset
  - `insert`, `update`, `remove`: target-aware mutation helpers that route to the intended dataset
  - `selection`: optional derived screen-ready state returned by a pure selector for low-boilerplate screens
- **Relationships**:
  - Owns many `LiveQuerySession` objects
  - Aggregates many `MutationTracker` objects
  - May expose one `LiveSelection`

## Relationships

- A `MultiQueryDefinition` owns many `LiveQueryDescriptor` objects.
- Each `LiveQueryDescriptor` creates one `LiveQuerySession`.
- Each live render context layers one `MutationTracker` over one `LiveQuerySession`.
- `MultiQueryRenderContext` aggregates named single-query result objects plus top-level state.
- `LiveSelection` derives screen-ready state from either a single or multi-query context without becoming a second authoritative row store.

## Identity Rules

- Row-level mutation tracking depends on stable row keys.
- Default row identity uses `id` when available.
- Typed descriptors may derive keys from Drizzle metadata or a supplied `getKey` rule.
- Raw SQL mode must either expose a stable `id` field or require an explicit key strategy for row-level update/delete tracking.

## Error Model

- Query-definition validation errors occur before subscription starts.
- Live-session errors surface in query state and must not erase the last confirmed `rows` snapshot.
- Mutation failures clear in-flight state and set mutation-visible error information without mutating confirmed rows locally.
- Selection logic errors must surface as React-visible errors without corrupting the underlying live-query session state.

## Compatibility Boundaries

- Raw SQL v1 supports the live-compatible subset only.
- Client-side ordering and capping are part of the projection plan, not the backend subscription contract.
- Future Angular/Vue packages should be able to consume the same descriptor, session, and mutation models without React-specific fields.
- Assistant workflows are supported through generic named datasets and derived selections, not through assistant-specific hardcoded entities in the shared live contract.

## Example Validation Domain

These entities belong to the `examples/react-ai-chat` validation app rather than the generic package contract, but they drive the example requirements and task breakdown.

### ConversationThread

- **Purpose**: Represents one selectable chat conversation in the example sidebar.
- **Expected behavior**:
  - Stored in a USER table.
  - Supports listing existing conversations and creating a new conversation.
  - Selecting a conversation scopes all live queries and history loading to that thread.

### ConversationMessage

- **Purpose**: Represents one user or assistant message inside a conversation.
- **Expected behavior**:
  - Stored in a USER table.
  - Can include edit and cancel lifecycle state for user-authored messages.
  - Supports streamed assistant response progress and final committed reply content.

### MessageAttachment

- **Purpose**: Represents one uploaded file associated with a user message.
- **Expected behavior**:
  - Multiple attachments may be linked to one message send action.
  - The example must surface attachment upload failures without corrupting the parent message flow.

### TypingSignal

- **Purpose**: Represents typing state for the current conversation.
- **Expected behavior**:
  - Stored in a USER table.
  - Must remain scoped to the active conversation when the user switches threads.

### ToolCallActivity

- **Purpose**: Represents AI tool-call progress shown while the assistant is working on a reply.
- **Expected behavior**:
  - Exposed live in the conversation workspace.
  - Demonstrates that the package can render intermediate AI workflow state before the final assistant reply is stored.