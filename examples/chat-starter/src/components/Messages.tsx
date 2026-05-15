import React from "react";
import { User, Sparkles, Check, X, ShieldQuestion } from "lucide-react";
import type { InferSelectModel } from "drizzle-orm";
import type { messages as MessagesTable, typingTokens as TokensTable, approvals as ApprovalsTable } from "@/schema";
import { cn, formatTime } from "@/lib/utils";

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
        const displayBody =
          m.status === "streaming" && streamed.length > 0 ? streamed : m.body;
        const isUser = m.role === "user";
        const messageApprovals = approvalsByMessage.get(m.id) ?? [];
        const isPending = !displayBody && (m.status === "pending" || m.status === "streaming");

        return (
          <li key={m.id} className={cn("flex gap-3 animate-slide-up", isUser ? "flex-row-reverse" : "")}>
            <div
              className={cn(
                "size-8 rounded-xl flex items-center justify-center shrink-0 shadow-sm",
                isUser
                  ? "bg-[var(--surface-elevated)] border border-[var(--border)]"
                  : "bg-gradient-to-br from-[var(--accent)] to-purple-500 shadow-[var(--accent-glow)]",
              )}
            >
              {isUser ? (
                <User className="size-4 opacity-70" />
              ) : (
                <Sparkles className="size-4 text-white" />
              )}
            </div>
            <div className={cn("max-w-[78%] flex flex-col gap-2", isUser ? "items-end" : "items-start")}>
              <div
                className={cn(
                  "rounded-2xl px-4 py-2.5 text-[14px] leading-relaxed whitespace-pre-wrap shadow-sm",
                  isUser
                    ? "bg-gradient-to-br from-[var(--accent)] to-purple-500 text-white rounded-br-md"
                    : "bg-[var(--surface)] border border-[var(--border)] rounded-bl-md",
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
                ) : (
                  displayBody
                )}
                {m.status === "cancelled" && (
                  <span className="block mt-1.5 text-[11px] opacity-70">(stopped)</span>
                )}
              </div>

              {messageApprovals.map((a) => (
                <div
                  key={a.id}
                  className={cn(
                    "rounded-2xl border bg-[var(--surface)] p-4 text-sm space-y-3 shadow-sm w-full max-w-md",
                    a.status === "pending"
                      ? "border-[var(--accent)]/40 shadow-[var(--accent-glow)]"
                      : "border-[var(--border)]",
                  )}
                >
                  <div className="flex items-start gap-2.5">
                    <div className="size-7 rounded-lg bg-[var(--accent)]/10 flex items-center justify-center shrink-0">
                      <ShieldQuestion className="size-4 text-[var(--accent)]" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-[12px] font-medium text-[var(--muted-foreground)] uppercase tracking-wide mb-1">
                        Approval required
                      </p>
                      <p className="text-[14px] leading-relaxed">{a.question}</p>
                    </div>
                  </div>
                  {a.status === "pending" ? (
                    <div className="flex gap-2 justify-end">
                      <button
                        onClick={() => onApproval(a.id, "rejected")}
                        className="px-3.5 py-1.5 text-[13px] rounded-lg border border-[var(--border)] hover:bg-[var(--muted)] inline-flex items-center gap-1.5 transition-colors"
                      >
                        <X className="size-3.5" /> Reject
                      </button>
                      <button
                        onClick={() => onApproval(a.id, "approved")}
                        className="px-3.5 py-1.5 text-[13px] rounded-lg bg-gradient-to-r from-[var(--accent)] to-purple-500 text-white shadow-md shadow-[var(--accent-glow)] inline-flex items-center gap-1.5 hover:shadow-lg transition-shadow"
                      >
                        <Check className="size-3.5" /> Approve
                      </button>
                    </div>
                  ) : (
                    <p className="text-[12px] text-[var(--muted-foreground)] flex items-center gap-1.5">
                      {a.status === "approved" ? (
                        <><Check className="size-3.5 text-emerald-500" /> Approved</>
                      ) : (
                        <><X className="size-3.5 text-red-500" /> Rejected</>
                      )}
                      {a.resolvedAt ? <span className="opacity-60"> {formatTime(a.resolvedAt)}</span> : null}
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
