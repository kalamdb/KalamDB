import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Card, CardAction, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { ICON_CHIP, NUMERIC, PANEL, enterAnimation } from "./theme";

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
      className={cn(PANEL, "gap-0 py-4", entrance.className, className)}
      style={entrance.style}
    >
      <CardHeader className="px-5">
        <CardTitle className="flex items-center gap-2 text-sm font-semibold">
          <span className={ICON_CHIP}>
            <Icon className="size-4" strokeWidth={2} />
          </span>
          {title}
        </CardTitle>
        {action && <CardAction>{action}</CardAction>}
      </CardHeader>

      <CardContent className="px-5 pt-4">
        {isLoading ? (
          <div className={cn("grid gap-3", columnsClass(items.length))}>
            {items.map((item) => (
              <Skeleton key={item.label} className="h-12" />
            ))}
          </div>
        ) : (
          <TooltipProvider delayDuration={200}>
            <div className={cn("grid divide-x divide-border/70", columnsClass(items.length))}>
              {items.map((item) => (
                <div key={item.label} className="min-w-0 px-3 text-center first:pl-0 last:pr-0">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <p className={cn("truncate text-2xl font-semibold leading-none", NUMERIC)}>
                        {item.value}
                      </p>
                    </TooltipTrigger>
                    {item.exactValue ? <TooltipContent>{item.exactValue}</TooltipContent> : null}
                  </Tooltip>
                  <p className="mt-2 truncate text-[11px] text-muted-foreground">{item.label}</p>
                </div>
              ))}
            </div>
          </TooltipProvider>
        )}
      </CardContent>
    </Card>
  );
}
