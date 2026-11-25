import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { Sparkline } from "./Sparkline";
import { PANEL, enterAnimation } from "./theme";

interface StatTileProps {
  label: string;
  value: string;
  caption: string;
  icon: LucideIcon;
  series?: number[];
  progress?: { percent: number; label: string };
  isLoading?: boolean;
  index?: number;
}

export function StatTile({
  label,
  value,
  caption,
  icon: Icon,
  series,
  progress,
  isLoading,
  index = 0,
}: StatTileProps) {
  const entrance = enterAnimation(index);
  const hasSparkline = !progress && (series?.length ?? 0) > 1;

  return (
    <Card
      className={cn(PANEL, "gap-0 px-4 py-4", entrance.className)}
      style={entrance.style}
    >
      <div className="flex items-center gap-2">
        <span className="flex size-7 items-center justify-center rounded-lg bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]">
          <Icon className="size-4" strokeWidth={2} />
        </span>
        <span className="truncate text-sm font-medium text-foreground">{label}</span>
      </div>

      {isLoading ? (
        <div className="mt-4 space-y-2">
          <Skeleton className="h-7 w-24" />
          <Skeleton className="h-3 w-32" />
        </div>
      ) : (
        <>
          <div className="mt-3 flex items-end justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate text-2xl font-semibold leading-none tracking-tight tabular-nums">
                {value}
              </p>
              <p className="mt-1.5 truncate text-xs text-muted-foreground">{caption}</p>
            </div>
            {hasSparkline && (
              <Sparkline values={series ?? []} className="w-24 shrink-0" height={34} />
            )}
          </div>

          {progress && (
            <div className="mt-3 flex items-center gap-3">
              <Progress
                value={progress.percent}
                className="h-1.5 [&_[data-slot=progress-indicator]]:bg-[var(--kdb-accent)]"
              />
              <span className="shrink-0 text-xs font-medium tabular-nums text-muted-foreground">
                {progress.label}
              </span>
            </div>
          )}
        </>
      )}
    </Card>
  );
}
