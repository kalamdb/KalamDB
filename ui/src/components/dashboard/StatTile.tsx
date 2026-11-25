import { ArrowDownRight, ArrowUpRight, type LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { Sparkline } from "./Sparkline";
import { ICON_CHIP, PANEL, enterAnimation } from "./theme";

interface StatTileProps {
  label: string;
  value: string;
  caption: string;
  icon: LucideIcon;
  series?: number[];
  delta?: number | null;
  deltaGoodWhen?: "up" | "down";
  progress?: { percent: number; label: string };
  detail?: string;
  isLoading?: boolean;
  index?: number;
}

function DeltaBadge({ delta, goodWhen }: { delta: number; goodWhen: "up" | "down" }) {
  const isUp = delta >= 0;
  const isGood = goodWhen === "up" ? isUp : !isUp;
  const Icon = isUp ? ArrowUpRight : ArrowDownRight;

  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-0.5 rounded-full px-2 py-1 text-[11px] font-semibold tabular-nums",
        isGood
          ? "bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]"
          : "bg-destructive/10 text-destructive",
      )}
    >
      <Icon className="size-3" strokeWidth={2.5} />
      {isUp ? "+" : "−"}
      {Math.abs(delta).toFixed(Math.abs(delta) >= 10 ? 0 : 1)}%
    </span>
  );
}

export function StatTile({
  label,
  value,
  caption,
  icon: Icon,
  series,
  delta,
  deltaGoodWhen = "up",
  progress,
  detail,
  isLoading,
  index = 0,
}: StatTileProps) {
  const entrance = enterAnimation(index);
  const hasSparkline = !progress && (series?.length ?? 0) > 1;

  return (
    <Card
      className={cn(PANEL, "gap-0 overflow-hidden px-5 py-4", entrance.className)}
      style={entrance.style}
    >
      <div className="flex items-center gap-2.5">
        <span className={ICON_CHIP}>
          <Icon className="size-4" strokeWidth={2} />
        </span>
        <span className="truncate text-sm font-semibold text-foreground">{label}</span>
      </div>

      {isLoading ? (
        <div className="mt-4 space-y-2">
          <Skeleton className="h-8 w-28" />
          <Skeleton className="h-3 w-32" />
          <Skeleton className="h-9 w-full" />
        </div>
      ) : (
        <>
          <div className="mt-3 flex items-start justify-between gap-2">
            <p className="min-w-0 truncate text-[1.75rem] font-semibold leading-none tracking-tight tabular-nums">
              {value}
            </p>
            {delta !== null && delta !== undefined && (
              <DeltaBadge delta={delta} goodWhen={deltaGoodWhen} />
            )}
          </div>
          <p className="mt-2 truncate text-xs text-muted-foreground">{caption}</p>

          {hasSparkline && (
            <Sparkline values={series ?? []} className="mt-3 w-full" height={44} />
          )}

          {detail && (
            <p className="mt-4 border-t pt-3 text-xs text-muted-foreground">{detail}</p>
          )}

          {progress && (
            <div className="mt-4 flex items-center gap-3">
              <Progress
                value={progress.percent}
                className="h-2 [&_[data-slot=progress-indicator]]:bg-[var(--kdb-accent)]"
              />
              <span className="shrink-0 text-xs font-semibold tabular-nums text-muted-foreground">
                {progress.label}
              </span>
            </div>
          )}
        </>
      )}
    </Card>
  );
}
