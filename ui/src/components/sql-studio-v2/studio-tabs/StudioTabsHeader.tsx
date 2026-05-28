import { Compass, Pencil } from "lucide-react";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import {
  setActiveStudioTab,
  type StudioTab,
} from "@/features/sql-studio/state/sqlStudioUiSlice";
import { selectActiveStudioTab } from "@/features/sql-studio/state/selectors";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { chromeLabelClassName } from "@/components/layout/typography";
import { cn } from "@/lib/utils";

interface TabConfig {
  value: StudioTab;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  disabled?: boolean;
  comingSoon?: boolean;
}

const TABS: TabConfig[] = [
  { value: "explorer", label: "Explorer", icon: Compass },
  { value: "editor", label: "Editor", icon: Pencil },
];

export function StudioTabsHeader() {
  const dispatch = useAppDispatch();
  const activeTab = useAppSelector(selectActiveStudioTab);

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex shrink-0 items-center gap-0.5 border-b border-border bg-background px-1 py-1">
        {TABS.map((tab) => {
          const Icon = tab.icon;
          const isActive = activeTab === tab.value;
          return (
            <Tooltip key={tab.value}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  disabled={tab.disabled}
                  onClick={() => dispatch(setActiveStudioTab(tab.value))}
                  className={cn(
                    "relative flex flex-1 flex-col items-center gap-0.5 rounded-md px-2 py-1.5 transition-colors",
                    chromeLabelClassName,
                    "hover:bg-accent hover:text-accent-foreground",
                    isActive
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground",
                    tab.disabled && "cursor-not-allowed opacity-50",
                  )}
                >
                  <Icon className="h-4 w-4" />
                  <span>{tab.label}</span>
                  {tab.comingSoon && (
                    <span className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-blue-500" />
                  )}
                </button>
              </TooltipTrigger>
              <TooltipContent>
                {tab.comingSoon ? `${tab.label} — coming soon` : tab.label}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </TooltipProvider>
  );
}
