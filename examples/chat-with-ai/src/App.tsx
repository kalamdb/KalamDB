import { useEffect, useRef, useState, type FormEvent } from 'react';
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

const LAST_MESSAGES = 80;
const LAST_EVENTS = 40;
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

function liveDraftFromEvents(events: AgentEventRow[]): LiveDraft | null {
  let active: AgentEventRow | null = null;

  for (const event of events) {
    if (event.stage === 'thinking' || event.stage === 'typing' || event.stage === 'message_saved') {
      active = event;
    }
    if (event.stage === 'complete' && active?.response_id === event.response_id) {
      active = null;
    }
  }

  if (!active) {
    return null;
  }

  if (active.stage === 'thinking') {
    return {
      stage: 'thinking',
      label: 'KalamDB Copilot is thinking',
      preview: 'Planning the reply and preparing the first streamed tokens…',
    };
  }

  if (active.stage === 'message_saved') {
    return {
      stage: 'saving',
      label: 'KalamDB Copilot is committing the reply',
      preview: active.preview,
    };
  }

  return {
    stage: 'typing',
    label: 'KalamDB Copilot is streaming characters',
    preview: active.preview,
  };
}

function waitingDraft(messages: ThreadRow[]): LiveDraft | null {
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
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<'connecting' | 'live' | 'error'>('connecting');
  const [error, setError] = useState<string | null>(null);
  const threadRef = useRef<HTMLDivElement>(null);
  const liveDraft = liveDraftFromEvents(events) ?? waitingDraft(messages);
  const destination = inbox === 'room' ? ROOM : CHAT_USERNAME;

  useEffect(() => {
    let active = true;
    const unsubscribers: Array<() => Promise<void>> = [];

    const onError = (label: string) => (event: SubscriptionErrorEvent) => {
      if (!active) {
        return;
      }
      setStatus('error');
      setError(`${label} (${event.code}): ${event.message}`);
    };

    const start = async (): Promise<void> => {
      try {
        if (inbox === 'room') {
          await api.chatDemo.joinRoom({ room_id: ROOM });
        }
        if (!active) {
          return;
        }

        const messagesUnsubscribe = inbox === 'room'
          ? await liveTable(
              client,
              chatMessages,
              (rows) => {
                if (active) {
                  setMessages(rows);
                }
              },
              {
                where: eq(chatMessages.room, ROOM),
                lastRows: LAST_MESSAGES,
                limit: LAST_MESSAGES,
                onError: onError('Message subscription failed'),
              },
            )
          : await liveTable(
              client,
              directMessages,
              (rows) => {
                if (active) {
                  setMessages(rows);
                }
              },
              {
                lastRows: LAST_MESSAGES,
                limit: LAST_MESSAGES,
                onError: onError('Inbox subscription failed'),
              },
            );
        if (!active) {
          await messagesUnsubscribe();
          return;
        }
        unsubscribers.push(messagesUnsubscribe);

        const eventsUnsubscribe = await liveTable(
          client,
          agentEvents,
          (rows) => {
            if (active) {
              setEvents(rows);
            }
          },
          {
            where: and(eq(agentEvents.scope, inbox), eq(agentEvents.room, destination)),
            lastRows: LAST_EVENTS,
            limit: LAST_EVENTS,
            onError: onError('Event subscription failed'),
          },
        );
        if (!active) {
          await eventsUnsubscribe();
          return;
        }
        unsubscribers.push(eventsUnsubscribe);
        setStatus('live');
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
    };
  }, [destination, inbox]);

  useEffect(() => {
    const thread = threadRef.current;
    if (thread) {
      thread.scrollTop = thread.scrollHeight;
    }
  }, [liveDraft?.preview, messages]);

  const send = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    const content = draft.trim();
    if (!content) {
      return;
    }

    try {
      setBusy(true);
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
      setBusy(false);
    }
  };

  return (
    <main className="chat-shell">
      <section className="chat-hero">
        <div>
          <p className="eyebrow">SHARED rooms + USER inbox + STREAM events</p>
          <h1>Chat With AI</h1>
          <p>
            Join a room, send a message, and a topic trigger inside KalamDB streams the reply.
            No extra agent process.
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
                  placeholder="Ask about latency, deploys, queues, or anything else"
                />
              </label>
              <div className="composer-actions">
                <button type="submit" disabled={busy}>
                  {busy ? 'Sending…' : 'Send through KalamDB'}
                </button>
                {liveDraft ? <span className="composer-writing-hint">Live reply in progress</span> : null}
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
