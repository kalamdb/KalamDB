# Quickstart: `@kalamdb/react`

## 1. Build the local SDK packages during development

From the repo root, build the shared packages before consuming the React package locally:

```bash
cd link/sdks/typescript/client && npm install && npm run build
cd ../orm && npm install && npm run build
cd ../react && npm install && npm run build
```

For the Admin UI, keep using the existing local SDK consumption pattern so Vite resolves the built package output reliably during local development.

## 2. Provide a `KalamDBClient` to React

```tsx
import { KalamProvider } from '@kalamdb/react';
import { getClient } from './lib/kalam-client';

export function AppShell() {
  const client = getClient();
  if (!client) return null;

  return (
    <KalamProvider client={client}>
      <App />
    </KalamProvider>
  );
}
```

`client` props remain available for consumers that prefer explicit wiring per component.

## 3. Use typed single-query mode

```tsx
import { LiveQuery } from '@kalamdb/react';
import { eq, asc } from 'drizzle-orm';
import { messageTable } from './schema';

export function MessagesPane() {
  return (
    <LiveQuery
      table={messageTable}
      where={(m) => eq(m.conversationId, 1)}
      orderBy={(m) => asc(m.createdAt)}
    >
      {({ rows, state, insert }) => (
        <section>
          {state.loading ? <p>Loading…</p> : null}
          {rows.map((row) => <div key={row.id}>{row.content}</div>)}
          <button
            disabled={state.inserting}
            onClick={() => insert(messageTable).values({ conversationId: 1, content: 'Hello' })}
          >
            Send
          </button>
        </section>
      )}
    </LiveQuery>
  );
}
```

## 4. Use raw SQL single-query mode

```tsx
import { LiveQuery } from '@kalamdb/react';

export function SqlMessagesPane() {
  return (
    <LiveQuery query="SELECT * FROM chat.messages WHERE conversation_id = 1 ORDER BY created_at ASC">
      {({ rows, state, insert }) => (
        <section>
          {state.error ? <p>{state.error.message}</p> : null}
          {rows.map((row) => <div key={String(row.id)}>{String(row.content)}</div>)}
          <button
            disabled={state.inserting}
            onClick={() => insert('chat.messages', { conversation_id: 1, content: 'Hi' })}
          >
            Send
          </button>
        </section>
      )}
    </LiveQuery>
  );
}
```

For v1, raw SQL live mode is limited to the live-compatible subset supported by the shared client controller. Ordering and row caps may be reapplied client-side when the SQL can be normalized safely.

## 5. Compose multiple live datasets

```tsx
import { LiveQueries } from '@kalamdb/react';
import { eq, asc } from 'drizzle-orm';
import { messageTable, typingTable } from './schema';

export function ChatScreen() {
  return (
    <LiveQueries
      queries={{
        messages: {
          table: messageTable,
          where: (m) => eq(m.conversationId, 1),
          orderBy: (m) => asc(m.createdAt),
        },
        typing: {
          table: typingTable,
          where: (t) => eq(t.conversationId, 1),
        },
      }}
    >
      {({ messages, typing, state, insert }) => (
        <section>
          {state.loading ? <p>Loading chat…</p> : null}
          {messages.rows.map((row) => <div key={row.id}>{row.content}</div>)}
          <p>{typing.rows.length > 0 ? 'Typing…' : null}</p>
          <button
            onClick={() => insert(typingTable).values({ conversationId: 1, userName: 'Jamal' })}
          >
            Send typing event
          </button>
        </section>
      )}
    </LiveQueries>
  );
}
```

For simple declarative screens, the component wrappers remain fine. For larger screens, prefer hooks so composition stays flat and React-friendly.

## 6. Build an AI assistant workflow with low boilerplate

