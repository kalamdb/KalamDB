import { HelpCircle, MessageSquarePlus, Settings } from 'lucide-react';
import type { Conversations as ConversationRow } from '../schema.generated';

const dateFormatter = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });

export function Aside({
  conversations,
  selectedConversationId,
  onCreate,
  onSelect,
}: {
  conversations: ConversationRow[];
  selectedConversationId: string;
  onCreate: () => void;
  onSelect: (conversationId: string) => void;
}) {
  return (
    <aside className="app-aside" aria-label="Conversations">
      <div className="brand-block">
        <div className="brand-mark">AI</div>
        <div>
          <strong>AI Workspace</strong>
          <span>Enterprise Pro</span>
        </div>
      </div>

      <button type="button" className="new-chat-button" onClick={onCreate}>
        <MessageSquarePlus size={18} />
        New Chat
      </button>

      <div className="aside-section">
        <div className="aside-label">Conversations</div>
        <div className="conversation-nav">
          {conversations.map((conversation) => (
            <button
              type="button"
              key={conversation.id}
              className={conversation.id === selectedConversationId ? 'conversation-nav-item active' : 'conversation-nav-item'}
              onClick={() => onSelect(conversation.id)}
            >
              <span className="conversation-icon" aria-hidden="true" />
              <span className="conversation-text">
                <strong>{conversation.title}</strong>
                <small>{conversation.summary}</small>
              </span>
              <time>{dateFormatter.format(conversation.updated_at)}</time>
            </button>
          ))}
          {conversations.length === 0 ? <p className="aside-empty">No conversations yet</p> : null}
        </div>
      </div>

      <div className="aside-footer">
        <button type="button"><Settings size={16} /> Settings</button>
        <button type="button"><HelpCircle size={16} /> Help</button>
      </div>
    </aside>
  );
}
