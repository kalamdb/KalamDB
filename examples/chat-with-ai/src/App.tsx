import { useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import {
  Auth,
  ChangeType,
  MessageType,
  type RowData,
  createClient,
  type SubscriptionErrorEvent,
} from '@kalamdb/client';
import { eq } from 'drizzle-orm';
import { kalamDriver, liveTable } from '@kalamdb/orm';
import { drizzle } from 'drizzle-orm/pg-proxy';
import {
  chat_demo_agent_eventsConfig as agentEventsConfig,
  chat_demo_messages as chatMessages,
  type ChatDemoAgentEvents as AgentEventRow,
  type ChatDemoMessages as ChatMessageRow,
} from './schema.generated';
import './styles.css';

type LiveDraft = {
  stage: 'thinking' | 'typing' | 'saving';
  label: string;
  preview: string;
};

const MAX_CHAT_MESSAGES = 80;
const MAX_AGENT_EVENTS = agentEventsConfig.tableType === 'stream' ? 40 : 20;
const canSortEventsBySeq = agentEventsConfig.systemColumns.includes('_seq');

const ROOM = import.meta.env.VITE_CHAT_ROOM ?? 'main';
const CHAT_USERNAME = import.meta.env.VITE_KALAMDB_USER ?? 'admin';
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});

function resolveBrowserWasmUrl(): string {
  const base = '/wasm/kalam_client_bg.wasm';
  return import.meta.env.DEV ? `${base}?t=${Date.now()}` : base;
}

function createAuthedClient() {
  return createClient({
    url: import.meta.env.VITE_KALAMDB_URL ?? 'http://127.0.0.1:8080',
    authProvider: async () => Auth.basic(
      CHAT_USERNAME,
      import.meta.env.VITE_KALAMDB_PASSWORD ?? 'kalamdb123',
    ),
    disableCompression: true,
    wasmUrl: resolveBrowserWasmUrl(),
  });
}

const client = createAuthedClient();
const db = drizzle(kalamDriver(client));

function formatCreatedAt(createdAt: Date): string {
  return Number.isNaN(createdAt.getTime()) ? 'Invalid date' : timeFormatter.format(createdAt);
}

function eventTimelineKey(row: { id: string; _seq?: string | null; created_at: Date }): string {
  return canSortEventsBySeq && row._seq ? row._seq : `${row.created_at.toISOString()}:${row.id}`;
}

function sortEvents(rows: AgentEventRow[]): AgentEventRow[] {
  return [...rows].sort(
    (left, right) => left.created_at.getTime() - right.created_at.getTime()
      || eventTimelineKey(left).localeCompare(eventTimelineKey(right))
      || left.id.localeCompare(right.id),
  );
}

function limitEvents(rows: AgentEventRow[]): AgentEventRow[] {
  // Change frames are merged locally, so keep a deterministic order after upserts/removals.
  const sorted = sortEvents(rows);
  return sorted.length > MAX_AGENT_EVENTS ? sorted.slice(-MAX_AGENT_EVENTS) : sorted;
}

function upsertEvents(current: AgentEventRow[], incoming: AgentEventRow[]): AgentEventRow[] {
  const next = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) {
    next.set(event.id, event);
  }
  return limitEvents(Array.from(next.values()));
}

function removeEvents(current: AgentEventRow[], removed: AgentEventRow[]): AgentEventRow[] {
  const removedIds = new Set(removed.map((event) => event.id));
  return limitEvents(current.filter((event) => !removedIds.has(event.id)));
}

function deriveLiveDraft(events: AgentEventRow[]): LiveDraft | null {
  let activeEvent: AgentEventRow | null = null;

  for (const event of events) {
    if (event.stage === 'thinking' || event.stage === 'typing' || event.stage === 'message_saved') {
      activeEvent = event;
    }

    if (event.stage === 'complete' && activeEvent?.response_id === event.response_id) {
      activeEvent = null;
    }
  }

  if (!activeEvent) {
    return null;
  }

  if (activeEvent.stage === 'thinking') {
    return {
      stage: 'thinking',
      label: 'KalamDB Copilot is thinking',
      preview: 'Planning the reply and preparing the first streamed tokens…',
    };
  }

  if (activeEvent.stage === 'message_saved') {
    return {
      stage: 'saving',
      label: 'KalamDB Copilot is committing the reply',
      preview: activeEvent.preview,
    };
  }

  return {
    stage: 'typing',
    label: 'KalamDB Copilot is streaming characters',
    preview: activeEvent.preview,
  };
}

