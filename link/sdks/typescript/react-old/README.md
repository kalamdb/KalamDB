# @kalamdb/react

React bindings for KalamDB live queries. The package exposes hook-first APIs plus thin component wrappers over the shared `@kalamdb/client` live controller.

## Install

```bash
npm install @kalamdb/react @kalamdb/client react react-dom
```

Typed Drizzle query mode also needs `@kalamdb/orm` and `drizzle-orm`:

```bash
npm install @kalamdb/orm drizzle-orm
```

## Basic Usage

```tsx
import { KalamProvider, LiveQuery } from '@kalamdb/react';
import { createClient } from '@kalamdb/client';

const client = createClient({ url: 'http://localhost:8080' });

export function App() {
  return (
    <KalamProvider client={client}>
      <LiveQuery query="SELECT * FROM chat.messages WHERE room = 'main' ORDER BY created_at ASC">
        {({ rows, state, insert }) => (
          <section>
            {state.loading ? <p>Loading...</p> : null}
            {rows.map((row) => <p key={row.id.asString()}>{row.body.asString()}</p>)}
            <button
              disabled={state.inserting}
              onClick={() => insert('chat.messages', { room: 'main', body: 'Hello' })}
            >
              Send
            </button>
          </section>
        )}
      </LiveQuery>
    </KalamProvider>
  );
}
```

Raw SQL live mode supports the live-compatible subset in v1. Ordering and limits are normalized into client-side projections where possible; unsupported SQL shapes fail with a descriptive error.

## Typed Live Queries

Use typed mode when you already have a Drizzle schema from `@kalamdb/orm`.

```tsx
import { LiveQuery } from '@kalamdb/react';
import { asc, eq } from 'drizzle-orm';
import { messages } from './schema.generated';

export function MessagesPane({ conversationId }: { conversationId: string }) {
  return (
    <LiveQuery
      table={messages}
      where={(table) => eq(table.conversationId, conversationId)}
      orderBy={(table) => asc(table.createdAt)}
      deps={[conversationId]}
    >
      {({ rows, state, insert }) => (
        <section>
          {rows.map((row) => <article key={row.id}>{row.body}</article>)}
          <button
            disabled={state.inserting}
            onClick={() => insert(messages).values({
              id: crypto.randomUUID(),
              conversationId,
              role: 'user',
              body: 'Hello',
              status: 'sent',
              createdAt: new Date(),
              updatedAt: new Date(),
            })}
          >
            Send
          </button>
        </section>
      )}
    </LiveQuery>
  );
}
```

## Multiple Live Datasets

`useLiveQueries` and `LiveQueries` open one controller per named query and return a typed context for each dataset plus aggregate mutation and connection state.

```tsx
import { useLiveQueries, useLiveSelection } from '@kalamdb/react';
import { asc, eq } from 'drizzle-orm';
import { approvals, messages, toolCalls, typing } from './schema.generated';

export function AssistantWorkspace({ conversationId }: { conversationId: string }) {
  const live = useLiveQueries({
    queries: {
      messages: {
        table: messages,
        where: (table) => eq(table.conversationId, conversationId),
        orderBy: (table) => asc(table.createdAt),
        deps: [conversationId],
      },
      typing: { table: typing, where: (table) => eq(table.conversationId, conversationId), deps: [conversationId] },
      toolCalls: { table: toolCalls, where: (table) => eq(table.conversationId, conversationId), deps: [conversationId] },
      approvals: { table: approvals, where: (table) => eq(table.conversationId, conversationId), deps: [conversationId] },
    },
    deps: [conversationId],
  });

  const assistant = useLiveSelection(live, (context) => ({
    messages: context.messages.rows,
    typingUsers: context.typing.rows.map((row) => row.userName),
    activeTools: context.toolCalls.rows.filter((row) => row.status !== 'completed'),
    pendingApprovals: context.approvals.rows.filter((row) => row.status === 'pending'),
    approve: (approvalId: string) => context.update(approvals, approvalId).set({ status: 'approved' }),
  }));

  return <AssistantLayout {...assistant} busy={live.state.loading || live.state.updating} />;
}
```

Rows remain authoritative from KalamDB live streams. Mutation state is local UI state for disabling buttons, showing spinners, and surfacing errors; it does not mutate row data optimistically.

## React AI Chat Example

The repository includes a standalone validation app at [../../../../examples/react-ai-chat](../../../../examples/react-ai-chat). It demonstrates conversation navigation, history loading, multi-file messages, typing state, streamed assistant activity, tool calls, and human approvals using `@kalamdb/react`.

```bash
cd examples/react-ai-chat
npm install
npm run setup
npm run dev
```

The example defaults to demo mode so the React components are immediately usable without a server. Set `VITE_KALAMDB_DEMO_MODE=false` after applying `chat-app.sql` to a running KalamDB server.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See [../../../../LICENSE.txt](../../../../LICENSE.txt) and [../../../../NOTICE](../../../../NOTICE).