```tsx
import { useLiveQueries } from '@kalamdb/react';
import { eq, asc, desc } from 'drizzle-orm';
import {
  messageTable,
  toolCallTable,
  toolResultTable,
  typingTable,
  presenceTable,
  approvalTable,
} from './schema';

export function AssistantWorkspace({ threadId }: { threadId: string }) {
  const assistant = useLiveQueries({
    queries: {
      messages: {
        table: messageTable,
        where: (m) => eq(m.threadId, threadId),
        orderBy: (m) => asc(m.createdAt),
      },
      toolCalls: {
        table: toolCallTable,
        where: (t) => eq(t.threadId, threadId),
        orderBy: (t) => desc(t.createdAt),
      },
      toolResults: {
        table: toolResultTable,
        where: (t) => eq(t.threadId, threadId),
        orderBy: (t) => desc(t.createdAt),
      },
      typing: {
        table: typingTable,
        where: (t) => eq(t.threadId, threadId),
      },
      presence: {
        table: presenceTable,
        where: (p) => eq(p.threadId, threadId),
      },
      approvals: {
        table: approvalTable,
        where: (a) => eq(a.threadId, threadId),
        orderBy: (a) => desc(a.createdAt),
      },
    },
    select: ({ messages, toolCalls, toolResults, typing, presence, approvals, state, update }) => ({
      messages: messages.rows,
      activeToolCalls: toolCalls.rows.filter((row) => row.status !== 'completed'),
      latestToolResults: toolResults.rows,
      typingUsers: typing.rows.map((row) => row.userName),
      onlineUsers: presence.rows.filter((row) => row.status === 'online'),
      pendingApprovals: approvals.rows.filter((row) => row.status === 'pending'),
      busy: state.loading || messages.state.inserting,
      approve: (approvalId: string) => update(approvalTable, approvalId).set({ status: 'approved' }),
      reject: (approvalId: string) => update(approvalTable, approvalId).set({ status: 'rejected' }),
    }),
  });

  return (
    <AssistantLayout
      busy={assistant.busy}
      messages={assistant.messages}
      activeToolCalls={assistant.activeToolCalls}
      latestToolResults={assistant.latestToolResults}
      typingUsers={assistant.typingUsers}
      onlineUsers={assistant.onlineUsers}
      pendingApprovals={assistant.pendingApprovals}
      onApprove={assistant.approve}
      onReject={assistant.reject}
    />
  );
}
```

This pattern keeps the live rows authoritative, derives screen-ready assistant state inline, and avoids copying live data into extra `useEffect`-managed local state.

## 7. Run the standalone `examples/react-ai-chat` validation app

The package release ships with a more complete validation app in `examples/react-ai-chat`.

Run it locally:

```bash
cd examples/react-ai-chat
npm install
npm run setup
npm run dev
```

The example defaults to demo mode so the React components can be tried without a server. To test the server-backed path, apply `chat-app.sql`, set `VITE_KALAMDB_DEMO_MODE=false`, and run `npm run agent` in another terminal.

The example demonstrates:

- a left sidebar that lists conversations and lets the user create a new conversation
- history loading when the user selects a conversation
- multi-file user messages
- a topic-consuming agent worker similar to `examples/chat-with-ai`
- typing feedback and streamed assistant replies while the AI is still responding
- message edit/cancel actions for user-authored messages
- visible tool-call activity while the AI is using tools

The example uses USER tables for conversations, messages, attachments, typing indicators, tool calls, tool results, and approvals so the package is validated against the intended app-development model.

## 8. Validate Admin UI adoption

- Consume the package through the existing singleton/client lifecycle in `ui/src/lib/kalam-client.ts`.
- Add one Admin UI example or pilot component that renders at least one `LiveQuery`, one `LiveQueries` scenario, and one assistant-style multi-query scenario.
- Cover the UI integration path with Vitest + React Testing Library so the package contract is proven in the actual browser app.

## 9. Document and publish

- Update package README files for `@kalamdb/client`, `@kalamdb/orm`, and `@kalamdb/react`.
- Update repo-side SDK docs under `docs/sdk/` where usage examples are surfaced.
- Update the corresponding KalamSite SDK docs as required by repo policy for SDK changes.
- Include one assistant-workflow documentation example covering tool activity, typing or presence, and human approval handling.
- Include dedicated documentation for `examples/react-ai-chat` covering the conversation sidebar, history loading, file uploads, streamed replies, edit/cancel actions, and tool-calling demonstration.