import { HardDrive } from "lucide-react";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { Storage, StorageHealthResult } from "@/services/storageService";
import { ACCENT, ICON_CHIP, PANEL, chartTooltipStyle, enterAnimation } from "./theme";

interface StorageUsageChartProps {
  storages: Storage[];
  selectedStorageId: string;
  onStorageChange: (storageId: string) => void;
  health: StorageHealthResult | null;
  isLoading?: boolean;
  error?: string | null;
  className?: string;
  index?: number;
}

function formatBytes(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "-";
  }

  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let size = value;
  let unitIndex = 0;

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }

  const precision = size >= 100 ? 0 : size >= 10 ? 1 : 2;
  return `${size.toFixed(precision)} ${units[unitIndex]}`;
}

function getUsageSlices(health: StorageHealthResult | null) {
  const used = Math.max(0, health?.used_bytes ?? 0);
  const total = Math.max(0, health?.total_bytes ?? 0);
  const free = Math.max(total - used, 0);

  return {
    used,
    total,
    free,
    data: [
      { name: "Used", value: used, color: ACCENT },
      { name: "Free", value: free, color: "var(--kdb-track)" },
    ],
  };
}

export function StorageUsageChart({
  storages,
  selectedStorageId,
  onStorageChange,
  health,
  isLoading,
  error,
  className,
  index = 0,
}: StorageUsageChartProps) {
  const selectedStorage = storages.find((storage) => storage.storage_id === selectedStorageId) ?? null;
  const { used, free, total, data } = getUsageSlices(health);
  const usagePercent = total > 0 ? Math.min(100, Math.round((used / total) * 100)) : null;
  const entrance = enterAnimation(index);

  return (
    <Card
      className={cn(PANEL, "gap-0 px-5 py-4", entrance.className, className)}
      style={entrance.style}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className={ICON_CHIP}>
            <HardDrive className="size-4" strokeWidth={2} />
          </span>
          <div className="min-w-0">
            <p className="text-sm font-semibold">Storage Usage</p>
            <p className="truncate text-xs text-muted-foreground">
              {selectedStorage?.base_directory ?? "Selected storage backend"}
            </p>
          </div>
        </div>
        <Select value={selectedStorageId} onValueChange={onStorageChange} disabled={storages.length === 0}>
          <SelectTrigger className="h-8 w-[150px] shrink-0 text-xs">
            <SelectValue placeholder="Select storage" />
          </SelectTrigger>
          <SelectContent>
            {storages.map((storage) => (
              <SelectItem key={storage.storage_id} value={storage.storage_id}>
                {storage.storage_name} ({storage.storage_id})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {isLoading ? (
        <div className="mt-4 flex items-center gap-5">
          <Skeleton className="size-[132px] rounded-full" />
          <div className="flex-1 space-y-2">
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-4 w-1/2" />
          </div>
        </div>
      ) : error ? (
        <div className="mt-4 rounded-lg border border-destructive/25 bg-destructive/5 p-3 text-xs text-destructive">
          {error}
        </div>
      ) : !health || total <= 0 ? (
        <div className="mt-4 flex h-[150px] flex-col items-center justify-center gap-1 rounded-lg border border-dashed text-center">
          <p className="text-sm font-semibold">No capacity data</p>
          <p className="max-w-[32ch] text-xs text-muted-foreground">
            This backend has not reported total or used bytes yet.
          </p>
        </div>
      ) : (
        <div className="mt-3 flex items-center gap-5">
          <div className="relative size-[140px] shrink-0">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={data}
                  dataKey="value"
                  nameKey="name"
                  innerRadius={48}
                  outerRadius={68}
                  paddingAngle={2}
                  strokeWidth={0}
                  startAngle={90}
                  endAngle={-270}
                >
                  {data.map((entry) => (
                    <Cell key={entry.name} fill={entry.color} />
                  ))}
                </Pie>
                <Tooltip
                  contentStyle={chartTooltipStyle}
                  formatter={(value) => formatBytes(Number(value ?? 0))}
                />
              </PieChart>
            </ResponsiveContainer>
            <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
              <span className="text-2xl font-semibold leading-none tabular-nums">
                {usagePercent === null ? "-" : `${usagePercent}%`}
              </span>
              <span className="mt-1 text-[11px] text-muted-foreground">Used</span>
            </div>
          </div>

          <dl className="min-w-0 flex-1 space-y-2 text-xs">
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-2 text-muted-foreground">
                <span className="size-2.5 rounded-[3px] bg-[var(--kdb-accent)]" />
                Used
              </dt>
              <dd className="font-semibold tabular-nums">{formatBytes(used)}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-2 text-muted-foreground">
                <span className="size-2.5 rounded-[3px] bg-[var(--kdb-track)]" />
                Free
              </dt>
              <dd className="font-semibold tabular-nums">{formatBytes(free)}</dd>
            </div>
            <div className="flex items-center justify-between gap-3 border-t pt-2">
              <dt className="text-muted-foreground">Total</dt>
              <dd className="font-semibold tabular-nums">{formatBytes(total)}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="text-muted-foreground">Status</dt>
              <dd className="font-semibold capitalize">{health.status}</dd>
            </div>
          </dl>
        </div>
      )}
    </Card>
  );
}
