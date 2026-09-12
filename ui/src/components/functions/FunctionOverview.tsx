import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { FunctionStatusBadge } from "@/components/functions/FunctionStatusBadge";
import { RevisionIdDisplay } from "@/components/functions/RevisionIdDisplay";
import { displayLogLevel, formatDurationMs } from "@/features/functions/format";
import {
  formatCount,
  formatEpochMs,
  formatLogTime,
  logLevelClassName,
} from "@/features/functions/display";
import { functionDetailPath } from "@/features/functions/paths";
import { currentRevisionForProcedure } from "@/features/functions/status";
import { moduleRuntimeLabel } from "@/services/functionsService";
import type { ProcedureCatalogSnapshot, ProcedureListItem, ProcedureLogRecord } from "@/features/functions/types";
import { useGetProcedureLogsQuery } from "@/store/apiSlice";

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
              <DetailItem label="Current revision">
                <RevisionIdDisplay revisionId={currentRevision?.revisionId ?? procedure.revisionId} />
              </DetailItem>
              <DetailItem label="Last updated">{formatEpochMs(procedure.lastUpdatedMs)}</DetailItem>
              <DetailItem label="Runtime">{moduleRuntimeLabel(snapshot, procedure.moduleId)}</DetailItem>
              <DetailItem label="Exports">
                <span className="font-mono text-xs">{procedure.id}</span>
              </DetailItem>
            </div>
            <div className="grid gap-4">
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

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
          <CardTitle className="text-sm">Recent activity</CardTitle>
          <Link
            to={functionDetailPath(procedure.id, "logs")}
            className="text-xs text-muted-foreground hover:text-foreground"
          >
            View all logs →
          </Link>
        </CardHeader>
        <CardContent>
          <RecentActivityTable logs={recentLogs} />
        </CardContent>
      </Card>
    </div>
  );
}

function RecentActivityTable({ logs }: { logs: ProcedureLogRecord[] }) {
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
            <TableHead>Message</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {logs.map((log) => (
            <TableRow key={`${log.timestamp}-${log.executionId}-${log.message}`}>
              <TableCell className="whitespace-nowrap text-muted-foreground">{formatLogTime(log.timestamp)}</TableCell>
              <TableCell className={logLevelClassName(log.level)}>{displayLogLevel(log.level)}</TableCell>
              <TableCell className="max-w-xl truncate">{log.message || "—"}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
