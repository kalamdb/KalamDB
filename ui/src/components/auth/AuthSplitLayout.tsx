import { type ReactNode } from "react";

const logoUrl = `${import.meta.env.BASE_URL}branding/kalamdb_logo.png`;
const appFlowUrl = `${import.meta.env.BASE_URL}branding/kalamdb-app-flow.webp`;

const gridLayerStyle = {
  backgroundImage:
    "linear-gradient(to right, rgba(255,255,255,0.6) 1px, transparent 1px), linear-gradient(to bottom, rgba(255,255,255,0.6) 1px, transparent 1px)",
  backgroundSize: "44px 44px",
};

interface AuthSplitLayoutProps {
  title?: string;
  heading?: string;
  description: string;
  children: ReactNode;
  panelTitle?: string;
  panelDescription?: string;
  panelFootnote?: string;
}

export default function AuthSplitLayout({
  title = "Welcome back",
  heading,
  description,
  children,
  panelTitle = "KalamDB",
  panelDescription = "Operational control plane for embedded and on-prem deployments.",
  panelFootnote = "Embedded UI v2",
}: AuthSplitLayoutProps) {
  const formHeading = heading ?? panelTitle;

  return (
    <div className="h-full min-h-0 overflow-auto bg-muted/40 p-4 md:p-8">
      <div className="mx-auto flex min-h-full max-w-6xl items-center">
        <div className="grid w-full overflow-hidden rounded-2xl border bg-card shadow-sm lg:grid-cols-[minmax(0,0.95fr)_minmax(0,1fr)]">
          <section className="flex min-h-[560px] flex-col gap-10 p-6 sm:p-10 lg:p-14">
            <div className="mx-auto flex w-full max-w-md items-center gap-3 lg:mx-0">
              <span className="inline-flex items-center rounded-md dark:bg-white dark:px-2.5 dark:py-1.5">
                <img src={logoUrl} alt="KalamDB" className="h-8 w-auto object-contain" />
              </span>
              <span className="border-l pl-3 text-xs font-medium text-muted-foreground">Admin UI</span>
            </div>

            <div className="flex flex-1 items-center">
              <div className="mx-auto w-full max-w-md space-y-7 lg:mx-0">
                <div className="space-y-2">
                  {title && (
                    <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">{title}</p>
                  )}
                  <h1 className="text-3xl font-semibold tracking-tight text-foreground">{formHeading}</h1>
                  <p className="text-sm leading-relaxed text-muted-foreground">{description}</p>
                </div>
                {children}
              </div>
            </div>
          </section>

          <aside className="relative hidden min-h-[560px] overflow-hidden border-l bg-[#00091f] lg:block">
            <div className="relative flex h-full flex-col gap-10 p-10 lg:p-12">
              <div className="flex flex-1 items-center">
                <img
                  src={appFlowUrl}
                  alt="Your app writes with SQL to KalamDB, which serves SQL tables, live queries and durable topics to connected clients and background workers."
                  className="w-full"
                />
              </div>

              <div className="space-y-2">
                <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-teal-300/90">
                  {panelFootnote}
                </p>
                <h2 className="text-2xl font-semibold text-white">{panelTitle}</h2>
                <p className="max-w-sm text-sm leading-relaxed text-white/65">{panelDescription}</p>
              </div>
            </div>

            <div
              aria-hidden
              className="pointer-events-none absolute inset-0 bg-[radial-gradient(circle_at_15%_6%,rgba(45,212,191,0.16),transparent_55%),radial-gradient(circle_at_95%_2%,rgba(56,189,248,0.10),transparent_45%)]"
            />
            <div aria-hidden className="pointer-events-none absolute inset-0 opacity-[0.05]" style={gridLayerStyle} />
          </aside>
        </div>
      </div>
    </div>
  );
}
