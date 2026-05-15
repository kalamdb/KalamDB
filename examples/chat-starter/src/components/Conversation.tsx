import React from "react";
import type { InferSelectModel, Table } from "drizzle-orm";
import { approvals, conversations, messages, tasks, typingTokens } from "@/schema";
import { Messages } from "./Messages";
import { Composer } from "./Composer";

type ConversationRow = InferSelectModel<typeof conversations>;
type MessageRow = InferSelectModel<typeof messages>;
type TokenRow = InferSelectModel<typeof typingTokens>;
type ApprovalRow = InferSelectModel<typeof approvals>;
type TaskRow = InferSelectModel<typeof tasks>;

type InsertFn = <T extends Table>(table: T) => { values: (row: Record<string, unknown>) => Promise<unknown> };
type UpdateFn = <T extends Table>(table: T, id: string) => { set: (patch: Record<string, unknown>) => Promise<unknown> };

interface ConversationProps {
  conversationId: string;
  conversation: ConversationRow | null;
  messages: MessageRow[];
  typingTokens: TokenRow[];
  approvals: ApprovalRow[];
  activeTask: TaskRow | null;
  isAgentBusy: boolean;
  insert: InsertFn;
  update: UpdateFn;
}

export function Conversation(props: ConversationProps) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const lastUserMessageIdRef = React.useRef<string | null>(null);

  // Auto-scroll: smooth when the user just sent a message (so they see the
  // movement), instant during streaming token deltas (~10/s otherwise creates
  // a fight between queued smooth animations).
  React.useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const latestUserMessage = [...props.messages].reverse().find((m) => m.role === "user");
    const justSentByUser = latestUserMessage && latestUserMessage.id !== lastUserMessageIdRef.current;
    lastUserMessageIdRef.current = latestUserMessage?.id ?? null;
    el.scrollTo({
      top: el.scrollHeight,
      behavior: justSentByUser ? "smooth" : "instant",
    });
  }, [props.messages, props.typingTokens.length]);

  async function sendUserMessage(body: string) {
    if (!body.trim()) return;
    const now = new Date();
    const isFirstMessage = props.messages.length === 0;
    const userMessageId = crypto.randomUUID();
    const assistantId = crypto.randomUUID();

    // Two writes only: the user's message and the task. The agent inserts the
    // assistant message row on the receiving end (idempotently) so we never
    // leave an orphan 'pending' assistant row if the task insert fails.
    await props.insert(messages).values({
      id: userMessageId,
      conversationId: props.conversationId,
      role: "user",
      body,
      status: "final",
      createdAt: now,
      updatedAt: now,
    });
    await props.insert(tasks).values({
      id: crypto.randomUUID(),
      conversationId: props.conversationId,
      messageId: assistantId,
      isCancelled: false,
      startedAt: now,
      finishedAt: null,
    });

    await props.update(conversations, props.conversationId).set({
      updatedAt: now,
      ...(isFirstMessage ? { title: body.slice(0, 60) } : {}),
    });
  }

  async function stopTask() {
    if (!props.activeTask) return;
    await props.update(tasks, props.activeTask.id).set({ isCancelled: true });
  }

  async function respondApproval(approvalId: string, decision: "approved" | "rejected") {
    await props.update(approvals, approvalId).set({
      status: decision,
      resolvedAt: new Date(),
    });
  }

  return (
    <>
      <header className="border-b border-[var(--border)] bg-[var(--surface)]/60 backdrop-blur-xl px-6 py-3.5 flex items-center justify-between">
        <div className="min-w-0">
          <h1 className="text-sm font-semibold truncate tracking-tight">
            {props.conversation?.title ?? "Conversation"}
          </h1>
          <p className="text-[11px] text-[var(--muted-foreground)] mt-0.5">
            {props.messages.length} message{props.messages.length === 1 ? "" : "s"}
            {props.isAgentBusy && <span className="ml-2 text-[var(--accent)]">  streaming</span>}
          </p>
        </div>
      </header>

      <div ref={scrollRef} className="flex-1 overflow-y-auto px-6 py-6">
        <div className="max-w-2xl mx-auto">
          <Messages
            messages={props.messages}
            typingTokens={props.typingTokens}
            approvals={props.approvals}
            onApproval={respondApproval}
          />
        </div>
      </div>

      <div className="border-t border-[var(--border)] bg-[var(--surface)]/60 backdrop-blur-xl px-6 py-4">
        <div className="max-w-2xl mx-auto">
          <Composer
            onSend={sendUserMessage}
            onStop={stopTask}
            isStreaming={props.isAgentBusy}
            canStop={props.activeTask !== null && !props.activeTask.isCancelled}
          />
        </div>
      </div>
    </>
  );
}
