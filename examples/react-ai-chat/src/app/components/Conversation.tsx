import { useCallback, useEffect, useMemo, useState } from 'react';
import type { KalamDBClient } from '@kalamdb/client';
import { Database, MoreVertical, Share2 } from 'lucide-react';
import type { ChatLiveContext } from '../App';
import type { approvalActions, conversations, messages } from '../schema.generated';
import type { ApprovalRow, ConversationRow, MessageRow, TypingTokenRow } from '../schema.generated';
import { ChatComposer } from './ChatComposer';
import { Messages } from './Messages';

type ConversationsTable = typeof conversations;
type MessagesTable = typeof messages;
type ApprovalActionsTable = typeof approvalActions;

export type PendingMessage = {
  id: string;
  clientId: string;
  conversationId: string;
  body: string;
  status: 'sending' | 'failed';
  attachmentName: string | null;
  createdAt: Date;
  error?: string;
};

export type MessageView = {
  id: string;
  clientId: string | null;
  conversationId: string;
  role: string;
  body: string;
  status: string;
  attachmentName: string | null;
  approvalId: string | null;
  createdAt: Date;
  updatedAt: Date;
  pending: boolean;
  error?: string;
};

export type ApprovalCache = Record<string, {
  loading: boolean;
  approval: ApprovalRow | null;
  error: string | null;
}>;

export function Conversation({
  client,
  demoMode,
  live,
  conversation,
  selectedConversationId,
  conversationsTable,
  messagesTable,
  approvalActionsTable,
  onSelectConversation,
}: {
  client: KalamDBClient;
  demoMode: boolean;
  live: ChatLiveContext;
  conversation: ConversationRow | null;
  selectedConversationId: string;
  conversationsTable: ConversationsTable;
  messagesTable: MessagesTable;
  approvalActionsTable: ApprovalActionsTable;
  onSelectConversation: (conversationId: string) => void;
}) {
  const [pendingMessages, setPendingMessages] = useState<PendingMessage[]>([]);
  const { approvals, markApprovalStatus } = useApprovalCache(client, live.messages.rows);
  const conversationId = conversation?.id ?? selectedConversationId;

  const confirmedClientIds = useMemo(() => new Set(
    live.messages.rows.map((message) => message.clientId).filter((clientId): clientId is string => Boolean(clientId)),
  ), [live.messages.rows]);
  const confirmedClientKey = useMemo(() => [...confirmedClientIds].sort().join('|'), [confirmedClientIds]);

  useEffect(() => {
    setPendingMessages((current) => current.filter((message) => !confirmedClientIds.has(message.clientId)));
  }, [confirmedClientIds, confirmedClientKey]);

  const messageViews = useMemo(() => buildMessageViews({
    messages: live.messages.rows,
    tokens: live.typingTokens.rows,
    pendingMessages,
    confirmedClientIds,
    conversationId,
  }), [confirmedClientIds, conversationId, live.messages.rows, live.typingTokens.rows, pendingMessages]);

  const ensureConversation = useCallback(async (firstMessage: string): Promise<string> => {
    if (conversation) {
      return conversation.id;
    }

    const id = createId('conversation');
    await live.insert(conversationsTable).values({
      id,
      title: titleFromMessage(firstMessage),
      summary: firstMessage.slice(0, 96),
      createdAt: new Date(),
      updatedAt: new Date(),
    });
    onSelectConversation(id);
    return id;
  }, [conversation, conversationsTable, live, onSelectConversation]);

  const sendMessage = useCallback(async (body: string, attachment: File | null) => {
    const targetConversationId = await ensureConversation(body);
    const clientId = createId('client-message');
    const pendingMessage: PendingMessage = {
      id: `pending-${clientId}`,
      clientId,
      conversationId: targetConversationId,
      body,
      status: 'sending',
      attachmentName: attachment?.name ?? null,
      createdAt: new Date(),
    };

    setPendingMessages((current) => [...current, pendingMessage]);

    try {
      if (attachment) {
        await client.queryWithFiles(
          'INSERT INTO react_ai_chat.messages (client_id, conversation_id, role, body, status, attachment) VALUES ($1, $2, $3, $4, $5, FILE("attachment"))',
          { attachment },
          [clientId, targetConversationId, 'user', body, 'sent'],
        );
      } else {
        await live.insert(messagesTable).values({
          clientId,
          conversationId: targetConversationId,
          replyToMessageId: null,
          role: 'user',
          body,
          status: 'sent',
          attachment: null,
          approvalId: null,
          createdAt: new Date(),
          updatedAt: new Date(),
        });
      }
    } catch (error) {
      setPendingMessages((current) => current.map((message) => message.clientId === clientId
        ? { ...message, status: 'failed', error: error instanceof Error ? error.message : String(error) }
        : message));
    }
  }, [client, ensureConversation, live, messagesTable]);

  const answerApproval = useCallback(async (approvalId: string, action: 'approved' | 'declined') => {
    markApprovalStatus(approvalId, action);
    await live.insert(approvalActionsTable).values({
      approvalId,
      conversationId,
      action,
      createdAt: new Date(),
    });
  }, [approvalActionsTable, conversationId, live, markApprovalStatus]);

  return (
    <section className="conversation-panel" aria-label="Conversation">
      <header className="conversation-header">
        <div className="conversation-title-group">
          <h1>{conversation?.title ?? 'Nexus AI'}</h1>
          <p>{conversation?.summary ?? 'Start a fresh conversation and the first message will create the thread.'}</p>
        </div>
        <div className="header-actions">
          <span className={demoMode ? 'model-pill demo' : 'model-pill'}>
            <Database size={14} />
            {demoMode ? 'Demo Mode' : live.state.loading ? 'Opening streams' : 'Live'}
          </span>
          <button type="button" className="icon-button" aria-label="Share conversation" title="Share conversation"><Share2 size={17} /></button>
          <button type="button" className="icon-button" aria-label="Conversation menu" title="Conversation menu"><MoreVertical size={17} /></button>
        </div>
      </header>

      <Messages
        messages={messageViews}
        approvals={approvals}
        onApprovalAction={answerApproval}
      />

      <div className="composer-dock">
        <ChatComposer disabled={live.state.inserting} onSend={sendMessage} />
        <p className="composer-footnote">AI can make mistakes. Verify important information.</p>
      </div>
    </section>
  );
}

