import React from "react";
import { ArrowUp, Square } from "lucide-react";

interface ComposerProps {
  onSend: (body: string) => Promise<void>;
  onStop: () => Promise<void>;
  isStreaming: boolean;
  canStop: boolean;
}

export function Composer({ onSend, onStop, isStreaming, canStop }: ComposerProps) {
  const [draft, setDraft] = React.useState("");
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);

  React.useEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }, [draft]);

  async function submit() {
    const body = draft.trim();
    if (!body || isStreaming) return;
    // Clear the input optimistically so the next character the user types
    // doesn't land on top of what they just sent. If onSend throws (network
    // down, KalamDB rejecting the insert) we restore the draft so the user
    // doesn't lose what they typed.
    setDraft("");
    try {
      await onSend(body);
    } catch (err) {
      setDraft(body);
      throw err;
    }
  }

  function handleKey(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
      className="relative flex items-end gap-2 rounded-2xl border border-[var(--surface-border)] bg-[var(--surface)] backdrop-blur-xl px-3.5 py-2.5 focus-within:border-[var(--accent)]/40 focus-within:shadow-[0_0_24px_var(--accent-glow)] transition-all"
    >
      <textarea
        ref={textareaRef}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={handleKey}
        placeholder={isStreaming ? "Streaming  press Stop to cancel" : "Message KalamDB Chat"}
        rows={1}
        // Deliberately disabled while the agent is busy — the starter has no
        // "queued send" UX. Drafting+queueing while a reply streams in is a
        // worthwhile addition for a real product (ChatGPT et al. do it) but
        // would couple the agent to the idea of a pending-message queue,
        // which isn't a primitive the chat-starter wants to demo.
        disabled={isStreaming}
        className="flex-1 resize-none bg-transparent outline-none text-[14px] leading-relaxed py-1.5 disabled:opacity-60 placeholder:text-[var(--muted-foreground)]"
      />
      {isStreaming ? (
        <button
          type="button"
          onClick={() => void onStop()}
          disabled={!canStop}
          aria-label="Stop"
          className="size-9 rounded-xl bg-[var(--destructive)] text-white inline-flex items-center justify-center hover:brightness-110 disabled:opacity-50 disabled:cursor-not-allowed shadow-[0_0_16px_rgba(255,92,114,0.3)] transition"
        >
          <Square className="size-4" fill="currentColor" />
        </button>
      ) : (
        <button
          type="submit"
          aria-label="Send"
          disabled={!draft.trim()}
          className="size-9 rounded-xl bg-[var(--accent)] text-[var(--accent-foreground)] inline-flex items-center justify-center disabled:opacity-30 disabled:cursor-not-allowed shadow-[0_0_20px_var(--accent-glow)] hover:brightness-110 transition"
        >
          <ArrowUp className="size-4 stroke-[2.5]" />
        </button>
      )}
    </form>
  );
}
