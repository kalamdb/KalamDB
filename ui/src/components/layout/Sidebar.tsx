import { useEffect, useState } from "react";
import { NavLink, useLocation } from "react-router-dom";
import {
  FileText,
  LayoutDashboard,
  PanelLeftClose,
  PanelLeftOpen,
  RadioTower,
  Settings,
  Terminal,
  Users,
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

const SIDEBAR_COLLAPSED_STORAGE_KEY = "kalamdb-admin-sidebar-collapsed";

const navigation = [
  { name: "Dashboard", href: "/dashboard", icon: LayoutDashboard },
  { name: "SQL Studio", href: "/sql", icon: Terminal },
  { name: "Streaming", href: "/streaming/topics", icon: RadioTower, activePrefix: "/streaming" },
  { name: "Users", href: "/users", icon: Users },
  { name: "Live Queries", href: "/live-queries", icon: Wifi },
  { name: "Logs & Analytics", href: "/logging", icon: FileText },
];

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    const stored = localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY);
    return stored ? stored === "1" : true;
  });
  const location = useLocation();

  useEffect(() => {
    localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  return (
    <TooltipProvider delayDuration={0}>
      <aside
        className={cn(
          "flex h-full min-h-0 shrink-0 flex-col border-r bg-background transition-[width] duration-200",
          collapsed ? "w-12" : "w-52",
        )}
      >
        <nav className="min-h-0 flex-1 overflow-y-auto p-2" aria-label="Primary navigation">
          {navigation.map((item) => {
            const isActive = location.pathname.startsWith(item.activePrefix ?? item.href);
            return (
              <Tooltip key={item.name} disableHoverableContent>
                <TooltipTrigger asChild>
                  <NavLink
                    to={item.href}
                    className={cn(
                      "mb-0.5 flex items-center rounded-md text-sm transition-colors",
                      collapsed ? "h-8 w-8 justify-center" : "h-8 w-full justify-start gap-2 px-2.5",
                      isActive
                        ? "bg-accent text-foreground font-medium"
                        : "text-muted-foreground hover:bg-accent/80 hover:text-accent-foreground"
                    )}
                  >
                    <item.icon className="h-4 w-4 shrink-0" />
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

        <div className="shrink-0 p-2">
          <div className="mx-auto mb-2 h-px w-full bg-border" />
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <NavLink
                to="/settings"
                className={cn(
                  "mb-0.5 flex items-center rounded-md text-sm transition-colors",
                  collapsed ? "h-8 w-8 justify-center" : "h-8 w-full justify-start gap-2 px-2.5",
                  location.pathname.startsWith("/settings")
                    ? "bg-accent text-foreground font-medium"
                    : "text-muted-foreground hover:bg-accent/80 hover:text-accent-foreground"
                )}
              >
                <Settings className="h-4 w-4 shrink-0" />
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
                className={cn(
                  "w-full justify-start text-muted-foreground hover:text-foreground",
                  collapsed ? "h-8 w-8 p-0 justify-center" : "h-8 gap-2 px-2.5 text-sm"
                )}
                aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
                onClick={() => setCollapsed((prev) => !prev)}
              >
                {collapsed ? (
                  <PanelLeftOpen className="h-4 w-4" />
                ) : (
                  <PanelLeftClose className="h-4 w-4 mr-2" />
                )}
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
