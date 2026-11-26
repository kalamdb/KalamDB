import { Activity, CheckCircle2, Crown, Network, TriangleAlert } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardAction, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { ClusterHealth, ClusterNode } from "@/services/clusterService";
import { ICON_CHIP, NUMERIC, PANEL, enterAnimation } from "./theme";

interface DashboardClusterOverviewProps {
  health: ClusterHealth | null;
  nodes: ClusterNode[];
  isLoading?: boolean;
  error?: string | null;
  className?: string;
  index?: number;
}

const MAX_VISIBLE_NODES = 4;

function formatLabel(value: string): string {
  return value
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1).toLowerCase())
    .join(" ");
}

function statusToneClass(status: string): string {
  switch (status.toLowerCase()) {
    case "active":
      return "bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]";
    case "offline":
      return "bg-destructive/10 text-destructive";
    default:
      return "bg-muted text-muted-foreground";
  }
}

export function DashboardClusterOverview({
  health,
  nodes,
  isLoading,
  error,
  className,
  index = 0,
}: DashboardClusterOverviewProps) {
  const visibleNodes = nodes.slice(0, MAX_VISIBLE_NODES);
  const entrance = enterAnimation(index);
  const isHealthy = health?.healthy ?? false;

  return (
    <Card
      className={cn(PANEL, "gap-0 py-4", entrance.className, className)}
      style={entrance.style}
    >
      <CardHeader className="px-5">
        <CardTitle className="flex items-center gap-2 text-sm font-semibold">
          <span className={ICON_CHIP}>
            <Network className="size-4" strokeWidth={2} />
          </span>
          Cluster Nodes &amp; Health
        </CardTitle>
        {health && (
          <CardAction>
            <Badge variant="outline" className="shrink-0 text-[11px]">
              {health.totalNodes} {health.totalNodes === 1 ? "node" : "nodes"}
            </Badge>
          </CardAction>
        )}
      </CardHeader>

      <CardContent className="px-5 pt-4">

      {error ? (
        <div className="rounded-lg border border-destructive/25 bg-destructive/5 p-3 text-xs text-destructive">
          {error}
        </div>
      ) : isLoading && nodes.length === 0 ? (
        <div className="space-y-3">
          <Skeleton className="h-14" />
          <Skeleton className="h-12" />
        </div>
      ) : !health ? (
        <div className="flex h-[150px] flex-col items-center justify-center gap-1 rounded-lg border border-dashed text-center">
          <p className="text-sm font-semibold">No cluster data</p>
          <p className="max-w-[32ch] text-xs text-muted-foreground">
            Node state appears once the server reports a cluster snapshot.
          </p>
        </div>
      ) : (
        <div className="space-y-4">
          <div
            className={cn(
              "flex items-center gap-3 rounded-lg px-3 py-2.5",
              isHealthy
                ? "bg-[var(--kdb-accent-soft)] text-[var(--kdb-accent-strong)]"
                : "bg-destructive/10 text-destructive",
            )}
          >
            {isHealthy ? (
              <CheckCircle2 className="size-5 shrink-0" strokeWidth={2} />
            ) : (
              <TriangleAlert className="size-5 shrink-0" strokeWidth={2} />
            )}
            <div className="min-w-0">
              <p className="text-sm font-semibold">{isHealthy ? "Healthy" : "Degraded"}</p>
              <p className="truncate text-xs opacity-80">
                {health.offlineNodes > 0
                  ? `${health.offlineNodes} offline`
                  : "All nodes responding"}
              </p>
            </div>
          </div>

          <div className="grid grid-cols-3 divide-x divide-border/70">
            <div className="px-2 text-center first:pl-0">
              <p className={cn("text-xl font-semibold leading-none", NUMERIC)}>{health.activeNodes}</p>
              <p className="mt-1.5 text-[11px] text-muted-foreground">Active Node</p>
            </div>
            <div className="px-2 text-center">
              <p className={cn("text-xl font-semibold leading-none", NUMERIC)}>{health.leaderNodes}</p>
              <p className="mt-1.5 text-[11px] text-muted-foreground">Leader</p>
            </div>
            <div className="px-2 text-center last:pr-0">
              <p className={cn("text-xl font-semibold leading-none", NUMERIC)}>{health.followerNodes}</p>
              <p className="mt-1.5 text-[11px] text-muted-foreground">Followers</p>
            </div>
          </div>

          <div className="space-y-1.5">
            {visibleNodes.map((node) => (
              <div
                key={node.node_id}
                className="flex items-center justify-between gap-3 rounded-lg px-2.5 py-2 ring-1 ring-foreground/5"
              >
                <div className="flex min-w-0 items-center gap-2">
                  {node.is_leader ? (
                    <Crown className="size-3.5 shrink-0 text-[var(--kdb-accent-strong)]" strokeWidth={2} />
                  ) : (
                    <Activity className="size-3.5 shrink-0 text-muted-foreground" strokeWidth={2} />
                  )}
                  <span className="text-xs font-medium">Node {node.node_id}</span>
                  <span className="truncate text-[11px] text-muted-foreground">
                    {node.api_addr || node.rpc_addr}
                  </span>
                </div>
                <span
                  className={cn(
                    "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium",
                    statusToneClass(node.status),
                  )}
                >
                  {formatLabel(node.status)}
                </span>
              </div>
            ))}

            {nodes.length > MAX_VISIBLE_NODES && (
              <p className="px-2.5 pt-1 text-[11px] text-muted-foreground">
                Showing {MAX_VISIBLE_NODES} of {nodes.length} nodes.
              </p>
            )}
          </div>
        </div>
      )}
      </CardContent>
    </Card>
  );
}