function useApprovalCache(client: KalamDBClient, rows: MessageRow[]) {
  const [cache, setCache] = useState<ApprovalCache>({});
  const approvalIds = useMemo(() => Array.from(new Set(
    rows.map((row) => row.approvalId).filter((approvalId): approvalId is string => Boolean(approvalId)),
  )), [rows]);
  const approvalKey = approvalIds.sort().join('|');

  useEffect(() => {
    let cancelled = false;
    for (const approvalId of approvalIds) {
      setCache((current) => current[approvalId]
        ? current
        : { ...current, [approvalId]: { loading: true, approval: null, error: null } });

      void (async () => {
        try {
          const row = await client.queryOne('SELECT * FROM react_ai_chat.approvals WHERE id = $1', [approvalId]);
          if (cancelled) {
            return;
          }
          setCache((current) => ({
            ...current,
            [approvalId]: { loading: false, approval: row ? mapApproval(row) : null, error: null },
          }));
        } catch (error) {
          if (cancelled) {
            return;
          }
          setCache((current) => ({
            ...current,
            [approvalId]: { loading: false, approval: null, error: error instanceof Error ? error.message : String(error) },
          }));
        }
      })();
    }

    return () => {
      cancelled = true;
    };
  }, [approvalIds, approvalKey, client]);

  const markApprovalStatus = useCallback((approvalId: string, status: 'approved' | 'declined') => {
    setCache((current) => {
      const entry = current[approvalId];
      if (!entry?.approval) {
        return current;
      }
      return {
        ...current,
        [approvalId]: {
          ...entry,
          approval: { ...entry.approval, status, updatedAt: new Date() },
        },
      };
    });
  }, []);

  return { approvals: cache, markApprovalStatus };
}

