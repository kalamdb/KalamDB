import React from "react";
import { LiveQueries } from "@kalamdb/react";
import { asc, desc, eq } from "drizzle-orm";
import { approvals, conversations, messages, tasks, typingTokens } from "@/schema";
import { Sidebar } from "./Sidebar";
import { Conversation } from "./Conversation";
import { Welcome } from "./Welcome";

export function App() {
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  return (
    <div className="flex h-full">
      <LiveQueries
        queries={{
          conversations: {
            table: conversations,
            orderBy: (t) => desc(t.updatedAt),
            limit: 100,
          },
          messages: {
            table: messages,
            where: (t) => (selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__")),
            orderBy: (t) => asc(t.createdAt),
            deps: [selectedId],
          },
          typing: {
            table: typingTokens,
            where: (t) => (selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__")),
            orderBy: (t) => asc(t.seq),
            deps: [selectedId],
          },
          approvals: {
            table: approvals,
            where: (t) => (selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__")),
            orderBy: (t) => asc(t.createdAt),
            deps: [selectedId],
          },
          tasks: {
            table: tasks,
            where: (t) => (selectedId ? eq(t.conversationId, selectedId) : eq(t.conversationId, "__none__")),
            orderBy: (t) => desc(t.startedAt),
            deps: [selectedId],
          },
        }}
      >
        {(ctx) => (
          <>
            <Sidebar
              conversations={ctx.conversations.rows}
              selectedId={selectedId}
              onSelect={setSelectedId}
              insert={ctx.insert}
            />
            <main className="flex-1 flex flex-col min-h-0 bg-[var(--background)]">
              {selectedId ? (
                <Conversation
                  conversationId={selectedId}
                  conversation={ctx.conversations.rows.find((c) => c.id === selectedId) ?? null}
                  messages={ctx.messages.rows}
                  typingTokens={ctx.typing.rows}
                  approvals={ctx.approvals.rows}
                  activeTask={ctx.tasks.rows.find((t) => !t.finishedAt && !t.isCancelled) ?? null}
                  insert={ctx.insert}
                  update={ctx.update}
                />
              ) : (
                <Welcome onCreate={async () => {
                  const id = crypto.randomUUID();
                  await ctx.insert(conversations).values({
                    id,
                    title: "New conversation",
                    createdAt: new Date(),
                    updatedAt: new Date(),
                  });
                  setSelectedId(id);
                }} />
              )}
            </main>
          </>
        )}
      </LiveQueries>
    </div>
  );
}
