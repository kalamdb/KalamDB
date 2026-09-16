import type { ReactNode } from "react";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { Badge } from "@/components/ui/badge";
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatCount } from "@/features/functions/display";
import { formatBytes } from "@/features/functions/format";
import {
  instancesForProcedure,
  memorySlices,
  summarizeModuleInstances,
  utilizationPercent,
} from "@/features/functions/runtime";
import type { ProcedureListItem } from "@/features/functions/types";
import type { SystemModuleInstanceRow } from "@/lib/models";
import { useGetModuleInstancesQuery } from "@/store/apiSlice";

function Stat({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <div className="rounded-lg border border-border/60 bg-muted/20 p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-1 text-lg font-semibold tabular-nums">{children}</p>
      {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
    </div>
  );
}

function StateBadge({ state }: { state: string }) {
  const active = state === "active";
  return (
    <Badge variant="outline" className={active ? "border-emerald-500/40 text-emerald-600" : "font-normal"}>
      <span className={`mr-1.5 inline-block size-1.5 rounded-full ${active ? "bg-emerald-500" : "bg-muted-foreground/60"}`} />
      {active ? "Running" : "Warm"}
    </Badge>
  );
}

function numeric(value: number | string | null | undefined): number {
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function FunctionMemoryCard({ procedure }: { procedure: ProcedureListItem }) {
  const { data: instances = [], isFetching, error } = useGetModuleInstancesQuery(undefined, {
    pollingInterval: 5_000,
  });
  const matched = instancesForProcedure(instances, procedure);
  const summary = summarizeModuleInstances(matched);
  const slices = memorySlices(summary).filter((slice) => slice.value > 0);
  const utilization = utilizationPercent(summary.usedHeapBytes, summary.reservedBytes);
  const running = matched.filter((row) => row.state === "active").length;

  return (
    <Card data-testid="function-runtime-memory">
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Runtime memory</CardTitle>
        <CardDescription>
          Resident V8 isolates for this revision. Reserved is the budget charged against the server
          function-memory limit; peak is the high-water heap while the isolate stayed warm.
        </CardDescription>
        {isFetching ? (
          <CardAction>
            <Spinner />
          </CardAction>
        ) : null}
      </CardHeader>
      <CardContent>
        {error ? (
          <p className="text-sm text-destructive">Failed to load isolate memory.</p>
        ) : matched.length === 0 ? (
          <Empty className="border border-dashed py-8">
            <EmptyHeader>
              <EmptyTitle>No resident isolate</EmptyTitle>
              <EmptyDescription>
                Memory appears after the first invocation while the isolate is still warm.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="flex flex-col gap-6">
            <div className="grid gap-6 lg:grid-cols-[minmax(0,260px)_1fr]">
              <div className="relative h-56">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={slices}
                      dataKey="value"
                      nameKey="name"
                      innerRadius={66}
                      outerRadius={92}
                      paddingAngle={slices.length > 1 ? 2 : 0}
                      strokeWidth={0}
                      startAngle={90}
                      endAngle={-270}
                      isAnimationActive={false}
                    >
                      {slices.map((slice) => (
                        <Cell key={slice.key} fill={slice.color} />
                      ))}
                    </Pie>
                    <Tooltip
                      formatter={(value) => formatBytes(typeof value === "number" ? value : Number(value ?? 0))}
                    />
                  </PieChart>
                </ResponsiveContainer>
                <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
                  <span className="text-2xl font-semibold tabular-nums">
                    {utilization === null ? "—" : `${utilization}%`}
                  </span>
                  <span className="text-xs text-muted-foreground">of reservation used</span>
                </div>
              </div>
              <div className="flex flex-col gap-4">
                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                  <Stat label="Used heap">{formatBytes(summary.usedHeapBytes)}</Stat>
                  <Stat label="Peak heap">{formatBytes(summary.peakHeapBytes)}</Stat>
                  <Stat label="Reserved">{formatBytes(summary.reservedBytes)}</Stat>
                  <Stat
                    label="Isolates"
                    hint={`${formatCount(running)} running · ${formatCount(matched.length - running)} warm`}
                  >
                    {formatCount(summary.isolateCount)}
                  </Stat>
                  <Stat label="Isolate calls" hint="since these isolates were created">
                    {formatCount(summary.invocations)}
                  </Stat>
                  <Stat label="Workers">
                    {formatCount(new Set(matched.map((row) => row.worker)).size)}
                  </Stat>
                </div>
                <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  {memorySlices(summary).map((slice) => (
                    <li key={slice.key} className="flex items-center gap-1.5">
                      <span className="inline-block size-2 rounded-sm" style={{ backgroundColor: slice.color }} />
                      {slice.name}
                    </li>
                  ))}
                </ul>
              </div>
            </div>
            {matched.length > 1 ? <IsolateTable rows={matched} /> : null}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function IsolateTable({ rows }: { rows: SystemModuleInstanceRow[] }) {
  const sorted = [...rows].sort((a, b) => numeric(b.used_heap_bytes) - numeric(a.used_heap_bytes));
  return (
    <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Isolate</TableHead>
            <TableHead>Worker</TableHead>
            <TableHead>State</TableHead>
            <TableHead className="text-right">Used</TableHead>
            <TableHead className="text-right">Peak</TableHead>
            <TableHead className="text-right">Reserved</TableHead>
            <TableHead className="text-right">Calls</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {sorted.map((row) => (
            <TableRow key={row.instance_id}>
              <TableCell className="font-mono text-xs">#{row.instance_id}</TableCell>
              <TableCell className="tabular-nums">{row.worker}</TableCell>
              <TableCell>
                <StateBadge state={row.state} />
              </TableCell>
              <TableCell className="text-right tabular-nums">{formatBytes(numeric(row.used_heap_bytes))}</TableCell>
              <TableCell className="text-right tabular-nums">{formatBytes(numeric(row.peak_heap_bytes))}</TableCell>
              <TableCell className="text-right tabular-nums">{formatBytes(numeric(row.reserved_bytes))}</TableCell>
              <TableCell className="text-right tabular-nums">{formatCount(numeric(row.invocations))}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