function buildMessageViews({
  messages: liveRows,
  tokens,
  pendingMessages,
  confirmedClientIds,
  conversationId,
}: {
  messages: MessageRow[];
  tokens: TypingTokenRow[];
  pendingMessages: PendingMessage[];
  confirmedClientIds: Set<string>;
  conversationId: string;
}): MessageView[] {
  const finalDraftClientIds = new Set(liveRows.map((row) => row.clientId).filter(Boolean));
  const liveViews = liveRows.map((row): MessageView => ({
    id: row.id,
    clientId: row.clientId,
    conversationId: row.conversationId,
    role: row.role,
    body: row.body,
    status: row.status,
    attachmentName: row.attachment?.name ?? null,
    approvalId: row.approvalId,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
    pending: false,
  }));

  const draftViews = buildDraftMessages(tokens)
    .filter((draft) => !finalDraftClientIds.has(draft.clientId))
    .filter((draft) => draft.conversationId === conversationId);
  const pendingViews = pendingMessages
    .filter((message) => message.conversationId === conversationId && !confirmedClientIds.has(message.clientId))
    .map((message): MessageView => ({
      id: message.id,
      clientId: message.clientId,
      conversationId: message.conversationId,
      role: 'user',
      body: message.body,
      status: message.status,
      attachmentName: message.attachmentName,
      approvalId: null,
      createdAt: message.createdAt,
      updatedAt: message.createdAt,
      pending: true,
      error: message.error,
    }));

  return [...liveViews, ...draftViews, ...pendingViews].sort((left, right) => left.createdAt.getTime() - right.createdAt.getTime());
}

function buildDraftMessages(tokens: TypingTokenRow[]): MessageView[] {
  const grouped = new Map<string, TypingTokenRow[]>();
  for (const token of tokens) {
    grouped.set(token.messageId, [...(grouped.get(token.messageId) ?? []), token]);
  }

  return [...grouped.entries()].map(([messageId, tokenRows]) => {
    const ordered = tokenRows.sort((left, right) => left.createdAt.getTime() - right.createdAt.getTime());
    const latest = ordered[ordered.length - 1];
    return {
      id: messageId,
      clientId: messageId,
      conversationId: latest.conversationId,
      role: 'assistant',
      body: ordered.map((token) => token.token).join(''),
      status: latest.status,
      attachmentName: null,
      approvalId: null,
      createdAt: ordered[0].createdAt,
      updatedAt: latest.createdAt,
      pending: true,
    } satisfies MessageView;
  });
}

function mapApproval(row: Record<string, unknown>): ApprovalRow {
  return {
    id: cellString(row.id),
    conversationId: cellString(row.conversation_id),
    messageId: cellString(row.message_id),
    title: cellString(row.title),
    body: cellString(row.body),
    status: cellString(row.status),
    createdAt: cellDate(row.created_at),
    updatedAt: cellDate(row.updated_at),
  };
}

function cellString(value: unknown): string {
  if (value && typeof value === 'object' && 'asString' in value && typeof value.asString === 'function') {
    return value.asString() ?? '';
  }
  return value == null ? '' : String(value);
}

function cellDate(value: unknown): Date {
  if (value && typeof value === 'object' && 'asDate' in value && typeof value.asDate === 'function') {
    return value.asDate() ?? new Date();
  }
  return value instanceof Date ? value : new Date(String(value));
}

function titleFromMessage(message: string): string {
  const words = message.trim().split(/\s+/).slice(0, 5).join(' ');
  return words || 'New chat';
}

function createId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`}`;
}