import { useEffect, useState } from "react";
import { NavLink, useLocation } from "react-router-dom";
import {
  BarChart3,
  Braces,
  Calendar,
  CodeXml,
  Database,
  Home,
  PanelLeftClose,
  PanelLeftOpen,
  RadioTower,
  Settings,
  Share2,
  User,
  Wifi,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useBackendStatus } from "@/lib/backend-status";
import { useGetStatsQuery } from "@/store/apiSlice";

const SIDEBAR_COLLAPSED_STORAGE_KEY = "kalamdb-admin-sidebar-collapsed";
const logoUrl = `${import.meta.env.BASE_URL}favicon.png`;

interface NavItem {
  name: string;
  href: string;
  icon: typeof Home;
  activePrefix?: string;
  matchExact?: boolean;
}

const navigation: NavItem[] = [
  { name: "Dashboard", href: "/dashboard", icon: Home },
  { name: "SQL Studio", href: "/sql", icon: CodeXml },
  { name: "Tables & Schemas", href: "/sql", icon: Database },
  { name: "Functions", href: "/functions", icon: Braces },
  { name: "Streaming", href: "/streaming/topics", icon: RadioTower, activePrefix: "/streaming" },
  { name: "Pub/Sub", href: "/streaming/offsets", icon: Share2, matchExact: true },
  { name: "Live Queries", href: "/live-queries", icon: Wifi },
  { name: "Users", href: "/users", icon: User },
  { name: "Schedules", href: "/schedules", icon: Calendar },
  { name: "Logs & Analytics", href: "/logging", icon: BarChart3 },
];

function matches(item: NavItem, pathname: string): boolean {
  return item.matchExact
    ? pathname === item.href
    : pathname.startsWith(item.activePrefix ?? item.href);
}

/** Only the first matching entry highlights, so routes shared by two items stay unambiguous. */
function activeIndex(pathname: string): number {
  const exact = navigation.findIndex((item) => item.matchExact && matches(item, pathname));
  return exact >= 0 ? exact : navigation.findIndex((item) => matches(item, pathname));
}

function itemClass(collapsed: boolean, isActive: boolean): string {
  return cn(
    "mb-1.5 flex items-center rounded-xl text-[15px] transition-colors",
    collapsed ? "h-11 w-11 justify-center" : "h-11 w-full justify-start gap-3 px-3.5",
    isActive
      ? "bg-[var(--kdb-nav-active)] font-semibold text-[var(--kdb-nav-active-fg)]"
      : "font-medium text-muted-foreground hover:bg-accent/70 hover:text-foreground",
  );
}

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    const stored = localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY);
    return stored ? stored === "1" : false;
  });
  const location = useLocation();
  const currentIndex = activeIndex(location.pathname);
  const { state } = useBackendStatus();
  const { data: stats } = useGetStatsQuery();

  const isOnline = state === "online";
  const isChecking = state === "checking";
  const statusLabel = isOnline ? "System Healthy" : isChecking ? "Connecting" : "System Offline";
  const version = stats?.server_version;

  useEffect(() => {
    localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  return (
    <TooltipProvider delayDuration={0}>
      <aside
        data-sidebar
        className={cn(
          "flex h-full min-h-0 shrink-0 flex-col border-r bg-[var(--kdb-nav-bg)] transition-[width] duration-200",
          collapsed ? "w-14" : "w-[264px]",
        )}
      >
        <NavLink
          to="/dashboard"
          className={cn(
            "flex h-14 shrink-0 items-center gap-2.5",
            collapsed ? "justify-center px-0" : "px-4",
          )}
        >
          <img src={logoUrl} alt="KalamDB" className="size-8 shrink-0 object-contain" />
          {!collapsed && (
            <>
              <span className="text-[17px] font-semibold tracking-tight">KalamDB</span>
              <span className="rounded-md bg-[var(--kdb-nav-active)] px-1.5 py-0.5 text-[10px] font-semibold text-[var(--kdb-nav-active-fg)]">
                Admin
              </span>
            </>
          )}
        </NavLink>

        <nav className="min-h-0 flex-1 overflow-y-auto px-3 pt-4" aria-label="Primary navigation">
          {navigation.map((item, itemIndex) => {
            const isActive = itemIndex === currentIndex;
            return (
              <Tooltip key={item.name} disableHoverableContent>
                <TooltipTrigger asChild>
                  <NavLink to={item.href} className={itemClass(collapsed, isActive)}>
                    <item.icon className="size-[19px] shrink-0" strokeWidth={2} />
                    {!collapsed && <span className="truncate">{item.name}</span>}
                  </NavLink>
                </TooltipTrigger>
                {collapsed && (
                  <TooltipContent side="right" className="flex items-center gap-4">
                    {item.name}
                  </TooltipContent>
                )}
              </Tooltip>
            );
          })}
        </nav>

        <div className="shrink-0 px-3 pb-3">
          <div
            className={cn(
              "mb-2 flex items-center gap-2 rounded-xl px-3 py-2",
              collapsed && "justify-center px-0",
              isOnline
                ? "bg-[var(--kdb-nav-active)] text-[var(--kdb-nav-active-fg)]"
                : isChecking
                  ? "bg-muted text-muted-foreground"
                  : "bg-destructive/10 text-destructive",
            )}
            title={statusLabel}
          >
            <span
              className={cn(
                "size-2 shrink-0 rounded-full",
                isOnline && "bg-current motion-safe:animate-pulse",
                !isOnline && "bg-current",
              )}
            />
            {!collapsed && (
              <>
                <span className="truncate text-xs font-semibold">{statusLabel}</span>
                {version && (
                  <span className="ml-auto rounded-md bg-background/70 px-1.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
                    {version.startsWith("v") ? version : `v${version}`}
                  </span>
                )}
              </>
            )}
          </div>

          <div className="mx-auto mb-2 h-px w-full bg-border" />

          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <NavLink
                to="/settings"
                className={itemClass(collapsed, location.pathname.startsWith("/settings"))}
              >
                <Settings className="size-[19px] shrink-0" strokeWidth={2} />
                {!collapsed && <span className="truncate">Settings</span>}
              </NavLink>
            </TooltipTrigger>
            {collapsed && (
              <TooltipContent side="right" className="flex items-center gap-4">
                Settings
              </TooltipContent>
            )}
          </Tooltip>

          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size={collapsed ? "icon-lg" : "lg"}
                className={cn(
                  "w-full rounded-xl text-muted-foreground hover:text-foreground",
                  collapsed ? "justify-center" : "justify-start",
                )}
                aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
                onClick={() => setCollapsed((prev) => !prev)}
              >
                {collapsed ? <PanelLeftOpen /> : <PanelLeftClose data-icon="inline-start" />}
                {!collapsed && <span>Collapse</span>}
              </Button>
            </TooltipTrigger>
            {collapsed && (
              <TooltipContent side="right" className="flex items-center gap-4">
                Expand
              </TooltipContent>
            )}
          </Tooltip>
        </div>
      </aside>
    </TooltipProvider>
  );
}
