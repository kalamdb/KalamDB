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
    setDraft("");
    await onSend(body);
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
      className="relative flex items-end gap-2 rounded-2xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2.5 shadow-xl shadow-black/5 focus-within:border-[var(--accent)]/60 focus-within:shadow-[var(--accent-glow)] transition-all"
    >
      <textarea
        ref={textareaRef}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={handleKey}
        placeholder={isStreaming ? "Streaming  press Stop to cancel" : "Message KalamDB Chat"}
        rows={1}
        disabled={isStreaming}
        className="flex-1 resize-none bg-transparent outline-none text-[14px] leading-relaxed py-1.5 disabled:opacity-60 placeholder:text-[var(--muted-foreground)]"
      />
      {isStreaming ? (
        <button
          type="button"
          onClick={() => void onStop()}
          disabled={!canStop}
          aria-label="Stop"
          className="size-9 rounded-xl bg-[var(--destructive)] text-white inline-flex items-center justify-center hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed shadow-md transition-opacity"
        >
          <Square className="size-4" fill="currentColor" />
        </button>
      ) : (
        <button
          type="submit"
          aria-label="Send"
          disabled={!draft.trim()}
          className="size-9 rounded-xl bg-gradient-to-br from-[var(--accent)] to-purple-500 text-white inline-flex items-center justify-center disabled:opacity-30 disabled:cursor-not-allowed shadow-md shadow-[var(--accent-glow)] hover:shadow-lg transition-shadow"
        >
          <ArrowUp className="size-4 stroke-[2.5]" />
        </button>
      )}
    </form>
  );
}
