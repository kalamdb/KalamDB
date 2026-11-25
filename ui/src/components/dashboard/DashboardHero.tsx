import type { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import { Braces, CheckCircle2, Database, RadioTower, Terminal, Wifi } from "lucide-react";
import { HeroGraphic } from "./HeroGraphic";
import { ACCENT_CHIP } from "./theme";

interface DashboardHeroProps {
  username: string;
  serverVersion?: string;
  actions?: ReactNode;
}

const QUICK_LINKS = [
  { name: "SQL Studio", href: "/sql", icon: Terminal, position: "left-0 top-8" },
  { name: "Live Queries", href: "/live-queries", icon: Wifi, position: "left-0 bottom-8" },
  { name: "Streaming", href: "/streaming/topics", icon: RadioTower, position: "right-0 top-8" },
  { name: "Functions", href: "/functions", icon: Braces, position: "right-0 bottom-8" },
];

const CAPABILITIES = ["PostgreSQL wire protocol", "Realtime pub/sub"];

const PILL_CLASS =
  "inline-flex items-center gap-1.5 rounded-full bg-card px-3 py-1.5 text-xs font-medium text-foreground shadow-[0_2px_10px_-6px_rgb(0_0_0_/_0.4)] ring-1 ring-foreground/10 transition-transform duration-200 hover:-translate-y-0.5 active:scale-[0.98]";

export function DashboardHero({ username, serverVersion, actions }: DashboardHeroProps) {
  return (
    <section className="overflow-hidden rounded-2xl bg-gradient-to-br from-[var(--kdb-accent-soft)] via-background to-background ring-1 ring-foreground/10">
      {actions && (
        <div className="flex flex-wrap items-center justify-end gap-2 px-5 pt-5 lg:px-8 lg:pt-6">
          {actions}
        </div>
      )}
      <div className="grid items-center gap-6 px-5 pb-5 pt-2 lg:grid-cols-[minmax(0,1fr)_auto] lg:px-8 lg:pb-6">
        <div className="min-w-0">
          <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-[var(--kdb-accent-strong)]">
            Dashboard
          </p>
          <h1 className="mt-2 text-3xl font-semibold tracking-tighter text-foreground lg:text-4xl">
            KalamDB for <span className="text-[var(--kdb-accent-strong)]">AI Agents</span>
          </h1>
          <p className="mt-2 max-w-[52ch] text-sm text-muted-foreground">
            Manage, monitor, and operate your KalamDB instance in real time.
          </p>
          <p className="mt-1 text-sm text-muted-foreground">Welcome back, {username}</p>

          <div className="mt-5 flex flex-wrap items-center gap-2">
            {CAPABILITIES.map((capability) => (
              <span key={capability} className={ACCENT_CHIP}>
                <CheckCircle2 className="size-3.5" strokeWidth={2} />
                {capability}
              </span>
            ))}
            {serverVersion && (
              <span className={ACCENT_CHIP}>
                <Database className="size-3.5" strokeWidth={2} />
                {serverVersion.startsWith("v") ? serverVersion : `v${serverVersion}`}
              </span>
            )}
          </div>

          <div className="mt-4 flex flex-wrap gap-2 lg:hidden">
            {QUICK_LINKS.map((link) => (
              <NavLink key={link.name} to={link.href} className={PILL_CLASS}>
                <link.icon className="size-3.5 text-[var(--kdb-accent-strong)]" strokeWidth={2} />
                {link.name}
              </NavLink>
            ))}
          </div>
        </div>

        <div className="relative hidden h-[210px] w-[470px] shrink-0 items-center justify-center lg:flex">
          <HeroGraphic />
          {QUICK_LINKS.map((link) => (
            <NavLink
              key={link.name}
              to={link.href}
              className={`absolute ${link.position} ${PILL_CLASS}`}
            >
              <link.icon className="size-3.5 text-[var(--kdb-accent-strong)]" strokeWidth={2} />
              {link.name}
            </NavLink>
          ))}
        </div>
      </div>
    </section>
  );
}
