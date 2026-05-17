import React from "react";
import { LiveQueries } from "@kalamdb/react";
import { asc, desc, eq } from "drizzle-orm";
import type { InferSelectModel, Table } from "drizzle-orm";
import { approvals, conversations, messages, tasks, typingTokens } from "@/schema";
import { NO_CONVERSATION_SENTINEL } from "@/lib/constants";
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

type ConversationRow = InferSelectModel<typeof conversations>;
type MessageRow = InferSelectModel<typeof messages>;
type TokenRow = InferSelectModel<typeof typingTokens>;
type ApprovalRow = InferSelectModel<typeof approvals>;
type TaskRow = InferSelectModel<typeof tasks>;

export function App() {
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

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
              selectedId
                ? eq(t.conversationId, selectedId)
                : eq(t.conversationId, NO_CONVERSATION_SENTINEL),
            orderBy: (t) => asc(t.createdAt),
            limit: MESSAGE_LIMIT,
            deps: [selectedId],
          },
          typing: {
            table: typingTokens,
            where: (t) =>
              selectedId
                ? eq(t.conversationId, selectedId)
                : eq(t.conversationId, NO_CONVERSATION_SENTINEL),
            orderBy: (t) => asc(t.seq),
            limit: TYPING_LIMIT,
            deps: [selectedId],
          },
          approvals: {
            table: approvals,
            where: (t) =>
              selectedId
                ? eq(t.conversationId, selectedId)
                : eq(t.conversationId, NO_CONVERSATION_SENTINEL),
            orderBy: (t) => asc(t.createdAt),
            limit: APPROVAL_LIMIT,
            deps: [selectedId],
          },
          tasks: {
            table: tasks,
            where: (t) =>
              selectedId
                ? eq(t.conversationId, selectedId)
                : eq(t.conversationId, NO_CONVERSATION_SENTINEL),
            orderBy: (t) => desc(t.startedAt),
            limit: TASK_LIMIT,
            deps: [selectedId],
          },
        }}
      >
        {(ctx) => (
          <ChatBody
            conversationsRows={ctx.conversations.rows as ConversationRow[]}
            messagesRows={ctx.messages.rows as MessageRow[]}
            typingRows={ctx.typing.rows as TokenRow[]}
            approvalsRows={ctx.approvals.rows as ApprovalRow[]}
            tasksRows={ctx.tasks.rows as TaskRow[]}
            insert={ctx.insert}
            update={ctx.update}
            selectedId={selectedId}
            setSelectedId={setSelectedId}
          />
        )}
      </LiveQueries>
    </div>
  );
}

// Inner component: hosts the auto-deselect effect (the render-prop body of
// <LiveQueries> can't host hooks directly). Typed row props let TS catch
// shape drift between schema.ts and the consumers below.
interface ChatBodyProps {
  conversationsRows: ConversationRow[];
  messagesRows: MessageRow[];
  typingRows: TokenRow[];
  approvalsRows: ApprovalRow[];
  tasksRows: TaskRow[];
  // LiveQueries' insert/update helpers — typed at the consumer interface
  // (Conversation expects the same shape). The SDK's exported generics
  // require pinning to a specific LiveQueriesDefinition, which we avoid
  // here by accepting the consumer-friendly signature.
  insert: <T extends Table>(
    table: T,
  ) => { values: (row: Record<string, unknown>) => Promise<unknown> };
  update: <T extends Table>(
    table: T,
    id: string,
  ) => { set: (patch: Record<string, unknown>) => Promise<unknown> };
  selectedId: string | null;
  setSelectedId: (id: string | null) => void;
}

function ChatBody(props: ChatBodyProps) {
  const {
    conversationsRows,
    messagesRows,
    typingRows,
    approvalsRows,
    tasksRows,
    insert,
    update,
    selectedId,
    setSelectedId,
  } = props;

  // When the selected conversation disappears from live results (e.g.,
  // delete_conversation just cascaded), drop selectedId so the UI returns
  // to the Welcome screen instead of leaving the user "inside" a now-empty
  // conversation row.
  React.useEffect(() => {
    if (!selectedId) return;
    const stillExists = conversationsRows.some((c) => c.id === selectedId);
    if (!stillExists) setSelectedId(null);
  }, [selectedId, conversationsRows, setSelectedId]);

  const createConversation = React.useCallback(async () => {
    const id = crypto.randomUUID();
    const now = new Date();
    await insert(conversations).values({
      id,
      title: "New conversation",
      createdAt: now,
      updatedAt: now,
    });
    setSelectedId(id);
  }, [insert, setSelectedId]);

  // "Agent is busy" = any task hasn't been finalized OR any message is still
  // mid-flight. Both signals matter: relying on isCancelled alone would
  // re-enable the composer the moment Stop is clicked, before the agent has
  // flushed and finalized.
  const activeTask = tasksRows.find((t) => !t.finishedAt) ?? null;
  const hasPendingMessage = messagesRows.some(
    (m) => m.status === "pending" || m.status === "streaming",
  );
  const isAgentBusy = Boolean(activeTask) || hasPendingMessage;

  return (
    <>
      <Sidebar
        conversations={conversationsRows}
        selectedId={selectedId}
        onSelect={setSelectedId}
        onCreate={createConversation}
      />
      <main className="flex-1 flex flex-col min-h-0">
        {selectedId ? (
          <Conversation
            conversationId={selectedId}
            conversation={conversationsRows.find((c) => c.id === selectedId) ?? null}
            messages={messagesRows}
            typingTokens={typingRows}
            approvals={approvalsRows}
            activeTask={activeTask}
            isAgentBusy={isAgentBusy}
            insert={insert}
            update={update}
          />
        ) : (
          <Welcome onCreate={createConversation} />
        )}
      </main>
    </>
  );
}
