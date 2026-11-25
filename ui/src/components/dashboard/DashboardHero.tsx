import type { CSSProperties, ReactNode } from "react";
import { NavLink } from "react-router-dom";
import { Braces, CheckCircle2, Database, Share2, Terminal, Wifi } from "lucide-react";
import { HeroGraphic } from "./HeroGraphic";

interface DashboardHeroProps {
  username: string;
  serverVersion?: string;
  actions?: ReactNode;
}

interface QuickLink {
  name: string;
  href: string;
  icon: typeof Terminal;
  anchor: CSSProperties;
}

const QUICK_LINKS: QuickLink[] = [
  { name: "SQL", href: "/sql", icon: Terminal, anchor: { left: 0, top: 47 } },
  { name: "Live Queries", href: "/live-queries", icon: Wifi, anchor: { left: 0, top: 185 } },
  { name: "Tables", href: "/sql", icon: Database, anchor: { right: 0, top: 30 } },
  { name: "Pub/Sub", href: "/streaming/topics", icon: Share2, anchor: { right: 0, top: 112 } },
  { name: "Functions", href: "/functions", icon: Braces, anchor: { right: 0, top: 194 } },
];

const CAPABILITIES = ["PostgreSQL Compatible", "Realtime (Pub/Sub)", "Per-tenant USER tables"];

const PILL_CLASS =
  "absolute flex w-[108px] items-center justify-center gap-1.5 rounded-full bg-[var(--kdb-surface)] px-2 py-1.5 text-[11px] font-medium text-foreground shadow-[0_4px_14px_-6px_rgb(0_0_0_/_0.25)] ring-1 ring-foreground/10 transition-transform duration-200 hover:-translate-y-0.5 active:scale-[0.98]";

export function DashboardHero({ username, serverVersion, actions }: DashboardHeroProps) {
  return (
    <section className="relative overflow-hidden rounded-2xl bg-[var(--kdb-surface)] ring-1 ring-foreground/[0.07]">
      <svg
        className="pointer-events-none absolute inset-0 size-full"
        viewBox="0 0 1200 400"
        preserveAspectRatio="none"
        aria-hidden
      >
        <defs>
          <linearGradient id="kdb-hero-wash" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--kdb-accent)" stopOpacity={0.2} />
            <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0.02} />
          </linearGradient>
          <radialGradient id="kdb-hero-blob">
            <stop offset="0%" stopColor="var(--kdb-accent)" stopOpacity={0.2} />
            <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0} />
          </radialGradient>
        </defs>
        <path
          d="M0,96 C220,22 420,150 660,96 C860,52 1020,150 1200,70 L1200,0 L0,0 Z"
          fill="url(#kdb-hero-wash)"
        />
        <ellipse cx={940} cy={210} rx={380} ry={220} fill="url(#kdb-hero-blob)" />
      </svg>

      <div className="relative">
        {actions && (
          <div className="flex flex-wrap items-center justify-end gap-2 px-5 pt-5 lg:px-8 lg:pt-6">
            {actions}
          </div>
        )}

        <div className="grid items-center gap-6 px-5 pb-6 pt-2 lg:grid-cols-[minmax(0,1fr)_auto] lg:px-8 lg:pb-8">
          <div className="min-w-0">
            <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-[var(--kdb-accent-strong)]">
              Dashboard
            </p>
            <h1 className="mt-2.5 text-[2.1rem] font-semibold leading-[1.05] tracking-tighter text-foreground lg:text-[2.6rem]">
              KalamDB for <span className="text-[var(--kdb-accent-strong)]">AI Agents</span>
            </h1>
            <p className="mt-3 max-w-[54ch] text-sm text-muted-foreground">
              Manage, monitor, and operate your KalamDB instance in real time.
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              Welcome back, {username}
              {serverVersion ? (
                <span className="ml-2 rounded-full bg-[var(--kdb-accent-soft)] px-2 py-0.5 text-[11px] font-medium text-[var(--kdb-accent-strong)]">
                  {serverVersion.startsWith("v") ? serverVersion : `v${serverVersion}`}
                </span>
              ) : null}
            </p>

            <div className="mt-5 flex flex-wrap items-center gap-x-7 gap-y-2">
              {CAPABILITIES.map((capability) => (
                <span key={capability} className="inline-flex items-center gap-2 text-sm text-foreground">
                  <CheckCircle2 className="size-4 text-[var(--kdb-accent)]" strokeWidth={2} />
                  {capability}
                </span>
              ))}
            </div>

            <div className="mt-5 flex flex-wrap gap-2 lg:hidden">
              {QUICK_LINKS.map((link) => (
                <NavLink
                  key={link.name}
                  to={link.href}
                  className="inline-flex items-center gap-1.5 rounded-full bg-[var(--kdb-surface)] px-3 py-1.5 text-[11px] font-medium ring-1 ring-foreground/10"
                >
                  <link.icon className="size-3.5 text-[var(--kdb-accent-strong)]" strokeWidth={2} />
                  {link.name}
                </NavLink>
              ))}
            </div>
          </div>

          <div className="relative hidden h-[250px] w-[470px] shrink-0 lg:block">
            <HeroGraphic />
            {QUICK_LINKS.map((link) => (
              <NavLink key={link.name} to={link.href} className={PILL_CLASS} style={link.anchor}>
                <link.icon className="size-3.5 shrink-0 text-[var(--kdb-accent-strong)]" strokeWidth={2} />
                {link.name}
              </NavLink>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
