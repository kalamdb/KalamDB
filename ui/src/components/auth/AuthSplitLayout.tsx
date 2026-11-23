import { type ReactNode } from "react";
import { type LucideIcon, Zap } from "lucide-react";
import { cn } from "cn";

const logoUrl = `${import.meta.env.BASE_URL}branding/kalamdb_logo.png`;
const appFlowUrl = `${import.meta.env.BASE_URL}branding/kalamdb-app-flow.webp`;

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
    <div className="relative h-full min-h-0 overflow-auto bg-[#f4f8f6] p-4 md:p-8">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(90%_60%_at_80%_0%,rgba(209,250,229,0.75),transparent_60%),radial-gradient(70%_50%_at_10%_100%,rgba(167,243,208,0.55),transparent_65%)]"
      />

      <div className="relative mx-auto grid min-h-full max-w-7xl items-center gap-8 lg:grid-cols-[minmax(0,460px)_minmax(0,1fr)] lg:gap-12">
        <section className="rounded-3xl border border-black/5 bg-white p-6 shadow-[0_24px_60px_-28px_rgba(6,78,59,0.28)] sm:p-9">
          <div className="flex items-center gap-3">
            <img src={logoUrl} alt="KalamDB" className="h-8 w-auto object-contain" />
            <span className="rounded-full bg-emerald-50 px-2.5 py-1 text-[11px] font-semibold text-emerald-800 ring-1 ring-emerald-600/15">
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
            <ul className="mt-4 grid w-full max-w-3xl grid-cols-2 gap-y-6 xl:grid-cols-4 xl:divide-x xl:divide-black/5">
              {panelFeatures.map(({ label, description: featureDescription, icon: Icon, iconClassName }) => (
                <li key={label} className="flex flex-col items-center gap-1.5 px-4 text-center">
                  <Icon className={cn("size-5", iconClassName ?? "text-emerald-600")} />
                  <span className="text-sm font-semibold text-zinc-900">{label}</span>
                  <span className="text-xs leading-relaxed text-zinc-500">{featureDescription}</span>
                </li>
              ))}
            </ul>
          )}
        </aside>
      </div>
    </div>
  );
}
