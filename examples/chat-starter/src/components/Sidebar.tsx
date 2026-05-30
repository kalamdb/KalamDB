import { Plus, MessageSquare } from "lucide-react";
import type { InferSelectModel } from "drizzle-orm";
import type { conversations as ConversationsTable } from "@/schema";
import { cn, formatTime } from "@/lib/utils";
import { UserPicker } from "./UserPicker";

type ConversationRow = InferSelectModel<typeof ConversationsTable>;

interface SidebarProps {
  conversations: ConversationRow[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onCreate: () => Promise<void> | void;
  currentUser: string;
  onUserChange: (next: string) => void;
}

export function Sidebar({
  conversations: convos,
  selectedId,
  onSelect,
  onCreate,
  currentUser,
  onUserChange,
}: SidebarProps) {
  return (
    <aside className="w-72 shrink-0 border-r border-[var(--surface-border)] flex flex-col min-h-0 bg-[var(--background-alt)]/60 backdrop-blur-xl">
      <div className="p-4 border-b border-[var(--surface-border)] flex items-end gap-2">
        <img
          src="/kalamdb-logo-dark.png"
          alt="KalamDB"
          className="h-7 w-auto object-contain"
        />
        <span className="font-semibold text-sm tracking-tight text-[var(--muted-foreground)] leading-none pb-0.5">
          Chat
        </span>
      </div>
      <div className="p-3">
        <button
          onClick={() => void onCreate()}
          className="w-full flex items-center gap-2 px-3 py-2 rounded-lg bg-[var(--accent)] text-[var(--accent-foreground)] text-sm font-medium hover:brightness-110 shadow-[0_0_18px_var(--accent-glow)] transition"
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
                      ? "bg-[var(--surface)] border border-[var(--surface-border)]"
                      : "border border-transparent hover:bg-[var(--surface)]/40 text-[var(--muted-foreground)] hover:text-[var(--foreground)]",
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
      <div className="border-t border-[var(--surface-border)] p-3 space-y-3">
        <UserPicker currentUser={currentUser} onUserChange={onUserChange} />
        <div className="text-[10px] text-[var(--muted-foreground)] font-mono tracking-wide">
          powered by <span className="text-[var(--accent)]">kalamdb</span> live queries
        </div>
      </div>
    </aside>
  );
}
