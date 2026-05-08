import { LiveQuery } from '@kalamdb/react';
import { asc, eq } from 'drizzle-orm';
import { messages } from './schema.js';

export function MessagesPane({ conversationId }: { conversationId: string }) {
  return (
    <LiveQuery
      table={messages}
      where={(table) => eq(table.conversationId, conversationId)}
      orderBy={(table) => asc(table.createdAt)}
      limit={100}
    >
      {({ rows, state, insert }) => (
        <section>
          {state.loading ? <p>Loading...</p> : null}
          {rows.map((message) => (
            <article key={message.id}>{message.body}</article>
          ))}
          <button
            disabled={state.inserting}
            onClick={() => insert(messages).values({ conversationId, body: 'Hello from KalamDB' })}
          >
            Send
          </button>
        </section>
      )}
    </LiveQuery>
  );
}