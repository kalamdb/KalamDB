import { useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { type SubscriptionErrorEvent } from '@kalamdb/client';
import { eq } from 'drizzle-orm';
import { liveTable } from '@kalamdb/orm';
import {
  chat_demo_agent_events as agentEvents,
  chat_demo_agent_eventsConfig as agentEventsConfig,
  chat_demo_messages as chatMessages,
  chat_demo_room_members as roomMembers,
  chat_demo_rooms as rooms,
  type ChatDemoAgentEvents as AgentEventRow,
  type ChatDemoMessages as ChatMessageRow,
} from './generated/kalam';
import { CHAT_USERNAME, client, db, membershipId, ROOM } from './db';
import './styles.css';

type LiveDraft = {
  stage: 'thinking' | 'typing' | 'saving';
  label: string;
  preview: string;
};

const MAX_CHAT_MESSAGES = 80;
const MAX_AGENT_EVENTS = agentEventsConfig.tableType === 'stream' ? 40 : 20;
const canSortEventsBySeq = agentEventsConfig.systemColumns.includes('_seq');
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});

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

async function ensureRoomAccess(): Promise<void> {
  const memberId = membershipId(CHAT_USERNAME, ROOM);
  const existingMembership = await db
    .select({ id: roomMembers.id })
    .from(roomMembers)
    .where(eq(roomMembers.id, memberId))
    .limit(1);

  // Schema seeds root/admin into `main`. Other users still join here.
  if (existingMembership.length === 0) {
    await db.insert(roomMembers).values({
      id: memberId,
      user_id: CHAT_USERNAME,
      room_id: ROOM,
    });
  }

  const existingRoom = await db
    .select({ id: rooms.id })
    .from(rooms)
    .where(eq(rooms.id, ROOM))
    .limit(1);

  if (existingRoom.length === 0) {
    await db.insert(rooms).values({ id: ROOM, title: ROOM });
  }
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
    const unsubscribers: Array<() => Promise<void>> = [];

    const start = async (): Promise<void> => {
      try {
        // Membership is required before SELECT policies let this tab see the room.
        await ensureRoomAccess();

        // Durable transcript for this room.
        const messagesUnsubscribe = await liveTable(
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
        );
        unsubscribers.push(messagesUnsubscribe);

        // STREAM rows for thinking / typing. TTL on the table keeps this short-lived.
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
            where: eq(agentEvents.room, ROOM),
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
      // One insert. The topic + agent turn this into a streamed assistant reply.
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
          <p className="eyebrow">SHARED rooms + RLS + STREAM events</p>
          <h1>Chat With AI</h1>
          <p>
            Members of the same room share one message stream. Policies keep other rooms private, a STREAM table shows the agent thinking, and a topic worker writes the assistant reply.
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
