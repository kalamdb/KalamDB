import React from "react";
import { LiveQueries } from "@kalamdb/react";
import { asc, desc, eq } from "drizzle-orm";
import { approvals, conversations, messages, tasks, typingTokens } from "@/schema";
import { Sidebar } from "./Sidebar";
import { Conversation } from "./Conversation";
import { Welcome } from "./Welcome";

// KalamDB's default_query_limit silently caps unbounded reads, so each
// useLive that could grow per-conversation gets an explicit ceiling.
const MESSAGE_LIMIT = 500;
const TYPING_LIMIT = 1000;
const APPROVAL_LIMIT = 100;
const TASK_LIMIT = 100;
const CONVERSATION_LIMIT = 100;

export function App() {
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  const createConversation = React.useCallback(
    async (
      insert: (table: typeof conversations) => {
        values: (row: Record<string, unknown>) => Promise<unknown>;
      },
    ) => {
      const id = crypto.randomUUID();
      const now = new Date();
      await insert(conversations).values({
        id,
        title: "New conversation",
        createdAt: now,
        updatedAt: now,
      });
      setSelectedId(id);
    },
    [],
  );

  return (
    <div className="flex h-full">
      <LiveQueries
        queries={{
          conversations: {
            table: conversations,
            orderBy: (t) => desc(t.updatedAt),
            limit: CONVERSATION_LIMIT,
          },
          messages: {
            table: messages,
            where: (t) =>
              selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__"),
            orderBy: (t) => asc(t.createdAt),
            limit: MESSAGE_LIMIT,
            deps: [selectedId],
          },
          typing: {
            table: typingTokens,
            where: (t) =>
              selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__"),
            orderBy: (t) => asc(t.seq),
            limit: TYPING_LIMIT,
            deps: [selectedId],
          },
          approvals: {
            table: approvals,
            where: (t) =>
              selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__"),
            orderBy: (t) => asc(t.createdAt),
            limit: APPROVAL_LIMIT,
            deps: [selectedId],
          },
          tasks: {
            table: tasks,
            where: (t) =>
              selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__"),
            orderBy: (t) => desc(t.startedAt),
            limit: TASK_LIMIT,
            deps: [selectedId],
          },
        }}
      >
        {(ctx) => {
          // "Agent is busy" = any task hasn't been finalized OR any message is
          // still mid-flight. Both signals matter: relying on isCancelled
          // alone would re-enable the composer the moment Stop is clicked,
          // before the agent has flushed and finalized.
          const activeTask = ctx.tasks.rows.find((t) => !t.finishedAt) ?? null;
          const hasPendingMessage = ctx.messages.rows.some(
            (m) => m.status === "pending" || m.status === "streaming",
          );
          const isAgentBusy = Boolean(activeTask) || hasPendingMessage;

          return (
            <>
              <Sidebar
                conversations={ctx.conversations.rows}
                selectedId={selectedId}
                onSelect={setSelectedId}
                onCreate={() => createConversation(ctx.insert)}
              />
              <main className="flex-1 flex flex-col min-h-0 bg-[var(--background)]">
                {selectedId ? (
                  <Conversation
                    conversationId={selectedId}
                    conversation={ctx.conversations.rows.find((c) => c.id === selectedId) ?? null}
                    messages={ctx.messages.rows}
                    typingTokens={ctx.typing.rows}
                    approvals={ctx.approvals.rows}
                    activeTask={activeTask}
                    isAgentBusy={isAgentBusy}
                    insert={ctx.insert}
                    update={ctx.update}
                  />
                ) : (
                  <Welcome onCreate={() => createConversation(ctx.insert)} />
                )}
              </main>
            </>
          );
        }}
      </LiveQueries>
    </div>
  );
}
