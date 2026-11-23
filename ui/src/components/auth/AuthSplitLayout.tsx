import { type ReactNode } from "react";
import { type LucideIcon, Zap } from "lucide-react";
import { cn } from "cn";

const appFlowUrl = `${import.meta.env.BASE_URL}branding/kalamdb-app-flow.webp`;
const wavesUrl = `${import.meta.env.BASE_URL}branding/kalamdb-auth-waves.webp`;
const leavesUrl = `${import.meta.env.BASE_URL}branding/kalamdb-auth-leaves.webp`;

function KalamDbMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" aria-hidden className={className}>
      <defs>
        <linearGradient id="kalamdb-mark" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#7ee89a" />
          <stop offset="50%" stopColor="#2eb86a" />
          <stop offset="100%" stopColor="#077f4c" />
        </linearGradient>
      </defs>
      <g fill="url(#kalamdb-mark)">
        <path d="M16 3.2 29 9.1a1 1 0 0 1 0 1.8L16 16.8 3 10.9a1 1 0 0 1 0-1.8z" />
        <path d="M4.9 14.4 16 19.4l11.1-5 1.9.9a1 1 0 0 1 0 1.8L16 23 3 17.1a1 1 0 0 1 0-1.8z" />
        <path d="M4.9 20.6 16 25.6l11.1-5 1.9.9a1 1 0 0 1 0 1.8L16 29.2 3 23.3a1 1 0 0 1 0-1.8z" />
      </g>
    </svg>
  );
}

export interface AuthPanelFeature {
  label: string;
  description: string;
  icon: LucideIcon;
  iconClassName?: string;
}

interface AuthSplitLayoutProps {
  title?: string;
  heading?: string;
  headingHighlight?: string;
  description: string;
  children: ReactNode;
  panelTitle?: string;
  panelTitleHighlight?: string;
  panelDescription?: string;
  panelFootnote?: string;
  panelFeatures?: AuthPanelFeature[];
}

function withHighlight(text: string, highlight: string | undefined, className: string) {
  if (!highlight) {
    return text;
  }

  const start = text.indexOf(highlight);
  if (start < 0) {
    return text;
  }

  return (
    <>
      {text.slice(0, start)}
      <span className={className}>{highlight}</span>
      {text.slice(start + highlight.length)}
    </>
  );
}

function renderHeading(heading: string, highlight: string | undefined) {
  if (!highlight) {
    return heading;
  }

  const start = heading.indexOf(highlight);
  if (start < 0) {
    return heading;
  }

  return (
    <>
      {heading.slice(0, start).trimEnd()}
      <span className="relative mt-1 block w-fit text-[#077f4c]">
        {highlight}
        <svg
          aria-hidden
          viewBox="0 0 200 10"
          preserveAspectRatio="none"
          className="absolute -bottom-1.5 left-0 h-2 w-full text-emerald-500/60"
        >
          <path
            d="M3 7.2C52 2.4 148 2.1 197 5.4"
            fill="none"
            stroke="currentColor"
            strokeWidth="3"
            strokeLinecap="round"
          />
        </svg>
      </span>
      {heading.slice(start + highlight.length)}
    </>
  );
}

