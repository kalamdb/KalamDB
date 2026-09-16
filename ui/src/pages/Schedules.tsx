import { useState } from "react";
import { Link } from "react-router-dom";
import { RefreshCw } from "lucide-react";
import { PageLayout } from "@/components/layout/PageLayout";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { apiSlice } from "@/store/apiSlice";
import { dropSchedule, fetchSchedules, scheduleStatus, setScheduleEnabled, type ScheduleFilter, type ScheduleRow } from "@/services/schedulesService";

const schedulesApi = apiSlice.injectEndpoints({ endpoints: (builder) => ({
  getSchedules: builder.query<Awaited<ReturnType<typeof fetchSchedules>>, { page: number; filter: ScheduleFilter }>({
    async queryFn({ page, filter }) {
      try { return { data: await fetchSchedules(page, filter) }; }
      catch (error) { return { error: { status: "CUSTOM_ERROR", error: error instanceof Error ? error.message : "Could not load schedules" } }; }
    },
  }),
}) });
const formatTime = (value: number | null) => value == null ? "—" : new Date(Number(value)).toLocaleString();

export default function Schedules() {
  const [page, setPage] = useState(0);
  const [filter, setFilter] = useState<ScheduleFilter>("all");
  const [busy, setBusy] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<ScheduleRow | null>(null);
  const { data, isFetching, error, refetch } = schedulesApi.useGetSchedulesQuery({ page, filter }, { pollingInterval: 5000, skipPollingIfUnfocused: true });
  const errorMessage = actionError ?? (error && "error" in error ? error.error : null);
  async function change(row: ScheduleRow, remove = false) {
    setBusy(row.schedule_id); setActionError(null);
    try {
      if (remove) await dropSchedule(row); else await setScheduleEnabled(row, !row.enabled);
      await refetch();
    } catch (error) { setActionError(error instanceof Error ? error.message : "Schedule update failed"); }
    finally { setBusy(null); }
  }
  return <PageLayout title="Schedules" description="Monitor scheduled procedures across the cluster. Refreshes every five seconds." actions={
    <Button variant="outline" size="sm" onClick={() => void refetch()} disabled={isFetching}><RefreshCw className={isFetching ? "animate-spin" : undefined} />Refresh</Button>
  }>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <label className="flex items-center gap-2 text-sm" htmlFor="schedules-filter">Show
        <select id="schedules-filter" className="rounded-md border bg-background px-3 py-2" value={filter} onChange={(event) => { setFilter(event.target.value as ScheduleFilter); setPage(0); }}>
          <option value="all">All schedules</option><option value="enabled">Enabled</option><option value="running">Running</option><option value="disabled">Disabled</option>
        </select>
      </label>
      <Link className="text-sm underline" to="/sql">Create schedules in SQL Studio</Link>
    </div>
    {errorMessage && <p role="alert" className="text-sm text-destructive">{errorMessage}</p>}
    {!data && isFetching ? <p role="status">Loading schedules…</p> : <div className="overflow-x-auto rounded-md border">
      <Table><TableHeader><TableRow>
        <TableHead>Schedule / procedure</TableHead><TableHead>Status</TableHead><TableHead>Timing</TableHead><TableHead>Next run</TableHead><TableHead>Latest execution</TableHead><TableHead>Runs / skipped</TableHead><TableHead>Actions</TableHead>
      </TableRow></TableHeader><TableBody>
        {data?.rows.map((row) => <TableRow key={row.schedule_id} data-testid={`schedules-row-${row.schedule_id}`}>
          <TableCell><div className="font-medium">{row.schedule_id}</div><Link className="text-xs text-muted-foreground underline" to={`/functions/${encodeURIComponent(row.routine_id)}`}>{row.routine_id}</Link><div className="text-xs text-muted-foreground">Principal: {row.principal_user_id}</div></TableCell>
          <TableCell><Badge variant="outline">{scheduleStatus(row)}</Badge>{!row.enabled && row.running_until != null && <div className="text-xs">Future runs disabled</div>}{row.owner && <div className="text-xs">Node {row.owner}</div>}</TableCell>
          <TableCell><code>{row.cron ?? `Every ${Number(row.interval_ms) / 1000}s`}</code><div className="text-xs text-muted-foreground">{row.timezone}</div></TableCell>
          <TableCell className="whitespace-nowrap">{row.enabled ? formatTime(row.next_run_at) : "—"}</TableCell>
          <TableCell><div className="whitespace-nowrap text-xs">Started: {formatTime(row.last_started_at)}</div><div className="whitespace-nowrap text-xs">Finished: {formatTime(row.last_finished_at)}</div>{row.last_error && <p className="max-w-xs break-words text-xs text-destructive">{row.last_error}</p>}</TableCell>
          <TableCell data-testid={`schedules-runs-${row.schedule_id}`}>{row.run_count} / {row.skip_count}</TableCell>
          <TableCell><div className="flex gap-2"><Button size="sm" variant="outline" disabled={busy !== null} onClick={() => void change(row)} aria-label={`${row.enabled ? "Disable" : "Enable"} ${row.schedule_id}`}>{row.enabled ? "Disable" : "Enable"}</Button><Button size="sm" variant="ghost" disabled={busy !== null || row.running_until != null} onClick={() => setDeleting(row)} aria-label={`Delete ${row.schedule_id}`}>Delete</Button></div></TableCell>
        </TableRow>)}
        {data?.rows.length === 0 && <TableRow><TableCell colSpan={7} className="py-10 text-center text-muted-foreground">No schedules match this view. Use CREATE SCHEDULE in SQL Studio to add one.</TableCell></TableRow>}
      </TableBody></Table>
    </div>}
    <div className="flex items-center justify-between text-sm"><span>Page {page + 1} · Times shown in your local time zone</span><div className="flex gap-2"><Button size="sm" variant="outline" disabled={page === 0 || isFetching} onClick={() => setPage(page - 1)}>Previous</Button><Button size="sm" variant="outline" disabled={!data?.hasMore || isFetching} onClick={() => setPage(page + 1)}>Next</Button></div></div>
    <p className="text-xs text-muted-foreground">Overlapping and missed runs are skipped. Disabling a schedule allows its current procedure to finish. <Link to="/logging/jobs" className="underline">View maintenance jobs</Link></p>
    <Dialog open={deleting !== null} onOpenChange={(open) => { if (!open) setDeleting(null); }}><DialogContent><DialogHeader><DialogTitle>Delete {deleting?.schedule_id}?</DialogTitle><DialogDescription>This removes the schedule and its latest execution metadata. The procedure remains available.</DialogDescription></DialogHeader><DialogFooter><Button variant="outline" onClick={() => setDeleting(null)}>Cancel</Button><Button onClick={() => { if (deleting) void change(deleting, true); setDeleting(null); }}>Delete schedule</Button></DialogFooter></DialogContent></Dialog>
  </PageLayout>;
}
