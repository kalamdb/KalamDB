import { useLiveQueries, useLiveSelection, KalamProvider } from '@kalamdb/react';
import { kTable } from '@kalamdb/orm';
import { bigint, text, timestamp } from 'drizzle-orm/pg-core';
import { desc, eq } from 'drizzle-orm';
import { Bot, Check, Hammer, MessageSquareText, Users } from 'lucide-react';
import { getClient } from '@/lib/kalam-client';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';

const messages = kTable.user('assistant_demo.messages', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  role: text('role').notNull(),
  body: text('body').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).notNull(),
});

const toolCalls = kTable.user('assistant_demo.tool_calls', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  name: text('name').notNull(),
  status: text('status').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).notNull(),
});

const typing = kTable.user('assistant_demo.typing', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  userName: text('user_name').notNull(),
});

const approvals = kTable.user('assistant_demo.approvals', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  summary: text('summary').notNull(),
  status: text('status').notNull(),
});

const THREAD_ID = 'demo-thread';

export function AssistantWorkflowDemo() {
  const client = getClient();

  if (!client) {
    return null;
  }

  return (
    <KalamProvider client={client}>
      <AssistantWorkflowContent />
    </KalamProvider>
  );
}

function AssistantWorkflowContent() {
  const live = useLiveQueries({
    queries: {
      messages: {
        table: messages,
        where: (table) => eq(table.threadId, THREAD_ID),
        orderBy: (table) => desc(table.createdAt),
      },
      toolCalls: {
        table: toolCalls,
        where: (table) => eq(table.threadId, THREAD_ID),
        orderBy: (table) => desc(table.createdAt),
      },
      typing: { table: typing, where: (table) => eq(table.threadId, THREAD_ID) },
      approvals: { table: approvals, where: (table) => eq(table.threadId, THREAD_ID) },
    },
  });
  const assistant = useLiveSelection(live, (context) => ({
    messages: context.messages.rows,
    activeTools: context.toolCalls.rows.filter((row) => row.status !== 'completed'),
    typingUsers: context.typing.rows.map((row) => row.userName),
    pendingApprovals: context.approvals.rows.filter((row) => row.status === 'pending'),
    approve: (id: string) => context.update(approvals, id).set({ status: 'approved' }),
  }));

  return (
    <Card className="p-4">
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Bot className="h-5 w-5 text-violet-600" />
          <h2 className="text-base font-semibold">Assistant workflow composition</h2>
        </div>
        <Badge variant={live.state.connected ? 'default' : 'outline'}>{live.state.loading ? 'Opening' : 'Live'}</Badge>
      </div>
      <div className="grid gap-3 md:grid-cols-4">
        <Summary icon={<MessageSquareText className="h-4 w-4" />} label="Messages" value={assistant.messages.length} />
        <Summary icon={<Hammer className="h-4 w-4" />} label="Active tools" value={assistant.activeTools.length} />
        <Summary icon={<Users className="h-4 w-4" />} label="Typing" value={assistant.typingUsers.length} />
        <Summary icon={<Check className="h-4 w-4" />} label="Approvals" value={assistant.pendingApprovals.length} />
      </div>
      <div className="mt-4 space-y-2">
        {assistant.pendingApprovals.map((approval) => (
          <div key={approval.id} className="flex items-center justify-between rounded border p-2 text-sm">
            <span>{approval.summary}</span>
            <Button type="button" size="sm" onClick={() => void assistant.approve(String(approval.id))}>Approve</Button>
          </div>
        ))}
      </div>
    </Card>
  );
}

function Summary({ icon, label, value }: { icon: React.ReactNode; label: string; value: number }) {
  return (
    <div className="rounded border p-3">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">{icon}{label}</div>
      <div className="text-2xl font-semibold">{value}</div>
    </div>
  );
}