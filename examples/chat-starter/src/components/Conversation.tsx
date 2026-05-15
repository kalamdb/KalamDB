import React from "react";
import type { InferSelectModel } from "drizzle-orm";
import { approvals, conversations, messages, tasks, typingTokens } from "@/schema";
import { Messages } from "./Messages";
import { Composer } from "./Composer";

type Conversation = InferSelectModel<typeof conversations>;
type Message = InferSelectModel<typeof messages>;
type Token = InferSelectModel<typeof typingTokens>;
type Approval = InferSelectModel<typeof approvals>;
type Task = InferSelectModel<typeof tasks>;

interface ConversationProps {
  conversationId: string;
  conversation: Conversation | null;
  messages: Message[];
  typingTokens: Token[];
  approvals: Approval[];
  activeTask: Task | null;
  insert: any;
  update: any;
}

export function Conversation(props: ConversationProps) {
  const scrollRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [props.messages.length, props.typingTokens.length]);

  async function sendUserMessage(body: string) {
    if (!body.trim()) return;
    const now = new Date();
    const userMessageId = crypto.randomUUID();
    await props.insert(messages).values({
      id: userMessageId,
      conversationId: props.conversationId,
      role: "user",
      body,
      status: "final",
      createdAt: now,
      updatedAt: now,
    });

    const assistantId = crypto.randomUUID();
    await props.insert(messages).values({
      id: assistantId,
      conversationId: props.conversationId,
      role: "assistant",
      body: "",
      status: "pending",
      createdAt: new Date(),
      updatedAt: new Date(),
    });
    await props.insert(tasks).values({
      id: crypto.randomUUID(),
      conversationId: props.conversationId,
      messageId: assistantId,
      isCancelled: false,
      startedAt: new Date(),
      finishedAt: null,
    });

    await props.update(conversations, props.conversationId).set({
      updatedAt: new Date(),
      ...(props.messages.length === 0 ? { title: body.slice(0, 60) } : {}),
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
            {props.activeTask && <span className="ml-2 text-[var(--accent)]">  streaming</span>}
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
            isStreaming={props.activeTask !== null}
          />
        </div>
      </div>
    </>
  );
}
