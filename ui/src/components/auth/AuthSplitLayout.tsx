import { type ReactNode } from "react";

const logoUrl = `${import.meta.env.BASE_URL}branding/kalamdb_logo.png`;

interface AuthSplitLayoutProps {
  title?: string;
  description: string;
  children: ReactNode;
  panelTitle?: string;
  panelDescription?: string;
  panelFootnote?: string;
}

export default function AuthSplitLayout({
  title = "Welcome back",
  description,
  children,
  panelTitle = "KalamDB",
  panelDescription = "Operational control plane for embedded and on-prem deployments.",
  panelFootnote = "Embedded UI v2",
}: AuthSplitLayoutProps) {
  return (
    <div className="h-full min-h-0 overflow-auto bg-muted/40 p-4 md:p-8">
      <div className="mx-auto flex min-h-full max-w-7xl items-center">
        <div className="grid w-full overflow-hidden rounded-2xl border bg-card shadow-xl lg:grid-cols-2">
          <section className="relative flex min-h-[680px] items-center justify-center p-6 sm:p-10 lg:p-14">
            <div className="absolute left-6 top-6 sm:left-8 sm:top-8">
              <img
                src={logoUrl}
                alt="KalamDB"
                className="h-9 w-auto object-contain"
              />
              <p className="mt-2 text-[11px] font-medium text-muted-foreground">Admin UI</p>
            </div>

            <div className="w-full max-w-md space-y-8">
              <div className="space-y-2">
                <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">{title}</p>
                <h1 className="text-3xl font-semibold text-foreground">{panelTitle}</h1>
                <p className="text-sm text-muted-foreground">{description}</p>
              </div>
              {children}
            </div>
          </section>

          <aside className="relative hidden min-h-[680px] overflow-hidden border-l bg-[radial-gradient(circle_at_15%_15%,rgba(56,189,248,0.35),transparent_40%),radial-gradient(circle_at_70%_20%,rgba(239,68,68,0.3),transparent_38%),linear-gradient(155deg,#0f172a_0%,#1e293b_45%,#111827_100%)] lg:block">
            <div className="absolute right-8 top-8 h-14 w-14 rounded-xl border border-white/20 bg-black/55" />
            <div className="absolute inset-0">
              <div className="absolute -right-16 top-24 h-72 w-72 rotate-12 rounded-[2rem] bg-gradient-to-tr from-red-500/45 via-rose-400/25 to-transparent blur-2xl" />
              <div className="absolute right-14 top-10 h-96 w-24 rotate-[16deg] rounded-full bg-red-500/30 blur-xl" />
              <div className="absolute right-32 top-16 h-[32rem] w-24 rotate-[14deg] rounded-full bg-red-400/35 blur-xl" />
              <div className="absolute right-48 top-8 h-[34rem] w-24 rotate-[12deg] rounded-full bg-sky-300/30 blur-xl" />
              <div className="absolute left-0 top-0 h-full w-full bg-[linear-gradient(to_bottom,transparent_35%,rgba(0,0,0,0.75)_95%)]" />
            </div>

            <div className="absolute bottom-8 left-8 right-8 space-y-2 text-white">
              <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-white/70">{panelFootnote}</p>
              <h2 className="text-2xl font-semibold">{panelTitle}</h2>
              <p className="max-w-sm text-sm text-white/80">{panelDescription}</p>
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}
