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

type InsertFn = <T extends Table>(
  table: T,
) => { values: (row: Record<string, unknown>) => Promise<unknown> };
type UpdateFn = <T extends Table>(
  table: T,
  id: string,
) => { set: (patch: Record<string, unknown>) => Promise<unknown> };

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

  // Auto-scroll policy:
  //   - Force a smooth scroll when the user just sent a message, a new
  //     approval card appears, OR the assistant message just transitioned
  //     to a terminal status (final / cancelled / error). All three are
  //     moments the user expects to see at the bottom.
  //   - Otherwise (streaming token deltas), only scroll if the user is
  //     already near the bottom. This avoids fighting a user who
  //     deliberately scrolled up to re-read while a reply streams in.
  //
  // Effect re-runs are kept cheap by depending on stable scalars (lengths,
  // ids, status strings) instead of array references that change on every
  // parent render.
  const messageCount = props.messages.length;
  const tokenCount = props.typingTokens.length;
  const approvalCount = props.approvals.length;
  const latestUserMessageId = props.messages.findLast((m) => m.role === "user")?.id ?? null;
  const latestAssistant = props.messages.findLast((m) => m.role === "assistant");
  const latestAssistantId = latestAssistant?.id ?? null;
  const latestAssistantStatus = latestAssistant?.status ?? null;
  const lastApprovalCountRef = React.useRef<number>(0);
  const lastAssistantStatusRef = React.useRef<{ id: string | null; status: string | null }>({
    id: null,
    status: null,
  });
  const SCROLL_NEAR_BOTTOM_PX = 80;
  React.useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const justSentByUser =
      latestUserMessageId !== null && latestUserMessageId !== lastUserMessageIdRef.current;
    lastUserMessageIdRef.current = latestUserMessageId;
    const newApproval = approvalCount > lastApprovalCountRef.current;
    lastApprovalCountRef.current = approvalCount;
    // "Assistant just terminated" = same assistant row transitioned out of
    // pending/streaming. Worth a scroll because (stopped) / final body
    // lands below the previous fold.
    const prev = lastAssistantStatusRef.current;
    const TERMINAL = ["final", "cancelled", "error"];
    const justTerminated =
      latestAssistantId !== null &&
      latestAssistantId === prev.id &&
      !TERMINAL.includes(prev.status ?? "") &&
      TERMINAL.includes(latestAssistantStatus ?? "");
    lastAssistantStatusRef.current = { id: latestAssistantId, status: latestAssistantStatus };
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    const nearBottom = distanceFromBottom < SCROLL_NEAR_BOTTOM_PX;
    const shouldScroll = justSentByUser || newApproval || justTerminated || nearBottom;
    if (!shouldScroll) return;
    // Defer to the next frame so the new approval card / message bubble has
    // been measured before we read scrollHeight — otherwise the target is
    // stale and the bottom row can end up clipped under the composer.
    const handle = requestAnimationFrame(() => {
      el.scrollTo({
        top: el.scrollHeight,
        behavior: justSentByUser || newApproval || justTerminated ? "smooth" : "instant",
      });
    });
    return () => cancelAnimationFrame(handle);
  }, [
    messageCount,
    tokenCount,
    approvalCount,
    latestUserMessageId,
    latestAssistantId,
    latestAssistantStatus,
  ]);

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
      <header className="border-b border-[var(--surface-border)] bg-[var(--background-alt)]/60 backdrop-blur-xl px-6 py-3.5 flex items-center justify-between">
        <div className="min-w-0">
          <h1 className="text-sm font-semibold truncate tracking-tight">
            {props.conversation?.title ?? "Conversation"}
          </h1>
          <p className="text-[11px] text-[var(--muted-foreground)] mt-0.5 font-mono">
            {props.messages.length} message{props.messages.length === 1 ? "" : "s"}
            {props.isAgentBusy && (
              <span className="ml-2 inline-flex items-center gap-1 text-[var(--accent)]">
                <span className="size-1.5 rounded-full bg-[var(--accent)] animate-pulse-soft" />
                streaming
              </span>
            )}
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

      <div className="border-t border-[var(--surface-border)] bg-[var(--background-alt)]/60 backdrop-blur-xl px-6 py-4">
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
