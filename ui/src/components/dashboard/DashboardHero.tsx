import type { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import {
  BadgeCheck,
  Braces,
  Database,
  LayoutGrid,
  Radio,
  Share2,
  SquareTerminal,
  Table2,
  Users,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { HeroGraphic } from "./HeroGraphic";
import { HeroMetricStrip, type HeroMetric } from "./HeroMetricStrip";

interface DashboardHeroProps {
  username: string;
  serverVersion?: string;
  actions?: ReactNode;
  metrics: HeroMetric[];
  isMetricsLoading?: boolean;
}

interface QuickLink {
  name: string;
  href: string;
  icon: typeof SquareTerminal;
  side: "left" | "right";
  /** Share of the graphic's height, so pills track the connectors as the art scales. */
  top: string;
}

const QUICK_LINKS: QuickLink[] = [
  { name: "SQL", href: "/sql", icon: SquareTerminal, side: "left", top: "61.5%" },
  { name: "Live subscriptions", href: "/live-queries", icon: Radio, side: "right", top: "22.4%" },
  { name: "Table changes", href: "/streaming/topics", icon: Share2, side: "right", top: "61.5%" },
  { name: "Save & replay", href: "/functions", icon: Braces, side: "right", top: "78%" },
];

const CAPABILITIES = [
  { label: "PostgreSQL Compatible", icon: Database },
  { label: "Realtime (Pub/Sub)", icon: Radio },
  { label: "Per-tenant USER tables", icon: Users },
];

const CHIP_CLASS =
  "inline-flex items-center gap-2 rounded-full border px-3.5 py-2 text-sm font-medium text-foreground";

const PILL_CLASS =
  "absolute flex -translate-y-1/2 items-center gap-1.5 whitespace-nowrap rounded-full bg-[var(--kdb-accent-soft)] px-3 py-1.5 text-[11px] font-semibold text-[var(--kdb-accent-strong)] shadow-[0_6px_16px_-10px_rgb(0_0_0_/_0.35)] ring-1 ring-[var(--kdb-accent-line)] transition-[scale] duration-200 hover:scale-[1.04] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--kdb-accent)] active:scale-[0.98]";

const CTA_BASE =
  "inline-flex items-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition-[scale,background-color] duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--kdb-accent)] focus-visible:ring-offset-2 active:scale-[0.98]";

export function DashboardHero({
  username,
  serverVersion,
  actions,
  metrics,
  isMetricsLoading,
}: DashboardHeroProps) {
  const version = serverVersion
    ? serverVersion.startsWith("v")
      ? serverVersion
      : `v${serverVersion}`
    : null;

  return (
    <Card className="kdb-panel gap-0 overflow-hidden py-0">
      <div className="relative grid lg:grid-cols-[minmax(0,1.02fr)_minmax(0,0.98fr)]">
        <div className="kdb-hero-wash pointer-events-none absolute inset-0" aria-hidden />
        <div className="kdb-hero-grid pointer-events-none absolute inset-0" aria-hidden />

        <div className="relative px-5 pb-6 pt-6 lg:px-8 lg:pb-8">
          <p className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.18em] text-[var(--kdb-accent-strong)]">
            <LayoutGrid className="size-4" strokeWidth={2} />
            Dashboard
          </p>

          <h1 className="mt-4 text-[2.2rem] font-semibold leading-[1.05] tracking-tighter text-foreground lg:text-[2.75rem]">
            KalamDB for <span className="text-[var(--kdb-accent-strong)]">AI Agents</span>
          </h1>

          <p className="mt-3 max-w-[52ch] text-[15px] text-muted-foreground">
            Manage, monitor, and operate your KalamDB instance in real time.
          </p>
          <p className="mt-1 text-sm text-muted-foreground">Welcome back, {username}</p>

          <div className="mt-5 flex flex-wrap items-center gap-2.5">
            {version && (
              <span
                className={cn(
                  CHIP_CLASS,
                  "border-transparent bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]",
                )}
              >
                <BadgeCheck className="size-4" strokeWidth={2} />
                {version}
              </span>
            )}
            {CAPABILITIES.map((capability) => (
              <span key={capability.label} className={CHIP_CLASS}>
                <capability.icon
                  className="size-4 text-[var(--kdb-accent-strong)]"
                  strokeWidth={2}
                />
                {capability.label}
              </span>
            ))}
          </div>

          <div className="mt-6 flex flex-wrap items-center gap-3">
            <NavLink
              to="/sql"
              className={cn(
                CTA_BASE,
                "bg-[var(--kdb-accent-strong)] text-white hover:bg-[var(--kdb-accent)]",
              )}
            >
              <SquareTerminal className="size-4" strokeWidth={2} />
              SQL Studio
            </NavLink>
            <NavLink to="/sql" className={cn(CTA_BASE, "border bg-[var(--kdb-surface)] hover:bg-accent/60")}>
              <Table2 className="size-4" strokeWidth={2} />
              Browse Tables
            </NavLink>
          </div>

          <div className="mt-6 flex flex-wrap gap-2 lg:hidden">
            {QUICK_LINKS.map((link) => (
              <NavLink
                key={link.name}
                to={link.href}
                className="inline-flex items-center gap-1.5 rounded-full bg-[var(--kdb-accent-soft)] px-3 py-1.5 text-[11px] font-semibold text-[var(--kdb-accent-strong)] ring-1 ring-[var(--kdb-accent-line)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--kdb-accent)]"
              >
                <link.icon className="size-3.5 shrink-0" strokeWidth={2} />
                {link.name}
              </NavLink>
            ))}
          </div>
        </div>

        <div className="relative flex flex-col border-t px-5 pb-6 pt-5 lg:border-l lg:border-t-0 lg:px-6 lg:pb-8">
          {actions && <div className="flex flex-wrap items-center justify-end gap-2">{actions}</div>}

          <div className="relative mx-auto mt-4 hidden aspect-[600/330] w-full max-w-[600px] flex-1 lg:block">
            <HeroGraphic />
            {QUICK_LINKS.map((link) => (
              <NavLink
                key={link.name}
                to={link.href}
                className={PILL_CLASS}
                style={{ [link.side]: 0, top: link.top }}
              >
                <link.icon className="size-3.5 shrink-0" strokeWidth={2} />
                {link.name}
              </NavLink>
            ))}
          </div>
        </div>
      </div>

      <HeroMetricStrip metrics={metrics} isLoading={isMetricsLoading} />
    </Card>
  );
}
