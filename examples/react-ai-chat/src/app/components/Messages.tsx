import { useEffect, useMemo, useRef } from 'react';
import { Bot, CheckCircle2, Clock3, Database, Download, LoaderCircle, ShieldCheck, User, X } from 'lucide-react';
import type { ApprovalCache, MessageView } from './Conversation';

const timeFormatter = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' });

export function Messages({
  messages,
  approvals,
  onApprovalAction,
}: {
  messages: MessageView[];
  approvals: ApprovalCache;
  onApprovalAction: (approvalId: string, action: 'approved' | 'declined') => Promise<void>;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stayAtBottomRef = useRef(true);
  const scrollKey = useMemo(() => messages.map((message) => [message.id, message.status, message.body, message.approvalId].join(':')).join('|'), [messages]);

  useEffect(() => {
    if (!stayAtBottomRef.current) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      const node = scrollRef.current;
      if (node) {
        node.scrollTop = node.scrollHeight;
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [scrollKey]);

  return (
    <div
      ref={scrollRef}
      className="messages-scroll"
      onScroll={() => {
        const node = scrollRef.current;
        if (!node) {
          return;
        }
        stayAtBottomRef.current = node.scrollHeight - node.scrollTop - node.clientHeight < 72;
      }}
    >
      <div className="messages-column">
        {messages.map((message) => (
          <MessageBubble
            key={message.id}
            message={message}
            approval={message.approvalId ? approvals[message.approvalId] : undefined}
            onApprovalAction={onApprovalAction}
          />
        ))}
        {messages.length === 0 ? (
          <div className="empty-conversation">
            <Bot size={22} />
            <strong>Nexus AI</strong>
            <span>Ready when you are.</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function MessageBubble({
  message,
  approval,
  onApprovalAction,
}: {
  message: MessageView;
  approval?: ApprovalCache[string];
  onApprovalAction: (approvalId: string, action: 'approved' | 'declined') => Promise<void>;
}) {
  const isUser = message.role === 'user';
  const isProjectAlpha = message.approvalId === 'approval-project-alpha' || message.body.toLowerCase().includes('summary chart');

  return (
    <article className={isUser ? 'message-row user' : 'message-row assistant'}>
      <div className="avatar" aria-hidden="true">{isUser ? <User size={16} /> : <Bot size={16} />}</div>
      <div className="message-content">
        <div className="message-meta">
          <span>{isUser ? 'You' : 'Nexus AI'}</span>
          <time>{timeFormatter.format(message.createdAt)}</time>
        </div>
        <div className="message-bubble">
          <p>{message.body || statusCopy(message.status)}</p>
          {isProjectAlpha ? <AnalysisPreview /> : null}
          {message.attachmentName ? (
            <span className="attachment-chip">
              <Database size={14} />
              {message.attachmentName}
            </span>
          ) : null}
          {message.approvalId ? (
            <ApprovalCard
              approvalId={message.approvalId}
              entry={approval}
              onAction={onApprovalAction}
            />
          ) : null}
        </div>
        <MessageStatus message={message} />
      </div>
    </article>
  );
}

function ApprovalCard({
  approvalId,
  entry,
  onAction,
}: {
  approvalId: string;
  entry?: ApprovalCache[string];
  onAction: (approvalId: string, action: 'approved' | 'declined') => Promise<void>;
}) {
  const approval = entry?.approval;
  const loading = entry?.loading ?? true;
  const resolved = approval?.status === 'approved' || approval?.status === 'declined';

  return (
    <section className={approval?.status === 'approved' ? 'approval-card approved' : 'approval-card'}>
      <div className="approval-icon"><ShieldCheck size={18} /></div>
      <div className="approval-main">
        <strong>{approval?.title ?? 'Action Required'}</strong>
        <span>{loading ? 'Loading approval...' : approval?.body ?? entry?.error ?? 'Approval was not found.'}</span>
        {approval ? <small>Status: {approval.status}</small> : null}
      </div>
      {loading ? <LoaderCircle className="spin" size={18} /> : null}
      {approval && !resolved ? (
        <div className="approval-actions">
          <button type="button" className="approve-button" onClick={() => void onAction(approvalId, 'approved')}>Approve</button>
          <button type="button" className="decline-button" onClick={() => void onAction(approvalId, 'declined')}>Decline</button>
        </div>
      ) : null}
    </section>
  );
}

function AnalysisPreview() {
  const bars = [58, 34, 49, 29, 62, 76, 26, 31, 36, 43, 55, 69, 82, 61, 47, 72];
  return (
    <figure className="analysis-preview" aria-label="Q3 performance analysis chart">
      <div className="chart-window">
        <div className="chart-toolbar"><span /> <span /> <span /></div>
        <div className="chart-grid">
          {bars.map((height, index) => <span key={index} style={{ height: `${height}%` }} />)}
        </div>
      </div>
      <figcaption>
        <span>Q3 Performance Analysis</span>
        <Download size={13} />
      </figcaption>
    </figure>
  );
}

function MessageStatus({ message }: { message: MessageView }) {
  if (message.status === 'failed') {
    return <span className="send-state failed"><X size={13} /> {message.error ?? 'Not sent'}</span>;
  }
  if (message.pending || message.status === 'sending') {
    return <span className="send-state"><Clock3 size={13} /> Sending</span>;
  }
  if (message.role === 'user') {
    return <span className="send-state sent"><CheckCircle2 size={13} /> Sent</span>;
  }
  if (message.status === 'thinking' || message.status === 'typing') {
    return <span className="send-state"><LoaderCircle className="spin" size={13} /> {statusCopy(message.status)}</span>;
  }
  return null;
}

function statusCopy(status: string): string {
  switch (status) {
    case 'thinking':
      return 'Thinking...';
    case 'typing':
      return 'Typing...';
    case 'saving':
      return 'Saving...';
    default:
      return status.replace(/_/g, ' ');
  }
}