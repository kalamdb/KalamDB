import { useLiveQueries, useLiveSelection } from '@kalamdb/react';
import { desc, eq } from 'drizzle-orm';
import { approvals, messages, presence, toolCalls, toolResults, typing } from './schema.js';

export function AssistantWorkspace({ threadId }: { threadId: string }) {
  const live = useLiveQueries({
    queries: {
      messages: { table: messages, where: (table) => eq(table.threadId, threadId), deps: [threadId] },
      toolCalls: {
        table: toolCalls,
        where: (table) => eq(table.threadId, threadId),
        orderBy: (table) => desc(table.createdAt),
        deps: [threadId],
      },
      toolResults: {
        table: toolResults,
        where: (table) => eq(table.threadId, threadId),
        orderBy: (table) => desc(table.createdAt),
        deps: [threadId],
      },
      typing: { table: typing, where: (table) => eq(table.threadId, threadId), deps: [threadId] },
      presence: { table: presence, where: (table) => eq(table.threadId, threadId), deps: [threadId] },
      approvals: { table: approvals, where: (table) => eq(table.threadId, threadId), deps: [threadId] },
    },
    deps: [threadId],
  });
  const assistant = useLiveSelection(live, (context) => ({
    messages: context.messages.rows,
    activeToolCalls: context.toolCalls.rows.filter((row) => row.status !== 'completed'),
    latestToolResults: context.toolResults.rows,
    typingUsers: context.typing.rows.map((row) => row.userName),
    onlineUsers: context.presence.rows.filter((row) => row.status === 'online'),
    pendingApprovals: context.approvals.rows.filter((row) => row.status === 'pending'),
    approve: (approvalId: string) => context.update(approvals, approvalId).set({ status: 'approved' }),
    reject: (approvalId: string) => context.update(approvals, approvalId).set({ status: 'rejected' }),
  }));

  return (
    <section>
      {assistant.messages.map((message) => <article key={message.id}>{message.body}</article>)}
      {assistant.typingUsers.length > 0 ? <p>{assistant.typingUsers.join(', ')} typing</p> : null}
      {assistant.activeToolCalls.map((call) => <p key={call.id}>{call.status}</p>)}
      {assistant.pendingApprovals.map((approval) => (
        <button key={approval.id} onClick={() => assistant.approve(String(approval.id))}>Approve</button>
      ))}
    </section>
  );
}