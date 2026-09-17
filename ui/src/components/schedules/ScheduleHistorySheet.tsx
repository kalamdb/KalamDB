import { Link } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatDurationMs } from "@/features/functions/format";
import { rtkErrorMessage } from "@/features/functions/display";
import type { SystemProcedureLogRow } from "@/lib/models";
import { formatScheduleTime, scheduleStatus, type ScheduleRow } from "@/services/schedulesService";

function outcomeLabel(outcome: string): string {
  const normalized = outcome.trim().toLowerCase();
  if (normalized === "ok") {
    return "Succeeded";
  }
  if (normalized === "error") {
    return "Failed";
  }
  return outcome || "—";
}

export function ScheduleHistorySheet({
  schedule,
  logs,
  isLoading,
  error,
  onOpenChange,
}: {
  schedule: ScheduleRow | null;
  logs: SystemProcedureLogRow[];
  isLoading: boolean;
  error: unknown;
  onOpenChange: (open: boolean) => void;
}) {
  const errorMessage = rtkErrorMessage(error, "Could not load schedule history");

  return (
    <Sheet open={schedule !== null} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full overflow-y-auto data-[side=right]:w-full data-[side=right]:sm:max-w-2xl">
        <SheetHeader className="pr-12">
          <SheetTitle className="break-all font-mono">{schedule?.schedule_id ?? "Schedule"}</SheetTitle>
          <SheetDescription>
            Scheduled runs of this schedule only. Manual SQL and HTTP calls of the same procedure are not shown.
          </SheetDescription>
        </SheetHeader>
        {schedule ? (
          <div className="flex flex-col gap-4 px-6 pb-6">
            <dl className="grid gap-3 sm:grid-cols-2">
              <div className="grid gap-1">
                <dt className="text-xs text-muted-foreground">Procedure</dt>
                <dd>
                  <Link className="text-sm underline" to={`/functions/${encodeURIComponent(schedule.routine_id)}/logs`}>
                    {schedule.routine_id}
                  </Link>
                </dd>
              </div>
              <div className="grid gap-1">
                <dt className="text-xs text-muted-foreground">Status</dt>
                <dd><Badge variant="outline">{scheduleStatus(schedule)}</Badge></dd>
              </div>
              <div className="grid gap-1">
                <dt className="text-xs text-muted-foreground">Next run</dt>
                <dd className="text-sm">{schedule.enabled ? formatScheduleTime(schedule.next_run_at) : "—"}</dd>
              </div>
              <div className="grid gap-1">
                <dt className="text-xs text-muted-foreground">Runs / skipped</dt>
                <dd className="text-sm">{schedule.run_count} / {schedule.skip_count}</dd>
              </div>
            </dl>
            {schedule.last_error ? (
              <p className="break-words text-sm text-destructive">{schedule.last_error}</p>
            ) : null}
            {errorMessage ? <p role="alert" className="text-sm text-destructive">{errorMessage}</p> : null}
            <div className="overflow-x-auto rounded-md border" data-testid="schedule-history-table">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Time</TableHead>
                    <TableHead>Outcome</TableHead>
                    <TableHead>Duration</TableHead>
                    <TableHead>Execution</TableHead>
                    <TableHead>Message</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {isLoading && logs.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5} className="py-10 text-center text-muted-foreground">
                        Loading history…
                      </TableCell>
                    </TableRow>
                  ) : logs.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5} className="py-10 text-center text-muted-foreground">
                        No scheduled invocations logged yet for this schedule.
                      </TableCell>
                    </TableRow>
                  ) : logs.map((log) => (
                    <TableRow key={`${log.timestamp}-${log.execution_id}-${log.outcome}`}>
                      <TableCell className="whitespace-nowrap">{formatScheduleTime(log.timestamp)}</TableCell>
                      <TableCell>
                        <Badge variant={log.outcome.trim().toLowerCase() === "error" ? "destructive" : "outline"}>
                          {outcomeLabel(log.outcome)}
                        </Badge>
                      </TableCell>
                      <TableCell className="whitespace-nowrap">{formatDurationMs(Number(log.duration_ms))}</TableCell>
                      <TableCell className="max-w-[10rem] truncate font-mono text-xs">
                        {log.execution_id || "—"}
                      </TableCell>
                      <TableCell className="max-w-sm truncate">{log.message || "—"}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          </div>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}
