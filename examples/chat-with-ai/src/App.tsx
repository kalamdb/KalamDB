import { useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { type SubscriptionErrorEvent } from '@kalamdb/client';
import { and, eq } from 'drizzle-orm';
import { liveTable } from '@kalamdb/orm';
import {
  chatDemoAgentEvents as agentEvents,
  chatDemoDirectMessages as directMessages,
  chatDemoMessages as chatMessages,
  type ChatDemoAgentEvents as AgentEventRow,
  type ChatDemoDirectMessages as DirectMessageRow,
  type ChatDemoMessages as ChatMessageRow,
  type ChatDemoMessageTarget,
} from './generated/kalam';
import { api, CHAT_USERNAME, client, ROOM } from './db';
import './styles.css';

type Inbox = ChatDemoMessageTarget;
type ThreadRow = ChatMessageRow | DirectMessageRow;

type LiveDraft = {
  stage: 'thinking' | 'typing' | 'saving';
  label: string;
  preview: string;
};

const MAX_CHAT_MESSAGES = 80;
const MAX_AGENT_EVENTS = 40;
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});

function asDate(value: Date | string): Date {
  return value instanceof Date ? value : new Date(value);
}

function formatCreatedAt(createdAt: Date | string): string {
  const date = asDate(createdAt);
  return Number.isNaN(date.getTime()) ? 'Invalid date' : timeFormatter.format(date);
}

function eventTimelineKey(row: { id: string | bigint | number; created_at: Date | string }): string {
  return `${asDate(row.created_at).toISOString()}:${String(row.id)}`;
}

function sortEvents(rows: AgentEventRow[]): AgentEventRow[] {
  return [...rows].sort(
    (left, right) => asDate(left.created_at).getTime() - asDate(right.created_at).getTime()
      || eventTimelineKey(left).localeCompare(eventTimelineKey(right))
      || String(left.id).localeCompare(String(right.id)),
  );
}

function limitEvents(rows: AgentEventRow[]): AgentEventRow[] {
  const sorted = sortEvents(rows);
  return sorted.length > MAX_AGENT_EVENTS ? sorted.slice(-MAX_AGENT_EVENTS) : sorted;
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

function deriveFallbackDraft(messages: ThreadRow[]): LiveDraft | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === 'assistant') {
      return null;
    }

    if (message.role === 'user') {
      return {
        stage: 'thinking',
        label: 'KalamDB Copilot is preparing the reply',
        preview: 'AI reply: waiting for the topic trigger…',
      };
    }
  }

  return null;
}

export function App() {
  const [inbox, setInbox] = useState<Inbox>('room');
  const [messages, setMessages] = useState<ThreadRow[]>([]);
  const [events, setEvents] = useState<AgentEventRow[]>([]);
  const [draft, setDraft] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [status, setStatus] = useState<'connecting' | 'live' | 'error'>('connecting');
  const [error, setError] = useState<string | null>(null);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const liveDraft = deriveLiveDraft(events) ?? deriveFallbackDraft(messages);
  const destination = inbox === 'room' ? ROOM : CHAT_USERNAME;

  useEffect(() => {
    let active = true;
    const unsubscribers: Array<() => Promise<void>> = [];

    const start = async (): Promise<void> => {
      try {
        if (inbox === 'room') {
          await api.chatDemo.joinRoom({ room_id: ROOM });
        }

        const messagesUnsubscribe = inbox === 'room'
          ? await liveTable(
              client,
              chatMessages,
              (nextMessages) => {
                if (active) {
                  setMessages(nextMessages);
                }
              },
              {
                where: eq(chatMessages.room, ROOM),
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
            )
          : await liveTable(
              client,
              directMessages,
              (nextMessages) => {
                if (active) {
                  setMessages(nextMessages);
                }
              },
              {
                limit: MAX_CHAT_MESSAGES,
                lastRows: MAX_CHAT_MESSAGES,
                onError: (event: SubscriptionErrorEvent) => {
                  if (!active) {
                    return;
                  }
                  setStatus('error');
                  setError(`Inbox subscription failed (${event.code}): ${event.message}`);
                },
              },
            );
        unsubscribers.push(messagesUnsubscribe);

        const eventsUnsubscribe = await liveTable(
          client,
          agentEvents,
          (nextEvents) => {
            if (!active) {
              return;
            }

            flushSync(() => {
              setEvents(limitEvents(nextEvents));
            });
          },
          {
            where: and(eq(agentEvents.scope, inbox), eq(agentEvents.room, destination)),
            lastRows: MAX_AGENT_EVENTS,
            limit: MAX_AGENT_EVENTS,
            onError: (event: SubscriptionErrorEvent) => {
              if (!active) {
                return;
              }
              setStatus('error');
              setError(`Event subscription failed (${event.code}): ${event.message}`);
            },
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

    setStatus('connecting');
    setMessages([]);
    setEvents([]);
    void start();

    return () => {
      active = false;
      for (const unsubscribe of unsubscribers) {
        void unsubscribe();
      }
      void client.disconnect();
    };
  }, [destination, inbox]);

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
      await api.chatDemo.sendMessage({
        target: inbox,
        target_id: destination,
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
          <p className="eyebrow">SHARED rooms + USER inbox + STREAM events</p>
          <h1>Chat With AI</h1>
          <p>
            Room chat is a SHARED transcript with RLS. Personal chat writes to your USER table.
            Both go through <code>send_message</code>. A topic trigger inside KalamDB writes STREAM
            progress to your session, then commits the reply.
          </p>
        </div>
        <div className={`status status-${status}`} data-testid="chat-status">
          <span className="status-dot" />
          {status === 'live' ? 'Live' : status === 'connecting' ? 'Connecting' : 'Stopped'}
        </div>
      </section>

      <section className="chat-panel">
        <div className="inbox-toggle" role="tablist" aria-label="Chat destination">
          <button
            type="button"
            role="tab"
            aria-selected={inbox === 'room'}
            className={inbox === 'room' ? 'is-active' : undefined}
            data-testid="chat-scope-room"
            onClick={() => setInbox('room')}
          >
            Room
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={inbox === 'direct'}
            className={inbox === 'direct' ? 'is-active' : undefined}
            data-testid="chat-scope-direct"
            onClick={() => setInbox('direct')}
          >
            Personal
          </button>
        </div>
        <div className="chat-layout">
          <div className="chat-main">
            <div className="chat-thread" data-testid="chat-thread" ref={threadRef}>
              {messages.map((message) => (
                <article className={`bubble bubble-${message.role}`} key={String(message.id)}>
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
                  placeholder="Ask about latency, deploys, queues, or anything else you want the procedure to stream back"
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
              <strong>Live procedure events</strong>
              <span>{events.length} buffered</span>
            </header>
            <ul className="event-list" data-testid="agent-events">
              {events.slice(-8).reverse().map((event) => (
                <li className="event-item" key={`${String(event.id)}-${event.response_id}`}>
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
