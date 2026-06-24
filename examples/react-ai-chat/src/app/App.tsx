import { useEffect, useMemo, useState } from 'react';
import { KalamProvider, LiveQueries, type MultiLiveQueryContext } from '@kalamdb/react';
import { asc, desc, eq } from 'drizzle-orm';
import { getExampleClient, isExampleDemoMode } from './client';
import { Aside } from './components/Aside';
import { Conversation } from './components/Conversation';
import { approval_actions as approvalActions, conversations, messages, typing_tokens as typingTokens } from './schema.generated';
import type { Conversations as ConversationRow } from './schema.generated';

const SELECTED_CONVERSATION_KEY = 'kalamdb-react-ai-chat-selected-v3';
const DEFAULT_CONVERSATION_ID = 'project-alpha';

type ChatQueries = {
  conversations: { table: typeof conversations };
  messages: { table: typeof messages };
  typingTokens: { table: typeof typingTokens };
};

export type ChatLiveContext = MultiLiveQueryContext<ChatQueries>;

export function App() {
  const client = useMemo(() => getExampleClient(), []);
  const [selectedConversationId, setSelectedConversationId] = useState(loadSelectedConversationId);

  useEffect(() => {
    window.localStorage.setItem(SELECTED_CONVERSATION_KEY, selectedConversationId);
  }, [selectedConversationId]);

  const queries = useMemo(() => ({
    conversations: {
      table: conversations,
      orderBy: (table: typeof conversations) => desc(table.updated_at),
      limit: 50,
    },
    messages: {
      table: messages,
      where: (table: typeof messages) => eq(table.conversation_id, selectedConversationId),
      orderBy: (table: typeof messages) => asc(table.created_at),
      deps: [selectedConversationId],
    },
    typingTokens: {
      table: typingTokens,
      where: (table: typeof typingTokens) => eq(table.conversation_id, selectedConversationId),
      orderBy: (table: typeof typingTokens) => asc(table.created_at),
      deps: [selectedConversationId],
    },
  }), [selectedConversationId]);

  return (
    <KalamProvider client={client}>
      <LiveQueries queries={queries} deps={[selectedConversationId]}>
        {(live) => (
          <ChatWorkspace
            live={live as ChatLiveContext}
            selectedConversationId={selectedConversationId}
            onSelectConversation={setSelectedConversationId}
          />
        )}
      </LiveQueries>
    </KalamProvider>
  );
}

function ChatWorkspace({
  live,
  selectedConversationId,
  onSelectConversation,
}: {
  live: ChatLiveContext;
  selectedConversationId: string;
  onSelectConversation: (conversationId: string) => void;
}) {
  const currentConversation = useMemo(
    () => resolveConversation(live.conversations.rows, selectedConversationId),
    [live.conversations.rows, selectedConversationId],
  );

  useEffect(() => {
    if (!currentConversation && live.conversations.rows[0]) {
      onSelectConversation(live.conversations.rows[0].id);
    }
  }, [currentConversation, live.conversations.rows, onSelectConversation]);

  const createConversation = async () => {
    const id = createId('conversation');
    await live.insert(conversations).values({
      id,
      title: 'New chat',
      summary: 'Fresh conversation',
      created_at: new Date(),
      updated_at: new Date(),
    });
    onSelectConversation(id);
  };

  return (
    <main className="workspace-shell">
      <Aside
        conversations={live.conversations.rows}
        selectedConversationId={currentConversation?.id ?? selectedConversationId}
        onCreate={() => void createConversation()}
        onSelect={onSelectConversation}
      />
      <Conversation
        client={getExampleClient()}
        demoMode={isExampleDemoMode()}
        live={live}
        selectedConversationId={selectedConversationId}
        conversation={currentConversation}
        conversationsTable={conversations}
        messagesTable={messages}
        approvalActionsTable={approvalActions}
        onSelectConversation={onSelectConversation}
      />
    </main>
  );
}

function resolveConversation(rows: ConversationRow[], selectedConversationId: string): ConversationRow | null {
  return rows.find((conversation) => conversation.id === selectedConversationId) ?? rows[0] ?? null;
}

function createId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`}`;
}

function loadSelectedConversationId(): string {
  return window.localStorage.getItem(SELECTED_CONVERSATION_KEY) ?? DEFAULT_CONVERSATION_ID;
}
