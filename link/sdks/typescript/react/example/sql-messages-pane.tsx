import { LiveQuery } from '@kalamdb/react';

export function SqlMessagesPane({ conversationId }: { conversationId: string }) {
  return (
    <LiveQuery
      query={`SELECT * FROM chat.messages WHERE conversation_id = '${conversationId}' ORDER BY created_at ASC LIMIT 100`}
      getKey="id"
    >
      {({ rows, state, insert }) => (
        <section>
          {state.error ? <p>{state.error.message}</p> : null}
          {rows.map((message) => (
            <article key={message.id.asString()}>{message.body.asString()}</article>
          ))}
          <button
            disabled={state.inserting}
            onClick={() => insert('chat.messages', { conversation_id: conversationId, body: 'Hello from SQL mode' })}
          >
            Send
          </button>
        </section>
      )}
    </LiveQuery>
  );
}