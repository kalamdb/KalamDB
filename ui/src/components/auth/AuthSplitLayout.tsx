import { type ReactNode } from "react";
import { type LucideIcon, Zap } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { cn } from "cn";

/** The KalamDB logo lockup, shared by every auth surface. */
export const kalamDbLogoUrl = `${import.meta.env.BASE_URL}branding/kalamdb_logo.png`;
const appFlowUrl = `${import.meta.env.BASE_URL}branding/kalamdb-app-flow.webp`;
const wavesUrl = `${import.meta.env.BASE_URL}branding/kalamdb-auth-waves.webp`;
const leavesUrl = `${import.meta.env.BASE_URL}branding/kalamdb-auth-leaves.webp`;

/** Sizing shared by the auth inputs; colors stay with the shadcn Input defaults. */
export const authInputClass =
  "h-11 rounded-xl text-sm focus-visible:border-brand focus-visible:ring-brand/20 md:text-sm";

/** Sizing shared by the auth buttons, whatever their variant. */
export const authButtonClass = "h-11 rounded-xl text-sm font-semibold";

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
      <span className="relative mt-1 block w-fit text-brand">
        {highlight}
        <svg
          aria-hidden
          viewBox="0 0 200 10"
          preserveAspectRatio="none"
          className="absolute -bottom-1.5 left-0 h-2 w-full text-brand-accent/60"
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
    // The backdrop artwork is drawn for a light canvas, so the whole auth tree
    // pins the light palette instead of inheriting the root theme.
    <div className="theme-light h-full min-h-0 overflow-auto bg-[#f5f8fb] text-foreground">
      <div className="relative flex min-h-full flex-col justify-center p-4 md:p-8">
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

        <div className="relative mx-auto grid w-full max-w-7xl items-center gap-8 lg:grid-cols-[minmax(0,460px)_minmax(0,1fr)] lg:gap-12">
          <Card className="gap-0 rounded-3xl shadow-[0_30px_70px_-30px] shadow-brand-deep/30 [--card-spacing:--spacing(6)] sm:[--card-spacing:--spacing(10)]">
            <CardHeader className="gap-0">
              <div className="flex items-center gap-3">
                <img src={kalamDbLogoUrl} alt="KalamDB" className="h-8 w-auto" />
                <Separator orientation="vertical" className="h-6" />
                <Badge className="h-6 bg-brand-subtle px-3 text-xs text-brand-subtle-foreground">Admin UI</Badge>
              </div>

              <div className="mt-9 space-y-3">
                {title && (
                  <p className="text-xs font-semibold uppercase tracking-[0.18em] text-brand-subtle-foreground">
                    {title}
                  </p>
                )}
                <h1 className="text-[2.1rem] font-semibold leading-[1.15] tracking-tight text-foreground">
                  {renderHeading(formHeading, headingHighlight)}
                </h1>
                <p className="text-sm leading-relaxed text-muted-foreground">{description}</p>
              </div>
            </CardHeader>

            <CardContent className="mt-8 text-sm">{children}</CardContent>
          </Card>

          <aside className="relative hidden flex-col items-center lg:flex">
            <Badge className="absolute right-0 top-0 h-7 gap-2 bg-card px-3 text-xs font-medium text-foreground shadow-sm ring-1 ring-foreground/10">
              <span className="size-1.5 rounded-full bg-brand-accent" />
              Enterprise Ready
            </Badge>

            <Badge className="h-9 gap-2 bg-card px-4 text-sm font-semibold text-foreground shadow-sm ring-1 ring-foreground/10">
              <Zap className="size-4! text-brand" />
              {panelFootnote}
            </Badge>

            <h2 className="mt-6 text-center text-4xl font-semibold tracking-tight text-foreground">
              {withHighlight(panelTitle, panelTitleHighlight, "text-brand")}
            </h2>
            <p className="mt-3 max-w-2xl text-center text-base leading-relaxed text-muted-foreground">
              {panelDescription}
            </p>

            <img
              src={appFlowUrl}
              alt="Your app writes with SQL to KalamDB, which serves SQL tables, live queries and durable topics to connected clients and background workers."
              className="mt-6 w-full max-w-3xl"
            />

            {panelFeatures.length > 0 && (
              <ul className="mt-5 grid w-full max-w-3xl grid-cols-2 gap-y-6 rounded-2xl bg-card/75 px-4 py-5 shadow-[0_20px_45px_-26px] shadow-brand-deep/40 ring-1 ring-card/70 backdrop-blur-md xl:grid-cols-4 xl:divide-x xl:divide-border">
                {panelFeatures.map(({ label, description: featureDescription, icon: Icon, iconClassName }) => (
                  <li key={label} className="flex flex-col items-center gap-1.5 px-4 text-center">
                    <Icon className={cn("size-5", iconClassName ?? "text-brand")} />
                    <span className="text-sm font-semibold text-foreground">{label}</span>
                    <span className="text-xs leading-relaxed text-muted-foreground">{featureDescription}</span>
                  </li>
                ))}
              </ul>
            )}
          </aside>
        </div>
      </div>
    </div>
  );
}
