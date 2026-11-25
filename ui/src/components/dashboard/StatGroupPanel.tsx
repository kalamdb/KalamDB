import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { PANEL, enterAnimation } from "./theme";

export interface StatGroupItem {
  label: string;
  value: string;
  exactValue?: string;
}

interface StatGroupPanelProps {
  title: string;
  icon: LucideIcon;
  items: StatGroupItem[];
  action?: ReactNode;
  isLoading?: boolean;
  className?: string;
  index?: number;
}

function columnsClass(count: number): string {
  if (count <= 2) return "grid-cols-2";
  if (count === 3) return "grid-cols-3";
  return "grid-cols-2 sm:grid-cols-4";
}

export function StatGroupPanel({
  title,
  icon: Icon,
  items,
  action,
  isLoading,
  className,
  index = 0,
}: StatGroupPanelProps) {
  const entrance = enterAnimation(index);

  return (
    <Card
      className={cn(PANEL, "gap-0 px-5 py-4", entrance.className, className)}
      style={entrance.style}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]">
            <Icon className="size-4" strokeWidth={2} />
          </span>
          <p className="text-sm font-medium">{title}</p>
        </div>
        {action}
      </div>

      {isLoading ? (
        <div className={cn("mt-4 grid gap-3", columnsClass(items.length))}>
          {items.map((item) => (
            <Skeleton key={item.label} className="h-12" />
          ))}
        </div>
      ) : (
        <TooltipProvider delayDuration={200}>
          <div className={cn("mt-4 grid divide-x divide-border/70", columnsClass(items.length))}>
            {items.map((item) => (
              <div key={item.label} className="min-w-0 px-3 text-center first:pl-0 last:pr-0">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <p className="truncate text-xl font-semibold leading-none tabular-nums">
                      {item.value}
                    </p>
                  </TooltipTrigger>
                  {item.exactValue ? <TooltipContent>{item.exactValue}</TooltipContent> : null}
                </Tooltip>
                <p className="mt-1.5 truncate text-[11px] text-muted-foreground">{item.label}</p>
              </div>
            ))}
          </div>
        </TooltipProvider>
      )}
    </Card>
  );
}
