import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { CodeBlock } from "@/components/ui/code-block";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { FunctionStatusBadge } from "@/components/functions/FunctionStatusBadge";
import { RevisionIdDisplay } from "@/components/functions/RevisionIdDisplay";
import {
  displayImplementation,
  displayLanguage,
  displayLogActor,
  displayLogLevel,
  displayLogOrigin,
  displaySecurityPolicy,
  formatBytes,
  formatDurationMs,
} from "@/features/functions/format";
import {
  formatCount,
  formatEpochMs,
  formatLogTime,
  logLevelClassName,
} from "@/features/functions/display";
import { functionDetailPath } from "@/features/functions/paths";
import { instancesForProcedure, summarizeModuleInstances, utf8ByteLength } from "@/features/functions/runtime";
import { currentRevisionForProcedure } from "@/features/functions/status";
import { moduleRuntimeLabel } from "@/services/functionsService";
import type { ProcedureCatalogSnapshot, ProcedureListItem, SystemProcedureLogRow } from "@/features/functions/types";
import { useGetModuleInstancesQuery, useGetProcedureLogsQuery } from "@/store/apiSlice";

function DetailItem({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-1">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="text-sm">{children}</dd>
    </div>
  );
}

export function FunctionOverview({
  procedure,
  snapshot,
}: {
  procedure: ProcedureListItem;
  snapshot: ProcedureCatalogSnapshot;
}) {
  const currentRevision = currentRevisionForProcedure(procedure, snapshot.revisions);
  const module = snapshot.modules.find((item) => item.module_id === procedure.moduleId);
  const revisionId =
    currentRevision?.revision_id ?? procedure.revisionId ?? module?.current_revision_id ?? null;
  const lastUpdated = currentRevision?.created_at ?? procedure.lastUpdatedMs;
  const { data: procedureLogs = [] } = useGetProcedureLogsQuery({
    procedureId: procedure.id,
    limit: 10,
  });
  const recentLogs = procedureLogs.slice(0, 8);

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Function details</CardTitle>
        </CardHeader>
        <CardContent>
          <dl className="grid gap-6 sm:grid-cols-2">
            <div className="grid gap-4">
              <DetailItem label="Status">
                <FunctionStatusBadge status={procedure.status} />
              </DetailItem>
              <DetailItem label="Implementation">
                <Badge variant="outline" className="font-normal">
                  {displayImplementation(procedure.implementation)}
                </Badge>
              </DetailItem>
              <DetailItem label="Security policy">
                <Badge variant="outline" className="font-normal">
                  {displaySecurityPolicy(procedure.security)}
                </Badge>
              </DetailItem>
              <DetailItem label="Signature">
                <span className="font-mono text-xs break-all">
                  {procedure.signature || "()"}
                </span>
              </DetailItem>
              <DetailItem label="Return type">
                <span className="font-mono text-xs">{procedure.returnTypeName}</span>
              </DetailItem>
              <DetailItem label="Current revision">
                <RevisionIdDisplay revisionId={revisionId} />
              </DetailItem>
            </div>
            <div className="grid gap-4">
              <DetailItem label="Last updated">{formatEpochMs(lastUpdated)}</DetailItem>
              <DetailItem label="Language">{displayLanguage(procedure.language)}</DetailItem>
              <DetailItem label="Runtime">{moduleRuntimeLabel(snapshot, procedure.moduleId)}</DetailItem>
              <DetailItem label="Calls (24h)">{formatCount(procedure.calls24h)}</DetailItem>
              <DetailItem label="Errors (24h)">{formatCount(procedure.errors24h)}</DetailItem>
              <DetailItem label="Average duration">{formatDurationMs(procedure.averageDurationMs)}</DetailItem>
              <DetailItem label="Description">
                <span className="text-muted-foreground">
                  {procedure.comment || "No description in the catalog."}
                </span>
              </DetailItem>
            </div>
          </dl>
        </CardContent>
      </Card>

      <RuntimeMemoryCard procedure={procedure} />

      {procedure.implementation === "inline" && procedure.source ? (
        <InlineSourceCard language={procedure.language} source={procedure.source} />
      ) : null}

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Recent activity</CardTitle>
          <CardAction>
            <Link
              to={functionDetailPath(procedure.id, "logs")}
              className="text-xs text-muted-foreground hover:text-foreground"
            >
              View all logs →
            </Link>
          </CardAction>
        </CardHeader>
        <CardContent>
          <RecentActivityTable logs={recentLogs} />
        </CardContent>
      </Card>
    </div>
  );
}

function RuntimeMemoryCard({ procedure }: { procedure: ProcedureListItem }) {
  const { data: instances = [], isFetching, error } = useGetModuleInstancesQuery(undefined, {
    pollingInterval: 5_000,
  });
  const matched = instancesForProcedure(instances, procedure);
  const summary = summarizeModuleInstances(matched);

  return (
    <Card data-testid="function-runtime-memory">
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Runtime memory</CardTitle>
        <CardDescription>
          Resident V8 isolate for this revision. Peak is the high-water heap while the isolate stays warm.
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
          <dl className="grid gap-4 sm:grid-cols-3">
            <DetailItem label="Isolates">{formatCount(summary.isolateCount)}</DetailItem>
            <DetailItem label="State">{summary.states.join(", ") || "—"}</DetailItem>
            <DetailItem label="Isolate calls">{formatCount(summary.invocations)}</DetailItem>
            <DetailItem label="Used heap">{formatBytes(summary.usedHeapBytes)}</DetailItem>
            <DetailItem label="Peak heap">{formatBytes(summary.peakHeapBytes)}</DetailItem>
            <DetailItem label="Reserved">{formatBytes(summary.reservedBytes)}</DetailItem>
          </dl>
        )}
      </CardContent>
    </Card>
  );
}

function InlineSourceCard({ language, source }: { language: string | null; source: string }) {
  return (
    <Card data-testid="function-inline-source">
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Inline source</CardTitle>
        <CardDescription>
          {displayLanguage(language)} · {formatBytes(utf8ByteLength(source))}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <CodeBlock value={source} jsonPreferred={false} maxHeightClassName="max-h-96" />
      </CardContent>
    </Card>
  );
}

function RecentActivityTable({ logs }: { logs: SystemProcedureLogRow[] }) {
  if (logs.length === 0) {
    return <p className="py-6 text-center text-sm text-muted-foreground">No recent procedure logs.</p>;
  }

  return (
    <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Time</TableHead>
            <TableHead>Level</TableHead>
            <TableHead>Actor</TableHead>
            <TableHead>Origin</TableHead>
            <TableHead>Message</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {logs.map((log) => (
            <TableRow key={`${log.timestamp}-${log.execution_id}-${log.message}`}>
              <TableCell className="whitespace-nowrap text-muted-foreground">{formatLogTime(log.timestamp)}</TableCell>
              <TableCell className={logLevelClassName(log.level)}>{displayLogLevel(log.level)}</TableCell>
              <TableCell className="max-w-[8rem] truncate">{displayLogActor(log.actor)}</TableCell>
              <TableCell className="text-muted-foreground">{displayLogOrigin(log.origin)}</TableCell>
              <TableCell className="max-w-xl truncate">{log.message || "—"}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
