import React from "react";
import { MessageSquarePlus, Sparkles, Zap, ShieldCheck, Square } from "lucide-react";

const FEATURES = [
  { icon: Zap, title: "Streaming", body: "Each LLM token is a row in `typing_tokens`. UI subscribes; no SSE plumbing." },
  { icon: ShieldCheck, title: "Approvals", body: "The agent calls `request_approval`. UI shows it; one click resolves." },
  { icon: Square, title: "Stop, live", body: "Stop button is an UPDATE. The agent watches its own row and aborts." },
];

export function Welcome({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex-1 flex items-center justify-center p-8 animate-slide-up">
      <div className="max-w-xl w-full text-center space-y-8">
        <div className="relative inline-flex">
          <div className="absolute inset-0 rounded-full bg-gradient-to-br from-[var(--accent)] to-purple-500 blur-2xl opacity-40" />
          <div className="relative inline-flex items-center justify-center size-16 rounded-2xl bg-gradient-to-br from-[var(--accent)] to-purple-500 shadow-xl shadow-[var(--accent-glow)]">
            <Sparkles className="size-7 text-white" />
          </div>
        </div>

        <div className="space-y-3">
          <h1 className="text-3xl font-semibold tracking-tight">KalamDB Chat Starter</h1>
          <p className="text-sm text-[var(--muted-foreground)] leading-relaxed max-w-md mx-auto">
            Live queries power every real-time behavior in this app  streaming, approvals,
            cancellation. No bespoke WebSocket plumbing.
          </p>
        </div>

        <div className="grid sm:grid-cols-3 gap-3 text-left">
          {FEATURES.map((f) => (
            <div
              key={f.title}
              className="rounded-xl border border-[var(--border)] bg-[var(--surface)]/60 backdrop-blur-sm p-4 hover:border-[var(--accent)]/50 transition-colors"
            >
              <f.icon className="size-4 text-[var(--accent)] mb-2" />
              <h3 className="text-sm font-medium">{f.title}</h3>
              <p className="text-xs text-[var(--muted-foreground)] mt-1 leading-relaxed">{f.body}</p>
            </div>
          ))}
        </div>

        <button
          onClick={onCreate}
          className="inline-flex items-center gap-2 px-5 py-2.5 rounded-xl bg-gradient-to-r from-[var(--accent)] to-purple-500 text-white text-sm font-medium shadow-lg shadow-[var(--accent-glow)] hover:shadow-xl transition-shadow"
        >
          <MessageSquarePlus className="size-4" /> Start a new chat
        </button>
      </div>
    </div>
  );
}