function deriveFallbackDraft(messages: ChatMessageRow[]): LiveDraft | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === 'assistant') {
      return null;
    }

    if (message.role === 'user') {
      return {
        stage: 'thinking',
        label: 'KalamDB Copilot is preparing the reply',
        preview: 'AI reply: waiting for live agent events...',
      };
    }
  }

  return null;
}

function sqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function mapAgentEventRow(row: RowData): AgentEventRow {
  return {
    _seq: row._seq?.asSeqId()?.toString() ?? row._seq?.asString() ?? null,
    id: row.id?.asString() ?? '',
    response_id: row.response_id?.asString() ?? '',
    room: row.room?.asString() ?? '',
    sender_username: row.sender_username?.asString() ?? '',
    stage: row.stage?.asString() ?? '',
    preview: row.preview?.asString() ?? '',
    message: row.message?.asString() ?? '',
    created_at: row.created_at?.asDate() ?? new Date(0),
  };
}

function mapAgentEventRows(rows: RowData[] | undefined): AgentEventRow[] {
  return (rows ?? []).map(mapAgentEventRow);
}

export function App() {
  const [messages, setMessages] = useState<ChatMessageRow[]>([]);
  const [events, setEvents] = useState<AgentEventRow[]>([]);
  const [draft, setDraft] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [status, setStatus] = useState<'connecting' | 'live' | 'error'>('connecting');
  const [error, setError] = useState<string | null>(null);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const liveDraft = deriveLiveDraft(events) ?? deriveFallbackDraft(messages);

  useEffect(() => {
    let active = true;
    let bufferedEvents: AgentEventRow[] = [];
    const unsubscribers: Array<() => Promise<void>> = [];

    const publishEvents = (nextEvents: AgentEventRow[]): void => {
      bufferedEvents = nextEvents;
      flushSync(() => {
        setEvents(nextEvents);
      });
    };

    const handleEventSubscription = (event: { type: string; rows?: RowData[]; old_values?: RowData[]; code?: string; message?: string; change_type?: string }): void => {
      if (!active) {
        return;
      }

      if (event.type === MessageType.Error) {
        setStatus('error');
        setError(`Event subscription failed (${event.code}): ${event.message}`);
        return;
      }

      if (event.type === MessageType.SubscriptionAck) {
        return;
      }

      if (event.type === MessageType.InitialDataBatch) {
        publishEvents(upsertEvents(bufferedEvents, mapAgentEventRows(event.rows)));
        return;
      }

      if (event.type !== MessageType.Change) {
        return;
      }

      if (event.change_type === ChangeType.Delete) {
        publishEvents(removeEvents(bufferedEvents, mapAgentEventRows(event.old_values)));
        return;
      }

      let nextEvents = bufferedEvents;
      if (event.change_type === ChangeType.Update) {
        nextEvents = removeEvents(nextEvents, mapAgentEventRows(event.old_values));
      }

      publishEvents(upsertEvents(nextEvents, mapAgentEventRows(event.rows)));
    };

    const start = async (): Promise<void> => {
      try {
        const messagesUnsubscribe = await liveTable(
          client,
          chatMessages,
          (nextMessages) => {
            if (active) {
              // Keep the materialized live query in server order.
              setMessages(nextMessages);
            }
          },
          {
            where: eq(chatMessages.room, ROOM),
            // `lastRows` asks the server for a rewind window at subscribe time.
            // `limit` keeps the materialized client-side live state bounded
            // after that rewind and across later live changes.
            limit: MAX_CHAT_MESSAGES,
            lastRows: MAX_CHAT_MESSAGES,
            onError: (event: SubscriptionErrorEvent) => {
              if (!active) {
                return;
              }
              setStatus('error');
              setError(`Message subscription failed (${event.code}): ${event.message}`);
            },
          },
        );
        unsubscribers.push(messagesUnsubscribe);

        // The draft rail keeps raw protocol frames so rapid typing bursts can
        // be reconciled locally instead of waiting for a full live-row view.
        const eventsUnsubscribe = await client.liveEvents(
          `SELECT * FROM chat_demo.agent_events WHERE room = ${sqlLiteral(ROOM)}`,
          handleEventSubscription,
          {
            lastRows: MAX_AGENT_EVENTS,
          },
        );
        unsubscribers.push(eventsUnsubscribe);

        if (active) {
          setStatus('live');
        }
      } catch (caughtError) {
        if (!active) {
            return;
        }
        setStatus('error');
        setError(caughtError instanceof Error ? caughtError.message : String(caughtError));
      }
    };

    void start();

    return () => {
      active = false;
      for (const unsubscribe of unsubscribers) {
        void unsubscribe();
      }
      void client.disconnect();
    };
  }, []);

  useEffect(() => {
    const thread = threadRef.current;
    if (!thread) {
      return;
    }

    thread.scrollTop = thread.scrollHeight;
  }, [liveDraft?.preview, messages]);

  const send = async (event: React.FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    const content = draft.trim();
    if (!content) {
      return;
    }

    try {
      setIsSubmitting(true);
      setError(null);
      await db.insert(chatMessages).values({
        room: ROOM,
        role: 'user',
        author: CHAT_USERNAME,
        sender_username: CHAT_USERNAME,
        content,
      });
      setDraft('');
    } catch (caughtError) {
      setError(caughtError instanceof Error ? caughtError.message : String(caughtError));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <main className="chat-shell">
      <section className="chat-hero">
        <div>
          <p className="eyebrow">USER table + STREAM table + EXECUTE AS USER</p>
          <h1>Chat With AI</h1>
          <p>
            The browser writes to a USER table, subscribes to a STREAM table, and watches the agent draft replies in real time before the final assistant row is committed.
          </p>
        </div>
        <div className={`status status-${status}`} data-testid="chat-status">
          <span className="status-dot" />
          {status === 'live' ? 'Live' : status === 'connecting' ? 'Connecting' : 'Stopped'}
        </div>
      </section>

      <section className="chat-panel">
        <div className="chat-layout">
          <div className="chat-main">
            <div className="chat-thread" data-testid="chat-thread" ref={threadRef}>
              {messages.map((message) => (
                <article className={`bubble bubble-${message.role}`} key={message.id}>
                  <header>
                    <strong>{message.author}</strong>
                    <span>{formatCreatedAt(message.created_at)}</span>
                  </header>
                  <p>{message.content}</p>
                </article>
              ))}
              {liveDraft ? (
                <article className="bubble bubble-assistant bubble-live" data-testid="stream-preview">
                  <header>
                    <strong>KalamDB Copilot</strong>
                    <span>{liveDraft.label}</span>
                  </header>
                  <div className="bubble-live-status" data-testid="write-status">
                    <span className="typing-indicator" aria-hidden="true">
                      <span />
                      <span />
                      <span />
                    </span>
                    <span>
                      {liveDraft.stage === 'thinking'
                        ? 'Thinking through the reply'
                        : liveDraft.stage === 'saving'
                          ? 'Saving the final assistant message'
                          : 'Writing the reply'}
                    </span>
                  </div>
                  <p>{liveDraft.preview}</p>
                </article>
              ) : null}
            </div>

            <form className="composer" onSubmit={send}>
              <label>
                Message
                <textarea
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  placeholder="Ask about latency, deploys, queues, or anything else you want the worker to stream back"
                />
              </label>
              <div className="composer-actions">
                <button type="submit" disabled={isSubmitting}>
                  {isSubmitting ? 'Sending…' : 'Send through KalamDB'}
                </button>
                {liveDraft ? (
                  <span className="composer-writing-hint">
                    Live reply in progress
                  </span>
                ) : null}
              </div>
            </form>
            {error ? <p className="error-text">{error}</p> : null}
          </div>

          <aside className="event-rail">
            <header className="event-rail-header">
              <strong>Live agent events</strong>
              <span>{events.length} buffered</span>
            </header>
            <ul className="event-list" data-testid="agent-events">
              {events.slice(-8).reverse().map((event) => (
                <li className="event-item" key={`${event.id}-${event.response_id}`}>
                  <div>
                    <strong>{event.stage}</strong>
                    <p>{event.message}</p>
                  </div>
                  <span>{formatCreatedAt(event.created_at)}</span>
                </li>
              ))}
            </ul>
          </aside>
        </div>
      </section>
    </main>
  );
}