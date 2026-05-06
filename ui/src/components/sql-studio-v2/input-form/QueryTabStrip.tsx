import { useEffect, useRef, useState } from "react";
import { FileCode2, Plus, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { QueryTab } from "../shared/types";

interface QueryTabStripProps {
  tabs: QueryTab[];
  activeTabId: string;
  onTabSelect: (tabId: string) => void;
  onAddTab: () => void;
  onCloseTab: (tabId: string) => void;
}

export function QueryTabStrip({
  tabs,
  activeTabId,
  onTabSelect,
  onAddTab,
  onCloseTab,
}: QueryTabStripProps) {
  const previousUnreadCountsRef = useRef<Record<string, number>>({});
  const [flashingTabIds, setFlashingTabIds] = useState<Record<string, boolean>>({});

  useEffect(() => {
    const nextUnreadCounts: Record<string, number> = {};
    const newFlashingTabIds: string[] = [];

    tabs.forEach((tab) => {
      const previousUnreadCount = previousUnreadCountsRef.current[tab.id] ?? 0;
      nextUnreadCounts[tab.id] = tab.unreadChangeCount;
      if (tab.id !== activeTabId && tab.unreadChangeCount > previousUnreadCount) {
        newFlashingTabIds.push(tab.id);
      }
    });

    previousUnreadCountsRef.current = nextUnreadCounts;

    if (newFlashingTabIds.length === 0) {
      return;
    }

    setFlashingTabIds((current) => {
      const next = { ...current };
      newFlashingTabIds.forEach((tabId) => {
        next[tabId] = true;
      });
      return next;
    });

    const timeoutIds = newFlashingTabIds.map((tabId) => window.setTimeout(() => {
      setFlashingTabIds((current) => {
        if (!current[tabId]) {
          return current;
        }
        const next = { ...current };
        delete next[tabId];
        return next;
      });
    }, 900));

    return () => {
      timeoutIds.forEach((timeoutId) => window.clearTimeout(timeoutId));
    };
  }, [activeTabId, tabs]);

  return (
    <div className="flex items-center border-b border-border bg-background">
      <div className="flex min-w-0 flex-1 overflow-x-auto">
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const isFlashing = flashingTabIds[tab.id] === true;
          const isLive = tab.isLive;
          const isLiveConnecting = isLive && tab.liveStatus === "connecting";
          const isLiveConnected = isLive && tab.liveStatus === "connected";
          const isLiveErrored = isLive && tab.liveStatus === "error";
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => onTabSelect(tab.id)}
              className={cn(
                "group relative flex h-11 min-w-[168px] max-w-[280px] items-center gap-2 overflow-hidden border-r border-border px-3 text-sm transition-colors",
                isActive
                  ? "border-t-2 border-t-sky-500 bg-background text-foreground"
                  : "text-muted-foreground hover:bg-muted :bg-accent",
                isLive && "backdrop-blur-sm",
                isLiveConnected && (isActive
                  ? "bg-sky-500/12 shadow-[0_0_24px_rgba(14,165,233,0.16)]"
                  : "bg-sky-500/8 shadow-[0_0_18px_rgba(14,165,233,0.12)]"),
                isLiveConnecting && "bg-sky-500/10 shadow-[0_0_18px_rgba(14,165,233,0.12)] animate-pulse",
                isLiveErrored && "bg-red-500/8 shadow-[0_0_18px_rgba(239,68,68,0.12)]",
                tab.unreadChangeCount > 0 && !isActive && "text-foreground",
                isFlashing && !isActive && "bg-amber-500/10 ring-inset ring-1 ring-amber-500/30 animate-[pulse_0.8s_ease-out_1]",
              )}
            >
              <FileCode2 className={cn("h-3.5 w-3.5 shrink-0", isActive ? "text-primary" : "text-muted-foreground")} />
              <span className="min-w-0 flex-1 truncate text-left">
                {isLive && (
                  <span
                    aria-label={`Live query ${tab.liveStatus}`}
                    className={cn(
                      "mr-2 inline-flex h-2.5 w-2.5 align-middle rounded-full",
                      isLiveErrored && "bg-red-500 shadow-[0_0_10px_rgba(239,68,68,0.32)]",
                      isLiveConnecting && "bg-sky-400/90 ring-2 ring-sky-500/20",
                      isLiveConnected && "bg-sky-500 shadow-[0_0_12px_rgba(14,165,233,0.36)] animate-pulse",
                      !isLiveErrored && !isLiveConnecting && !isLiveConnected && "bg-sky-500/80",
                    )}
                  />
                )}
                <span className="align-middle">{tab.title}</span>
              </span>
              <span className="ml-auto flex shrink-0 items-center gap-1.5">
                {tab.unreadChangeCount > 0 && !isActive && (
                  <span className="inline-flex min-w-5 items-center justify-center rounded-full bg-amber-500 px-1.5 py-0.5 text-[10px] font-semibold leading-none text-amber-950">
                    {tab.unreadChangeCount}
                  </span>
                )}
                {tab.isDirty && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />}
                {tabs.length > 1 && (
                  <span
                    onClick={(event) => {
                      event.stopPropagation();
                      onCloseTab(tab.id);
                    }}
                    className="rounded p-0.5 opacity-0 transition group-hover:opacity-100 hover:bg-border :bg-muted"
                  >
                    <X className="h-3.5 w-3.5" />
                  </span>
                )}
              </span>
            </button>
          );
        })}

        <button
          type="button"
          onClick={onAddTab}
          className="flex h-11 w-11 shrink-0 items-center justify-center border-r border-border text-muted-foreground transition hover:bg-muted hover:text-foreground :bg-accent :text-foreground"
          title="New query tab"
        >
          <Plus className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}