export default function AuthSplitLayout({
  title = "Welcome back",
  heading,
  headingHighlight,
  description,
  children,
  panelTitle = "KalamDB",
  panelTitleHighlight,
  panelDescription = "Operational control plane for embedded and on-prem deployments.",
  panelFootnote = "Embedded UI v2",
  panelFeatures = [],
}: AuthSplitLayoutProps) {
  const formHeading = heading ?? panelTitle;

  return (
    <div className="relative h-full min-h-0 overflow-auto bg-[#f5f8fb] p-4 md:p-8">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(70%_55%_at_88%_30%,rgba(205,236,246,0.85),transparent_62%),radial-gradient(55%_45%_at_6%_4%,rgba(245,246,255,0.95),transparent_60%),radial-gradient(65%_40%_at_55%_100%,rgba(222,245,238,0.9),transparent_65%)]"
      />
      <img aria-hidden alt="" src={wavesUrl} className="pointer-events-none absolute inset-x-0 bottom-0 w-full" />
      <img
        aria-hidden
        alt=""
        src={leavesUrl}
        className="pointer-events-none absolute bottom-0 left-0 hidden w-14 md:block"
      />

      <div className="relative mx-auto grid min-h-full max-w-7xl items-center gap-8 lg:grid-cols-[minmax(0,460px)_minmax(0,1fr)] lg:gap-12">
        <section className="rounded-3xl border border-black/5 bg-white p-6 shadow-[0_30px_70px_-30px_rgba(6,78,59,0.3)] sm:p-10">
          <div className="flex items-center gap-3">
            <KalamDbMark className="size-8 shrink-0" />
            <span className="text-2xl font-bold tracking-tight text-zinc-900">KalamDB</span>
            <span className="h-6 w-px bg-zinc-200" />
            <span className="rounded-full bg-emerald-100/80 px-3 py-1.5 text-xs font-semibold text-emerald-800">
              Admin UI
            </span>
          </div>

          <div className="mt-9 space-y-3">
            {title && (
              <p className="text-xs font-semibold uppercase tracking-[0.18em] text-emerald-700">{title}</p>
            )}
            <h1 className="text-[2.1rem] font-semibold leading-[1.15] tracking-tight text-zinc-900">
              {renderHeading(formHeading, headingHighlight)}
            </h1>
            <p className="text-sm leading-relaxed text-zinc-500">{description}</p>
          </div>

          <div className="mt-8">{children}</div>
        </section>

        <aside className="relative hidden flex-col items-center lg:flex">
          <span className="absolute right-0 top-0 inline-flex items-center gap-2 rounded-full bg-white px-3 py-1.5 text-xs font-medium text-zinc-700 shadow-sm ring-1 ring-black/5">
            <span className="size-1.5 rounded-full bg-emerald-500" />
            Enterprise Ready
          </span>

          <span className="inline-flex items-center gap-2 rounded-full bg-white px-4 py-2 text-sm font-semibold text-zinc-800 shadow-sm ring-1 ring-black/5">
            <Zap className="size-4 text-emerald-600" />
            {panelFootnote}
          </span>

          <h2 className="mt-6 text-center text-4xl font-semibold tracking-tight text-zinc-900">
            {withHighlight(panelTitle, panelTitleHighlight, "text-[#077f4c]")}
          </h2>
          <p className="mt-3 max-w-2xl text-center text-base leading-relaxed text-zinc-500">{panelDescription}</p>

          <img
            src={appFlowUrl}
            alt="Your app writes with SQL to KalamDB, which serves SQL tables, live queries and durable topics to connected clients and background workers."
            className="mt-6 w-full max-w-3xl"
          />

          {panelFeatures.length > 0 && (
            <ul className="mt-5 grid w-full max-w-3xl grid-cols-2 gap-y-6 rounded-2xl border border-white/70 bg-white/75 px-4 py-5 shadow-[0_20px_45px_-26px_rgba(6,78,59,0.4),inset_0_1px_0_rgba(255,255,255,0.9)] backdrop-blur-md xl:grid-cols-4 xl:divide-x xl:divide-emerald-950/10">
              {panelFeatures.map(({ label, description: featureDescription, icon: Icon, iconClassName }) => (
                <li key={label} className="flex flex-col items-center gap-1.5 px-4 text-center">
                  <Icon className={cn("size-5", iconClassName ?? "text-emerald-600")} />
                  <span className="text-sm font-semibold text-zinc-900">{label}</span>
                  <span className="text-xs leading-relaxed text-zinc-600">{featureDescription}</span>
                </li>
              ))}
            </ul>
          )}
        </aside>
      </div>
    </div>
  );
}
