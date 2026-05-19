import React from "react";
import { User, Sparkles, Check, X, ShieldQuestion } from "lucide-react";
import type { InferSelectModel } from "drizzle-orm";
import type {
  messages as MessagesTable,
  typingTokens as TokensTable,
  approvals as ApprovalsTable,
} from "@/schema";
import { cn, formatTime } from "@/lib/utils";
import { MarkdownBody } from "./MarkdownBody";

type Message = InferSelectModel<typeof MessagesTable>;
type Token = InferSelectModel<typeof TokensTable>;
type Approval = InferSelectModel<typeof ApprovalsTable>;

interface MessagesProps {
  messages: Message[];
  typingTokens: Token[];
  approvals: Approval[];
  onApproval: (id: string, decision: "approved" | "rejected") => Promise<void>;
}

export function Messages({ messages, typingTokens, approvals, onApproval }: MessagesProps) {
  const tokensByMessage = React.useMemo(() => {
    const map = new Map<string, Token[]>();
    for (const t of typingTokens) {
      const list = map.get(t.messageId) ?? [];
      list.push(t);
      map.set(t.messageId, list);
    }
    return map;
  }, [typingTokens]);

  const approvalsByMessage = React.useMemo(() => {
    const map = new Map<string, Approval[]>();
    for (const a of approvals) {
      const list = map.get(a.messageId) ?? [];
      list.push(a);
      map.set(a.messageId, list);
    }
    return map;
  }, [approvals]);

  // CSS keyframe animations attach to the DOM node, not to React's virtual
  // DOM. As long as each <li> has a stable key (m.id), React reuses the same
  // DOM element across renders so the animation only fires on first mount —
  // exactly what we want for streaming where messages update ~10x/sec.

  if (messages.length === 0) {
    return (
      <p className="text-sm text-[var(--muted-foreground)] text-center py-12">
        Send a message to start the conversation.
      </p>
    );
  }

  return (
    <ul className="space-y-5">
      {messages.map((m) => {
        const tokens = tokensByMessage.get(m.id) ?? [];
        const streamed = tokens.map((t) => t.body).join("");
        const displayBody = m.status === "streaming" && streamed.length > 0 ? streamed : m.body;
        const isUser = m.role === "user";
        const messageApprovals = approvalsByMessage.get(m.id) ?? [];
        const isPending = !displayBody && (m.status === "pending" || m.status === "streaming");

        return (
          <li
            key={m.id}
            className={cn("flex gap-3 animate-slide-up", isUser ? "flex-row-reverse" : "")}
          >
            <div
              className={cn(
                "size-8 rounded-xl flex items-center justify-center shrink-0 shadow-sm",
                isUser
                  ? "bg-[var(--surface-elevated)] border border-[var(--border)]"
                  : "bg-[var(--accent)] shadow-[0_0_24px_var(--accent-glow)]",
              )}
            >
              {isUser ? (
                <User className="size-4 opacity-70" />
              ) : (
                <Sparkles className="size-4 text-[var(--accent-foreground)]" />
              )}
            </div>
            <div
              className={cn(
                "max-w-[78%] flex flex-col gap-2",
                isUser ? "items-end" : "items-start",
              )}
            >
              <div
                className={cn(
                  "rounded-2xl px-4 py-2.5 text-[14px] leading-relaxed shadow-sm",
                  isUser
                    ? "bg-[var(--accent)] text-[var(--accent-foreground)] rounded-br-md whitespace-pre-wrap user-bubble"
                    : "bg-[var(--surface)] backdrop-blur-md border border-[var(--surface-border)] rounded-bl-md",
                  m.status === "cancelled" && "opacity-60 italic",
                  m.status === "error" && "border-[var(--destructive)]",
                )}
              >
                {isPending ? (
                  <span className="inline-flex gap-1.5 items-center">
                    <span className="size-1.5 rounded-full bg-current animate-pulse-soft" />
                    <span className="size-1.5 rounded-full bg-current animate-pulse-soft [animation-delay:0.15s]" />
                    <span className="size-1.5 rounded-full bg-current animate-pulse-soft [animation-delay:0.3s]" />
                  </span>
                ) : isUser ? (
                  displayBody
                ) : (
                  <MarkdownBody>{displayBody}</MarkdownBody>
                )}
                {m.status === "cancelled" && (
                  <span className="block mt-1.5 text-[11px] opacity-70">(stopped)</span>
                )}
              </div>

              {messageApprovals.map((a) => (
                <div
                  key={a.id}
                  className={cn(
                    "rounded-2xl border bg-[var(--surface)] backdrop-blur-md p-4 text-sm space-y-3 w-full max-w-md",
                    a.status === "pending"
                      ? "border-[var(--accent)]/40 shadow-[0_0_28px_var(--accent-glow)]"
                      : "border-[var(--surface-border)]",
                  )}
                >
                  <div className="flex items-start gap-2.5">
                    <div className="size-7 rounded-lg bg-[var(--accent)]/10 border border-[var(--accent)]/30 flex items-center justify-center shrink-0">
                      <ShieldQuestion className="size-4 text-[var(--accent)]" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-[11px] font-mono text-[var(--muted-foreground)] uppercase tracking-widest mb-1">
                        Approval required
                      </p>
                      <p className="text-[14px] leading-relaxed">{a.question}</p>
                    </div>
                  </div>
                  {a.status === "pending" ? (
                    <div className="flex gap-2 justify-end">
                      <button
                        onClick={() => onApproval(a.id, "rejected")}
                        className="px-3.5 py-1.5 text-[13px] rounded-lg border border-[var(--surface-border)] hover:border-[var(--destructive)]/60 hover:text-[var(--destructive)] inline-flex items-center gap-1.5 transition"
                      >
                        <X className="size-3.5" /> Reject
                      </button>
                      <button
                        onClick={() => onApproval(a.id, "approved")}
                        className="px-3.5 py-1.5 text-[13px] rounded-lg bg-[var(--accent)] text-[var(--accent-foreground)] font-medium shadow-[0_0_20px_var(--accent-glow)] inline-flex items-center gap-1.5 hover:brightness-110 transition"
                      >
                        <Check className="size-3.5" /> Approve
                      </button>
                    </div>
                  ) : (
                    <p className="text-[12px] text-[var(--muted-foreground)] flex items-center gap-1.5 font-mono">
                      {a.status === "approved" ? (
                        <>
                          <Check className="size-3.5 text-[var(--accent)]" /> Approved
                        </>
                      ) : (
                        <>
                          <X className="size-3.5 text-[var(--destructive)]" /> Rejected
                        </>
                      )}
                      {a.resolvedAt ? (
                        <span className="opacity-60"> {formatTime(a.resolvedAt)}</span>
                      ) : null}
                    </p>
                  )}
                </div>
              ))}

              <p className="text-[10px] text-[var(--muted-foreground)] px-1 opacity-60">
                {formatTime(m.createdAt)}
              </p>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
