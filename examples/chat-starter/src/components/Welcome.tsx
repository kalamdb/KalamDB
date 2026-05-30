import { MessageSquarePlus, Zap, ShieldCheck, Square, BookOpen } from "lucide-react";

const FEATURES = [
  {
    icon: Zap,
    title: "Streaming",
    body: "Replies appear live as the model thinks — no waiting for the full answer.",
  },
  {
    icon: ShieldCheck,
    title: "Approvals",
    body: "The app pauses before anything destructive. One click and it continues.",
  },
  {
    icon: Square,
    title: "Stop, live",
    body: "Hit Stop mid-response — the agent halts instantly.",
  },
  {
    icon: BookOpen,
    title: "RAG · Ask your KB",
    body: "Ask anything about your knowledge base. Instant answers, with sources.",
  },
];

export function Welcome({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex-1 flex items-center justify-center p-8 animate-slide-up">
      <div className="max-w-3xl w-full text-center space-y-8">
        <div className="relative inline-flex">
          <div className="absolute inset-0 bg-[var(--accent)] blur-3xl opacity-25" />
          <img
            src="/kalamdb-logo-dark.png"
            alt="KalamDB"
            className="relative h-16 w-auto object-contain"
          />
        </div>

        <div className="space-y-3">
          <h1 className="text-3xl font-semibold tracking-tight">KalamDB Chat Starter</h1>
          <p className="text-sm text-[var(--muted-foreground)] leading-relaxed max-w-md mx-auto">
            Live queries power every real-time behavior in this app — streaming, approvals,
            cancellation. No bespoke WebSocket/SSE plumbing.
          </p>
        </div>

        <div className="grid sm:grid-cols-2 lg:grid-cols-4 gap-3 text-left">
          {FEATURES.map((f) => (
            <div
              key={f.title}
              className="rounded-xl border border-[var(--surface-border)] bg-[var(--surface)] backdrop-blur-md p-4 hover:border-[var(--accent)]/40 transition-colors"
            >
              <f.icon className="size-4 text-[var(--accent)] mb-2" />
              <h3 className="text-sm font-medium">{f.title}</h3>
              <p className="text-xs text-[var(--muted-foreground)] mt-1 leading-relaxed">
                {f.body}
              </p>
            </div>
          ))}
        </div>

        <button
          onClick={onCreate}
          className="inline-flex items-center gap-2 px-5 py-2.5 rounded-xl bg-[var(--accent)] text-[var(--accent-foreground)] text-sm font-semibold shadow-[0_0_28px_var(--accent-glow)] hover:brightness-110 transition"
        >
          <MessageSquarePlus className="size-4" /> Start a new chat
        </button>
      </div>
    </div>
  );
}
