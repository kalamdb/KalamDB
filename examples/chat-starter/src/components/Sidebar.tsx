import { Plus, MessageSquare, Sparkles } from "lucide-react";
import type { InferSelectModel } from "drizzle-orm";
import type { conversations as ConversationsTable } from "@/schema";
import { cn, formatTime } from "@/lib/utils";

type ConversationRow = InferSelectModel<typeof ConversationsTable>;

interface SidebarProps {
  conversations: ConversationRow[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onCreate: () => Promise<void> | void;
}

export function Sidebar({ conversations: convos, selectedId, onSelect, onCreate }: SidebarProps) {
  return (
    <aside className="w-72 shrink-0 border-r border-[var(--border)] flex flex-col min-h-0 bg-[var(--surface)]/60 backdrop-blur-xl">
      <div className="p-4 border-b border-[var(--border)] flex items-center gap-2">
        <div className="size-7 rounded-lg bg-gradient-to-br from-[var(--accent)] to-purple-500 flex items-center justify-center shadow-lg shadow-[var(--accent-glow)]">
          <Sparkles className="size-4 text-white" />
        </div>
        <span className="font-semibold text-sm tracking-tight">KalamDB Chat</span>
      </div>
      <div className="p-3">
        <button
          onClick={() => void onCreate()}
          className="w-full flex items-center gap-2 px-3 py-2 rounded-lg bg-gradient-to-r from-[var(--accent)] to-purple-500 text-white text-sm font-medium hover:opacity-95 shadow-lg shadow-[var(--accent-glow)] hover:shadow-xl transition-all"
        >
          <Plus className="size-4" /> New chat
        </button>
      </div>
      <div className="flex-1 overflow-y-auto px-2 pb-3">
        {convos.length === 0 ? (
          <p className="text-xs text-[var(--muted-foreground)] px-3 py-2">No conversations yet.</p>
        ) : (
          <ul className="space-y-0.5">
            {convos.map((c) => (
              <li key={c.id}>
                <button
                  onClick={() => onSelect(c.id)}
                  className={cn(
                    "w-full text-left flex items-start gap-2.5 px-3 py-2.5 rounded-lg text-sm transition-all",
                    c.id === selectedId
                      ? "bg-[var(--surface-elevated)] shadow-sm"
                      : "hover:bg-[var(--surface-elevated)]/60 text-[var(--muted-foreground)] hover:text-[var(--foreground)]",
                  )}
                >
                  <MessageSquare className="size-4 mt-0.5 shrink-0 opacity-60" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-medium leading-tight">{c.title}</div>
                    <div className="text-[10px] mt-0.5 opacity-60">{formatTime(c.updatedAt)}</div>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="p-3 border-t border-[var(--border)] text-[10px] text-[var(--muted-foreground)]">
        Powered by <span className="font-medium text-[var(--foreground)]">KalamDB</span> live
        queries
      </div>
    </aside>
  );
}
